//! Surveillance en temps reel : sondes (box, Internet, serveur), detection de javaw.exe,
//! RTT/retransmissions reels de la connexion de jeu, charge locale, Wi-Fi, marqueurs.
//!
//! Chaque source tourne dans son propre thread a basse priorite ; l'interface lit un
//! instantane via `Monitor::view`.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::analysis::stats::{compute_stats, to_f64, Stats};
use crate::config::MonitorConfig;
use crate::model::{
    LoadSample, Marker, Sample, Session, SessionGroup, Target, TcpInfoSample, WifiInfo, WifiSample,
};
use crate::platform::{self, TcpConn};
use crate::probe;
use crate::util::{now_ms, session_id};

/// Noms d'executables consideres comme le jeu (Lunar, Badlion et lanceur officiel utilisent javaw).
pub const GAME_PROCESSES: [&str; 2] = ["javaw.exe", "java.exe"];

#[derive(Clone, Debug, Default)]
pub struct GameStatus {
    pub java_pids: Vec<u32>,
    pub connection: Option<String>,
    pub server_hint: Option<SocketAddr>,
    pub estats: String,
    pub last_rtt: Option<f32>,
}

pub struct LiveData {
    pub session: Session,
    pub game: GameStatus,
    pub wifi: Option<WifiInfo>,
    pub gateway_ip: Option<Ipv4Addr>,
    pub server_target: Option<String>,
    pub load_now: Option<(f32, f32)>,
    pub notes: Vec<String>,
}

fn lock(m: &Arc<Mutex<LiveData>>) -> MutexGuard<'_, LiveData> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn sleep_until_or_stop(stop: &AtomicBool, deadline: Instant) {
    while !stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        std::thread::sleep((deadline - now).min(Duration::from_millis(50)));
    }
}

fn advance(next: &mut Instant, interval: Duration) {
    *next += interval;
    let now = Instant::now();
    if *next < now {
        *next = now;
    }
}

/// Choisit la connexion de jeu parmi la table TCP : etablie, appartenant a un processus Java,
/// hors loopback ; priorite au port 25565, sinon tout port hors 53/80/443.
pub fn pick_game_connection(table: &[TcpConn], pids: &[u32]) -> Option<TcpConn> {
    let cands: Vec<&TcpConn> = table
        .iter()
        .filter(|c| c.is_established() && pids.contains(&c.pid) && !c.remote.ip().is_loopback())
        .collect();
    if let Some(c) = cands.iter().find(|c| c.remote.port() == 25565) {
        return Some((*c).clone());
    }
    cands
        .iter()
        .find(|c| !matches!(c.remote.port(), 53 | 80 | 443))
        .map(|c| (*c).clone())
}

// ---------------------------------------------------------------------------
// Threads de mesure
// ---------------------------------------------------------------------------

fn icmp_thread(shared: Arc<Mutex<LiveData>>, stop: Arc<AtomicBool>, target: Target, ip: Ipv4Addr, interval: Duration) {
    let pinger = match platform::IcmpPinger::new() {
        Some(p) => p,
        None => {
            lock(&shared).notes.push("Impossible de créer le handle ICMP.".to_string());
            return;
        }
    };
    let mut next = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        let rtt = pinger.ping(ip, 1000);
        let t = now_ms();
        lock(&shared).session.samples.push(Sample { t_ms: t, target, rtt_ms: rtt });
        advance(&mut next, interval);
        sleep_until_or_stop(&stop, next);
    }
}

fn server_thread(shared: Arc<Mutex<LiveData>>, stop: Arc<AtomicBool>, cfg: MonitorConfig) {
    let interval = Duration::from_millis(cfg.tcp_interval_ms);
    let mut resolved: Option<(SocketAddr, Instant)> = None;
    let mut next = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        let hint = if cfg.auto_detect_server { lock(&shared).game.server_hint } else { None };
        let addr = match hint {
            Some(a) => Some(a),
            None => {
                let stale = resolved.map(|(_, t)| t.elapsed() > Duration::from_secs(120)).unwrap_or(true);
                if stale {
                    resolved = probe::resolve_ipv4(&cfg.server_host, cfg.server_port).map(|a| (a, Instant::now()));
                }
                resolved.map(|(a, _)| a)
            }
        };
        match addr {
            Some(a) => {
                lock(&shared).server_target = Some(a.to_string());
                let rtt = probe::tcp_connect_rtt(a, Duration::from_millis(1500));
                let t = now_ms();
                lock(&shared).session.samples.push(Sample { t_ms: t, target: Target::Server, rtt_ms: rtt });
            }
            None => {
                lock(&shared).server_target = None;
            }
        }
        advance(&mut next, interval);
        sleep_until_or_stop(&stop, next);
    }
}

fn game_thread(shared: Arc<Mutex<LiveData>>, stop: Arc<AtomicBool>) {
    let admin = platform::is_admin();
    let mut tick: u64 = 0;
    let mut current: Option<TcpConn> = None;
    let mut enabled = false;
    // Cumul des compteurs a travers les reconnexions.
    let (mut carry_retrans, mut carry_timeouts) = (0u32, 0u32);
    let (mut last_retrans, mut last_timeouts) = (0u32, 0u32);
    let mut next = Instant::now();

    while !stop.load(Ordering::Relaxed) {
        if tick % 2 == 0 || current.is_none() {
            let pids = platform::find_pids(&GAME_PROCESSES);
            let mut conn: Option<TcpConn> = None;
            if !pids.is_empty() {
                if let Ok(table) = platform::tcp_table() {
                    conn = pick_game_connection(&table, &pids);
                }
            }
            let key = |c: &Option<TcpConn>| c.as_ref().map(|x| (x.local, x.remote));
            if key(&conn) != key(&current) {
                // Nouvelle connexion (ou perdue) : on reporte les compteurs de l'ancienne.
                carry_retrans = carry_retrans.saturating_add(last_retrans);
                carry_timeouts = carry_timeouts.saturating_add(last_timeouts);
                last_retrans = 0;
                last_timeouts = 0;
                enabled = false;
            }
            {
                let mut d = lock(&shared);
                d.game.java_pids = pids.clone();
                d.game.connection = conn.as_ref().map(|c| format!("{} → {}", c.local, c.remote));
                d.game.server_hint = conn.as_ref().map(|c| SocketAddr::V4(SocketAddrV4::new(*c.remote.ip(), c.remote.port())));
            }
            current = conn;
        }

        match &current {
            Some(c) => {
                if !enabled {
                    if admin {
                        match platform::estats_enable(c) {
                            Ok(()) => enabled = true,
                            Err(code) => {
                                lock(&shared).game.estats = format!("activation impossible (code Windows {})", code);
                            }
                        }
                    } else {
                        lock(&shared).game.estats =
                            "droits administrateur requis pour lire le RTT réel de la connexion".to_string();
                    }
                }
                if enabled {
                    match platform::estats_read(c) {
                        Ok(st) => {
                            last_retrans = st.pkts_retrans;
                            last_timeouts = st.timeouts;
                            let rtt = if st.sample_rtt_ms > 0 { st.sample_rtt_ms } else { st.smoothed_rtt_ms };
                            let t = now_ms();
                            let mut d = lock(&shared);
                            d.game.estats = "actif (RTT et retransmissions réels)".to_string();
                            d.game.last_rtt = Some(rtt as f32);
                            d.session.samples.push(Sample { t_ms: t, target: Target::Game, rtt_ms: Some(rtt as f32) });
                            d.session.tcp.push(TcpInfoSample {
                                t_ms: t,
                                pkts_retrans: carry_retrans.saturating_add(st.pkts_retrans),
                                timeouts: carry_timeouts.saturating_add(st.timeouts),
                                smoothed_rtt_ms: st.smoothed_rtt_ms,
                            });
                        }
                        Err(code) => {
                            let msg = if code == 0 {
                                "valeurs incohérentes (structure Windows différente) : mesure ignorée".to_string()
                            } else {
                                format!("lecture impossible (code Windows {})", code)
                            };
                            lock(&shared).game.estats = msg;
                        }
                    }
                }
            }
            None => {
                let mut d = lock(&shared);
                d.game.last_rtt = None;
                d.game.estats = if d.game.java_pids.is_empty() {
                    "Minecraft (javaw.exe) non détecté".to_string()
                } else {
                    "javaw.exe détecté, aucune connexion serveur établie".to_string()
                };
            }
        }

        tick += 1;
        advance(&mut next, Duration::from_millis(1000));
        sleep_until_or_stop(&stop, next);
    }
}

fn env_thread(shared: Arc<Mutex<LiveData>>, stop: Arc<AtomicBool>) {
    let mut prev: HashMap<u32, (u32, u32)> = HashMap::new();
    let mut last = Instant::now();
    let mut first = true;
    let mut tick = 0u64;
    let mut next = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        // --- charge reseau locale ---
        let table = platform::if_octets();
        let dt = last.elapsed().as_secs_f64().max(0.001);
        last = Instant::now();
        let (mut din, mut dout) = (0u64, 0u64);
        for (idx, i, o) in table {
            if let Some((pi, po)) = prev.get(&idx) {
                din += i.wrapping_sub(*pi) as u64;
                dout += o.wrapping_sub(*po) as u64;
            }
            prev.insert(idx, (i, o));
        }
        if !first {
            let down = (din as f64 * 8.0 / 1000.0 / dt) as f32;
            let up = (dout as f64 * 8.0 / 1000.0 / dt) as f32;
            let mut d = lock(&shared);
            d.load_now = Some((down, up));
            d.session.load.push(LoadSample { t_ms: now_ms(), down_kbps: down, up_kbps: up });
        }
        first = false;

        // --- Wi-Fi (toutes les 2 s) ---
        if tick % 2 == 0 {
            let w = platform::wlan_current();
            let mut d = lock(&shared);
            if let Some(info) = &w {
                d.session.wifi.push(WifiSample {
                    t_ms: now_ms(),
                    quality: info.quality.min(100) as u8,
                    rssi_dbm: info.rssi_dbm,
                    tx_mbps: info.tx_mbps,
                    rx_mbps: info.rx_mbps,
                    bssid: info.bssid.clone(),
                    channel: info.channel,
                });
            }
            d.wifi = w;
        }
        tick += 1;
        advance(&mut next, Duration::from_millis(1000));
        sleep_until_or_stop(&stop, next);
    }
}

// ---------------------------------------------------------------------------
// API publique
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct TargetView {
    pub target: Target,
    pub last: Option<Option<f32>>,
    pub stats: Stats,
    pub points: Vec<(u64, Option<f32>)>,
}

#[derive(Clone, Debug)]
pub struct LiveView {
    pub now_ms: u64,
    pub elapsed_s: f64,
    pub targets: Vec<TargetView>,
    pub game: GameStatus,
    pub wifi: Option<WifiInfo>,
    pub gateway_ip: Option<Ipv4Addr>,
    pub server_target: Option<String>,
    pub load_now: Option<(f32, f32)>,
    pub markers: Vec<Marker>,
    pub notes: Vec<String>,
}

pub struct Monitor {
    shared: Arc<Mutex<LiveData>>,
    stop: Arc<AtomicBool>,
    handles: Vec<JoinHandle<()>>,
}

impl Monitor {
    pub fn start(cfg: &MonitorConfig, label: String, group: SessionGroup, active_tweaks: Vec<String>) -> Monitor {
        let cfg = cfg.sanitized();
        let started = now_ms();
        let mut session = Session::default();
        session.id = session_id(started);
        session.label = label;
        session.group = group;
        session.started_ms = started;
        session.env.admin = platform::is_admin();
        session.env.active_tweaks = active_tweaks;
        session.env.app_version = env!("CARGO_PKG_VERSION").to_string();

        let gateway = platform::default_gateway();
        let data = LiveData {
            session,
            game: GameStatus { estats: "en attente".to_string(), ..Default::default() },
            wifi: None,
            gateway_ip: gateway,
            server_target: None,
            load_now: None,
            notes: Vec::new(),
        };
        let shared = Arc::new(Mutex::new(data));
        let stop = Arc::new(AtomicBool::new(false));
        let mut handles = Vec::new();
        let icmp_iv = Duration::from_millis(cfg.icmp_interval_ms);

        if let Some(gw) = gateway {
            let (s, st) = (shared.clone(), stop.clone());
            handles.push(std::thread::spawn(move || icmp_thread(s, st, Target::Gateway, gw, icmp_iv)));
        } else {
            lock(&shared).notes.push("Passerelle par défaut introuvable : la box n'est pas mesurée.".to_string());
        }

        let inet: Ipv4Addr = cfg.internet_ip.parse().unwrap_or(Ipv4Addr::new(1, 1, 1, 1));
        {
            let (s, st) = (shared.clone(), stop.clone());
            handles.push(std::thread::spawn(move || icmp_thread(s, st, Target::Internet, inet, icmp_iv)));
        }
        {
            let (s, st, c) = (shared.clone(), stop.clone(), cfg.clone());
            handles.push(std::thread::spawn(move || server_thread(s, st, c)));
        }
        {
            let (s, st) = (shared.clone(), stop.clone());
            handles.push(std::thread::spawn(move || game_thread(s, st)));
        }
        {
            let (s, st) = (shared.clone(), stop.clone());
            handles.push(std::thread::spawn(move || env_thread(s, st)));
        }

        Monitor { shared, stop, handles }
    }

    pub fn add_marker(&self, t_ms: u64, note: &str) {
        lock(&self.shared).session.markers.push(Marker { t_ms, note: note.to_string() });
    }

    pub fn set_active_tweaks(&self, ids: Vec<String>) {
        lock(&self.shared).session.env.active_tweaks = ids;
    }

    pub fn set_label(&self, label: &str, group: SessionGroup) {
        let mut d = lock(&self.shared);
        d.session.label = label.to_string();
        d.session.group = group;
    }

    pub fn marker_count(&self) -> usize {
        lock(&self.shared).session.markers.len()
    }

    pub fn session_id(&self) -> String {
        lock(&self.shared).session.id.clone()
    }

    /// Copie de la session en cours (environnement a jour), sans l'arreter.
    pub fn snapshot_session(&self) -> Session {
        let d = lock(&self.shared);
        let mut s = d.session.clone();
        s.env.wifi = d.wifi.clone();
        s.env.gateway = d.gateway_ip.map(|g| g.to_string());
        s.env.server = d.server_target.clone();
        s.env.game_detected = !d.game.java_pids.is_empty();
        s.env.estats_status = d.game.estats.clone();
        s.ended_ms = now_ms();
        s
    }

    /// Instantane pour l'interface (statistiques sur `stats_window_s`, serie sur `chart_window_s`).
    pub fn view(&self, stats_window_s: u64, chart_window_s: u64) -> LiveView {
        let d = lock(&self.shared);
        let now = now_ms();
        let since_chart = now.saturating_sub(chart_window_s * 1000);
        let since_stats = now.saturating_sub(stats_window_s * 1000);
        let mut targets = Vec::new();
        for t in Target::ALL {
            let mut pts: Vec<(u64, Option<f32>)> = Vec::new();
            for s in d.session.samples.iter().rev() {
                if s.t_ms + 5000 < since_chart {
                    break;
                }
                if s.target == t && s.t_ms >= since_chart {
                    pts.push((s.t_ms, s.rtt_ms));
                }
            }
            pts.reverse();
            let win: Vec<(u64, Option<f32>)> = pts.iter().filter(|p| p.0 >= since_stats).cloned().collect();
            let stats = compute_stats(&to_f64(&win));
            let last = pts.last().map(|p| p.1);
            targets.push(TargetView { target: t, last, stats, points: pts });
        }
        LiveView {
            now_ms: now,
            elapsed_s: now.saturating_sub(d.session.started_ms) as f64 / 1000.0,
            targets,
            game: d.game.clone(),
            wifi: d.wifi.clone(),
            gateway_ip: d.gateway_ip,
            server_target: d.server_target.clone(),
            load_now: d.load_now,
            markers: d.session.markers.clone(),
            notes: d.notes.clone(),
        }
    }

    /// Arrete toutes les sondes et renvoie la session finale.
    pub fn stop(mut self) -> Session {
        self.stop.store(true, Ordering::Relaxed);
        for h in self.handles.drain(..) {
            let _ = h.join();
        }
        self.snapshot_session()
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn(pid: u32, remote_port: u16, remote: [u8; 4], state: u32) -> TcpConn {
        TcpConn {
            state,
            local: SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 20), 50000),
            remote: SocketAddrV4::new(Ipv4Addr::from(remote), remote_port),
            pid,
            row: [0; 5],
        }
    }

    #[test]
    fn prefers_minecraft_port() {
        let t = vec![
            conn(100, 443, [34, 1, 1, 1], 5),
            conn(100, 25565, [172, 65, 1, 1], 5),
        ];
        let c = pick_game_connection(&t, &[100]).unwrap();
        assert_eq!(c.remote.port(), 25565);
    }

    #[test]
    fn ignores_other_pids_loopback_and_unestablished() {
        let t = vec![
            conn(200, 25565, [1, 2, 3, 4], 5),
            conn(100, 25565, [127, 0, 0, 1], 5),
            conn(100, 25565, [5, 6, 7, 8], 2),
        ];
        assert!(pick_game_connection(&t, &[100]).is_none());
    }

    #[test]
    fn falls_back_to_custom_port_but_not_web_ports() {
        let t = vec![conn(100, 443, [34, 1, 1, 1], 5)];
        assert!(pick_game_connection(&t, &[100]).is_none());
        let t = vec![conn(100, 443, [34, 1, 1, 1], 5), conn(100, 30000, [9, 9, 9, 9], 5)];
        assert_eq!(pick_game_connection(&t, &[100]).unwrap().remote.port(), 30000);
    }

    #[test]
    fn monitor_runs_and_collects_samples() {
        // Test d'integration reel : demarre toutes les sondes ~4 s sur la machine de test.
        let cfg = MonitorConfig::default();
        let m = Monitor::start(&cfg, "test".to_string(), SessionGroup::None, vec![]);
        std::thread::sleep(Duration::from_millis(3500));
        m.add_marker(now_ms(), "test");
        let v = m.view(60, 120);
        assert_eq!(v.targets.len(), 4);
        assert_eq!(v.markers.len(), 1);
        let s = m.stop();
        assert!(!s.id.is_empty());
        assert!(s.ended_ms >= s.started_ms);
        // La sonde Internet (ICMP) a produit des echantillons (recus ou perdus).
        assert!(s.samples.iter().any(|x| x.target == Target::Internet));
        assert_eq!(s.markers.len(), 1);
    }
}
