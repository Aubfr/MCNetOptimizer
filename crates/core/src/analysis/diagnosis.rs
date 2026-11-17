//! Diagnostic par regles : relie les mesures (box / Internet / serveur / connexion reelle,
//! Wi-Fi, charge locale, marqueurs) a une cause probable, en indiquant ce qui est
//! corrigeable localement et ce qui ne l'est pas. Chaque conclusion cite ses chiffres.

use crate::model::{Session, Target};

use super::spikes::{detect_periodicity, detect_spikes, spike_threshold};
use super::stats::{compute_stats, loss_bursts, pearson, to_f64, Stats};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Ok,
    Info,
    Warning,
    Critical,
}

#[derive(Clone, Debug)]
pub struct Finding {
    pub severity: Severity,
    pub title: String,
    pub detail: String,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct MarkerTargetReport {
    pub target: Target,
    pub max_rtt: f64,
    pub lost: usize,
    pub anomalous: bool,
}

#[derive(Clone, Debug)]
pub struct MarkerReport {
    pub t_ms: u64,
    pub note: String,
    pub per_target: Vec<MarkerTargetReport>,
    pub any_anomaly: bool,
}

#[derive(Clone, Debug)]
pub struct Diagnosis {
    pub duration_s: f64,
    pub stats: Vec<(Target, Stats)>,
    pub findings: Vec<Finding>,
    pub marker_reports: Vec<MarkerReport>,
}

impl Diagnosis {
    pub fn stats_for(&self, t: Target) -> Option<&Stats> {
        self.stats.iter().find(|(x, _)| *x == t).map(|(_, s)| s)
    }

    pub fn worst(&self) -> Severity {
        self.findings.iter().map(|f| f.severity).max().unwrap_or(Severity::Ok)
    }
}

/// Fenetre d'analyse autour d'un marqueur : 8 s avant (delai de reaction), 1 s apres.
pub const MARKER_BEFORE_MS: u64 = 8_000;
pub const MARKER_AFTER_MS: u64 = 1_000;

fn f1(x: f64) -> String {
    format!("{:.1}", x)
}

fn push(
    v: &mut Vec<Finding>,
    severity: Severity,
    title: &str,
    detail: &str,
    evidence: Vec<String>,
) {
    v.push(Finding {
        severity,
        title: title.to_string(),
        detail: detail.to_string(),
        evidence,
    });
}

pub fn marker_reports(s: &Session) -> Vec<MarkerReport> {
    let series: Vec<(Target, Vec<(u64, Option<f32>)>, f64)> = Target::ALL
        .iter()
        .map(|t| {
            let ser = s.series(*t);
            let rtts: Vec<f64> = ser.iter().filter_map(|(_, r)| r.map(|v| v as f64)).collect();
            let thr = spike_threshold(&rtts);
            (*t, ser, thr)
        })
        .filter(|(_, ser, _)| !ser.is_empty())
        .collect();

    let mut out = Vec::new();
    for m in &s.markers {
        let lo = m.t_ms.saturating_sub(MARKER_BEFORE_MS);
        let hi = m.t_ms + MARKER_AFTER_MS;
        let mut per = Vec::new();
        for (t, ser, thr) in &series {
            let win: Vec<&(u64, Option<f32>)> =
                ser.iter().filter(|(ts, _)| *ts >= lo && *ts <= hi).collect();
            if win.is_empty() {
                continue;
            }
            let max_rtt = win
                .iter()
                .filter_map(|(_, r)| r.map(|v| v as f64))
                .fold(0.0_f64, f64::max);
            let lost = win.iter().filter(|(_, r)| r.is_none()).count();
            let anomalous = lost > 0 || max_rtt > *thr;
            per.push(MarkerTargetReport { target: *t, max_rtt, lost, anomalous });
        }
        let any_anomaly = per.iter().any(|p| p.anomalous);
        out.push(MarkerReport { t_ms: m.t_ms, note: m.note.clone(), per_target: per, any_anomaly });
    }
    out
}

/// Charge reseau moyenne par fenetre de `win_ms` alignee sur `t0`.
fn load_windows(s: &Session, t0: u64, win_ms: u64) -> Vec<(u64, f64)> {
    let mut acc: std::collections::BTreeMap<u64, (f64, usize)> = std::collections::BTreeMap::new();
    for l in &s.load {
        let k = l.t_ms.saturating_sub(t0) / win_ms;
        let e = acc.entry(k).or_insert((0.0, 0));
        e.0 += (l.down_kbps + l.up_kbps) as f64;
        e.1 += 1;
    }
    acc.into_iter().map(|(k, (sum, n))| (k, sum / n as f64)).collect()
}

fn rtt_windows(series: &[(u64, Option<f32>)], t0: u64, win_ms: u64) -> std::collections::BTreeMap<u64, f64> {
    let mut acc: std::collections::BTreeMap<u64, Vec<f64>> = std::collections::BTreeMap::new();
    for (t, r) in series {
        if let Some(v) = r {
            acc.entry(t.saturating_sub(t0) / win_ms).or_default().push(*v as f64);
        }
    }
    acc.into_iter()
        .map(|(k, v)| (k, super::stats::median(&v)))
        .collect()
}

pub fn diagnose(s: &Session) -> Diagnosis {
    let duration = s.duration_s();
    let mut stats: Vec<(Target, Stats)> = Vec::new();
    for t in Target::ALL {
        let ser = s.series(t);
        if !ser.is_empty() {
            stats.push((t, compute_stats(&to_f64(&ser))));
        }
    }
    let get = |t: Target| -> Option<Stats> {
        stats.iter().find(|(x, _)| *x == t).map(|(_, st)| st.clone()).filter(|st| st.total > 0)
    };

    let mut f: Vec<Finding> = Vec::new();

    let g = get(Target::Gateway);
    let i = get(Target::Internet);
    let sv = get(Target::Server);
    let game = get(Target::Game);

    // --- Donnees suffisantes ? ---
    let enough = duration >= 60.0 && g.as_ref().map(|x| x.total >= 30).unwrap_or(false);
    if !enough {
        push(
            &mut f,
            Severity::Info,
            "Données insuffisantes",
            "Une session d'au moins 5 à 10 minutes (idéalement pendant des combats) est nécessaire pour un diagnostic fiable. Les conclusions ci-dessous sont indicatives.",
            vec![format!("Durée : {:.0} s", duration)],
        );
    }

    // --- Box / Wi-Fi / local ---
    let mut local_bad = false;
    if let Some(g) = &g {
        if g.has_data() {
            let critical = g.loss_pct >= 0.5 || g.p95 >= 20.0 || g.jitter >= 5.0;
            let warning = g.loss_pct >= 0.2 || g.p95 >= 10.0 || g.jitter >= 3.0;
            let ev = vec![
                format!("Box : p50 {} ms, p95 {} ms, p99 {} ms", f1(g.p50), f1(g.p95), f1(g.p99)),
                format!("Gigue {} ms, perte {:.2} %", f1(g.jitter), g.loss_pct),
            ];
            if critical || warning {
                local_bad = true;
                push(
                    &mut f,
                    if critical { Severity::Critical } else { Severity::Warning },
                    "Instabilité entre le PC et la box (Wi-Fi / réseau local)",
                    "La latence vers la box devrait être de 1 à 5 ms et très stable. Un écart ici est introduit AVANT Internet : c'est le cas où un réglage local peut réellement aider (Wi-Fi 5 GHz, rapprochement de la box, câble Ethernet, économie d'énergie de la carte Wi-Fi).",
                    ev,
                );
            } else {
                push(
                    &mut f,
                    Severity::Ok,
                    "Liaison PC ↔ box saine",
                    "Aucune instabilité significative entre le PC et la box.",
                    ev,
                );
            }
        }
    } else {
        push(
            &mut f,
            Severity::Info,
            "Box non mesurée",
            "La passerelle par défaut n'a pas pu être détectée : la séparation « local / Internet » est impossible.",
            vec![],
        );
    }

    // --- Ligne / FAI ---
    if let Some(i) = &i {
        if i.has_data() {
            let spread = i.p95 - i.p50;
            let bad = i.loss_pct >= 0.5 || i.jitter >= 5.0 || spread >= 25.0;
            let ev = vec![
                format!("Internet : p50 {} ms, p95 {} ms, p99 {} ms", f1(i.p50), f1(i.p95), f1(i.p99)),
                format!("Gigue {} ms, perte {:.2} %", f1(i.jitter), i.loss_pct),
            ];
            if bad && !local_bad {
                push(
                    &mut f,
                    Severity::Warning,
                    "Instabilité côté ligne / FAI (au-delà de la box)",
                    "La box répond bien mais Internet est irrégulier. Causes typiques : saturation de la ligne par un autre appareil ou programme (bufferbloat), ligne dégradée, congestion du FAI. Un SQM (fq_codel/CAKE) sur le routeur est la vraie solution au bufferbloat ; l'outil ne peut pas le remplacer.",
                    ev,
                );
            } else if bad {
                push(
                    &mut f,
                    Severity::Info,
                    "Instabilité Internet probablement héritée du problème local",
                    "La box est déjà instable : corrigez d'abord le local avant d'interpréter le reste.",
                    ev,
                );
            }
        }
    }

    // --- Serveur ---
    if let Some(sv) = &sv {
        if sv.has_data() {
            let baseline_jitter = i.as_ref().filter(|x| x.has_data()).map(|x| x.jitter).unwrap_or(0.0);
            let extra = sv.jitter - baseline_jitter;
            let ev = vec![
                format!("Serveur : p50 {} ms, p95 {} ms, p99 {} ms", f1(sv.p50), f1(sv.p95), f1(sv.p99)),
                format!("Gigue {} ms, perte {:.2} %", f1(sv.jitter), sv.loss_pct),
            ];
            if extra >= 5.0 && !local_bad {
                push(
                    &mut f,
                    Severity::Warning,
                    "Irrégularité propre au chemin vers le serveur (route / serveur)",
                    "Les mesures vers la box et Internet sont meilleures que vers le serveur. Cela dépend de la route de votre FAI vers le serveur : aucun réglage local ne peut le corriger. Un VPN gaming ou un autre FAI pourraient changer la route, sans garantie.",
                    ev,
                );
            }
            if sv.p50 >= 80.0 {
                push(
                    &mut f,
                    Severity::Info,
                    "Latence de base élevée vers le serveur",
                    "Une latence constante élevée est liée à la distance : elle n'est pas de la gigue et ne se corrige pas localement.",
                    vec![format!("p50 {} ms", f1(sv.p50))],
                );
            }
        }
    }

    // --- Connexion reelle (EStats) ---
    let retrans_total = s.tcp.last().map(|x| x.pkts_retrans).unwrap_or(0)
        .saturating_sub(s.tcp.first().map(|x| x.pkts_retrans).unwrap_or(0));
    let timeouts_total = s.tcp.last().map(|x| x.timeouts).unwrap_or(0)
        .saturating_sub(s.tcp.first().map(|x| x.timeouts).unwrap_or(0));
    if !s.tcp.is_empty() {
        let minutes = (duration / 60.0).max(1.0 / 60.0);
        let rate = retrans_total as f64 / minutes;
        let ev = vec![
            format!("Retransmissions TCP : {} ({} par minute)", retrans_total, f1(rate)),
            format!("Délais de retransmission (RTO) : {}", timeouts_total),
        ];
        if rate >= 5.0 || timeouts_total >= 3 {
            push(
                &mut f,
                Severity::Critical,
                "Retransmissions TCP fréquentes sur la connexion de jeu",
                "Minecraft utilise TCP : un paquet perdu bloque tous les suivants jusqu'à sa retransmission (head-of-line blocking), ce qui produit des gels de 100 à 300 ms bien plus perceptibles qu'un ping élevé. Les pertes viennent du Wi-Fi, de la ligne ou de la route.",
                ev,
            );
        } else if rate >= 1.0 || timeouts_total >= 1 {
            push(
                &mut f,
                Severity::Warning,
                "Quelques retransmissions TCP sur la connexion de jeu",
                "Des paquets de la vraie connexion ont dû être renvoyés : chaque événement peut avoir causé un micro-gel.",
                ev,
            );
        } else {
            push(
                &mut f,
                Severity::Ok,
                "Connexion de jeu sans retransmission notable",
                "Aucune perte significative n'a été observée sur la connexion TCP réelle.",
                ev,
            );
        }
    } else if let Some(game) = &game {
        let _ = game;
    } else {
        push(
            &mut f,
            Severity::Info,
            "Pas de mesure de la connexion de jeu réelle",
            "Aucune connexion de javaw.exe n'a été mesurée (jeu non lancé ou droits administrateur absents). Relancez l'outil en administrateur pour lire le RTT et les retransmissions réels de la connexion Minecraft.",
            vec![],
        );
    }

    // --- Pertes en rafale ---
    for t in [Target::Gateway, Target::Internet, Target::Server] {
        let ser = s.series(t);
        let li = loss_bursts(&to_f64(&ser));
        if li.bursts >= 1 {
            push(
                &mut f,
                Severity::Warning,
                &format!("Pertes en rafale vers : {}", t.label()),
                "Des pertes consécutives (≥ 2 paquets) sont typiques d'un Wi-Fi qui décroche, d'un scan de réseaux ou d'un lien saturé.",
                vec![format!(
                    "{} rafale(s), plus longue : {} paquets ; {} perte(s) isolée(s)",
                    li.bursts, li.longest, li.isolated
                )],
            );
        }
    }

    // --- Periodicite ---
    for t in [Target::Gateway, Target::Internet] {
        let sp = detect_spikes(&s.series(t));
        if let Some(p) = detect_periodicity(&sp) {
            push(
                &mut f,
                Severity::Warning,
                &format!("Pics périodiques vers : {}", t.label()),
                "Des pics à intervalle régulier trahissent une tâche récurrente : scan Wi-Fi en arrière-plan de Windows, application qui synchronise, tâche planifiée. Fermez les programmes de synchronisation pendant le jeu, et rapprochez-vous de la box pour réduire les scans.",
                vec![format!("{} pics, période d'environ {} s", p.spike_count, f1(p.period_s))],
            );
        }
    }

    // --- Wi-Fi ---
    if !s.wifi.is_empty() {
        let n = s.wifi.len() as f64;
        let avg_q = s.wifi.iter().map(|w| w.quality as f64).sum::<f64>() / n;
        let min_q = s.wifi.iter().map(|w| w.quality).min().unwrap_or(0);
        let avg_tx = s.wifi.iter().map(|w| w.tx_mbps as f64).sum::<f64>() / n;
        let mut roams = 0usize;
        for w in s.wifi.windows(2) {
            if !w[0].bssid.is_empty() && !w[1].bssid.is_empty() && w[0].bssid != w[1].bssid {
                roams += 1;
            }
        }
        let ev = vec![
            format!("Qualité du signal : moyenne {:.0} %, minimum {} %", avg_q, min_q),
            format!("Débit négocié moyen (émission) : {:.0} Mbit/s", avg_tx),
        ];
        if avg_q < 50.0 {
            push(
                &mut f,
                Severity::Critical,
                "Signal Wi-Fi faible",
                "Un signal faible provoque des retransmissions radio et de la gigue. Rapprochez le PC de la box, retirez les obstacles, ou passez en Ethernet / CPL.",
                ev.clone(),
            );
        } else if avg_q < 70.0 || min_q < 40 {
            push(
                &mut f,
                Severity::Warning,
                "Signal Wi-Fi moyen",
                "Le signal est utilisable mais pas optimal pour un jeu sensible à la gigue.",
                ev.clone(),
            );
        }
        if roams >= 2 {
            push(
                &mut f,
                Severity::Warning,
                "Changements de point d'accès (roaming) pendant la session",
                "Chaque changement de borne (box, répéteur, Wi-Fi mesh) provoque une micro-coupure. Verrouillez le PC sur une seule borne ou utilisez la bande 5 GHz de la borne la plus proche.",
                vec![format!("{} changement(s) de BSSID", roams)],
            );
        }
    }
    if let Some(w) = &s.env.wifi {
        if w.band.starts_with("2,4") {
            push(
                &mut f,
                Severity::Warning,
                "Connexion Wi-Fi en 2,4 GHz",
                "La bande 2,4 GHz est très encombrée (voisins, Bluetooth, micro-ondes). La bande 5 GHz est généralement plus stable pour le jeu si le signal est bon.",
                vec![format!("Canal {}", w.channel.map(|c| c.to_string()).unwrap_or_else(|| "?".to_string()))],
            );
        }
    } else if s.wifi.is_empty() {
        push(
            &mut f,
            Severity::Info,
            "Aucune liaison Wi-Fi détectée",
            "Le PC semble être en connexion filaire (ou le Wi-Fi n'a pas pu être lu).",
            vec![],
        );
    }

    // --- Bufferbloat (correlation charge / latence) ---
    if !s.load.is_empty() {
        if let Some(i) = &i {
            let t0 = s.started_ms;
            let win = 5_000u64;
            let rw = rtt_windows(&s.series(Target::Internet), t0, win);
            let lw = load_windows(s, t0, win);
            let mut xs = Vec::new();
            let mut ys = Vec::new();
            for (k, load) in lw {
                if let Some(r) = rw.get(&k) {
                    xs.push(load);
                    ys.push(*r);
                }
            }
            if xs.len() >= 12 {
                if let Some(r) = pearson(&xs, &ys) {
                    if r >= 0.5 && (i.p95 - i.p50) >= 10.0 {
                        push(
                            &mut f,
                            Severity::Warning,
                            "Latence corrélée à la charge réseau (bufferbloat probable)",
                            "Plus la ligne est chargée, plus la latence monte : le routeur met les paquets en file d'attente. Fermez les téléchargements / streams pendant le jeu, et activez un SQM (fq_codel / CAKE) sur le routeur si possible.",
                            vec![format!(
                                "Corrélation charge ↔ latence Internet : r = {:.2} sur {} fenêtres de 5 s",
                                r,
                                xs.len()
                            )],
                        );
                    }
                }
            }
        }
    }

    // --- Marqueurs ---
    let reports = marker_reports(s);
    if !reports.is_empty() {
        let total = reports.len();
        let anomalous = reports.iter().filter(|r| r.any_anomaly).count();
        let ev = vec![format!("{} marqueur(s) sur {} coïncident avec une anomalie réseau mesurée", anomalous, total)];
        let low_conf = if total < 3 { " (peu de marqueurs : conclusion peu fiable)" } else { "" };
        if anomalous == 0 {
            push(
                &mut f,
                Severity::Info,
                &format!("Vos ressentis de lag ne coïncident avec aucune anomalie réseau{}", low_conf),
                "Quand vous avez senti un problème, le réseau était stable. La cause est alors plutôt hors réseau : enregistrement de coups côté serveur (tick de 50 ms, compensation de lag), invulnérabilité 1.8, FPS irréguliers, charge CPU. Un outil réseau ne peut pas les corriger.",
                ev,
            );
        } else if anomalous * 2 >= total {
            push(
                &mut f,
                Severity::Warning,
                &format!("Vos ressentis de lag coïncident souvent avec des anomalies réseau{}", low_conf),
                "Le réseau explique au moins une partie de ce que vous ressentez : corriger les causes listées ci-dessus est pertinent.",
                ev,
            );
        } else {
            push(
                &mut f,
                Severity::Info,
                &format!("Corrélation partielle entre ressentis et anomalies réseau{}", low_conf),
                "Une partie seulement de vos ressentis coïncide avec le réseau ; le reste a vraisemblablement une autre cause.",
                ev,
            );
        }
    }

    if f.iter().all(|x| x.severity <= Severity::Info) {
        push(
            &mut f,
            Severity::Ok,
            "Aucune anomalie réseau significative détectée",
            "Sur cette session, le réseau est stable. Si vous ressentez encore des hits irréguliers, la cause est probablement hors réseau (voir les remarques ci-dessus). L'outil ne promet aucune amélioration du hit registration.",
            vec![],
        );
    }

    Diagnosis { duration_s: duration, stats, findings: f, marker_reports: reports }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Marker, Sample, TcpInfoSample, WifiSample};

    fn add(s: &mut Session, t: Target, n: usize, f: impl Fn(usize) -> Option<f32>) {
        for k in 0..n {
            s.samples.push(Sample { t_ms: s.started_ms + k as u64 * 1000, target: t, rtt_ms: f(k) });
        }
    }

    fn base_session(n: usize) -> Session {
        let mut s = Session::default();
        s.started_ms = 1_000_000;
        s.ended_ms = s.started_ms + n as u64 * 1000;
        s
    }

    fn has_title(d: &Diagnosis, needle: &str) -> bool {
        d.findings.iter().any(|f| f.title.contains(needle))
    }

    #[test]
    fn healthy_session_reports_ok() {
        let mut s = base_session(600);
        add(&mut s, Target::Gateway, 600, |_| Some(2.0));
        add(&mut s, Target::Internet, 600, |_| Some(12.0));
        add(&mut s, Target::Server, 600, |_| Some(30.0));
        let d = diagnose(&s);
        assert!(d.worst() <= Severity::Info, "findings: {:?}", d.findings);
        assert!(has_title(&d, "Aucune anomalie réseau significative"));
    }

    #[test]
    fn unstable_gateway_is_critical_local() {
        let mut s = base_session(600);
        add(&mut s, Target::Gateway, 600, |k| Some(if k % 4 == 0 { 60.0 } else { 3.0 }));
        add(&mut s, Target::Internet, 600, |_| Some(15.0));
        let d = diagnose(&s);
        assert!(has_title(&d, "entre le PC et la box"));
        assert_eq!(d.worst(), Severity::Critical);
    }

    #[test]
    fn server_only_instability_is_not_local() {
        let mut s = base_session(600);
        add(&mut s, Target::Gateway, 600, |_| Some(2.0));
        add(&mut s, Target::Internet, 600, |_| Some(12.0));
        add(&mut s, Target::Server, 600, |k| Some(if k % 2 == 0 { 30.0 } else { 70.0 }));
        let d = diagnose(&s);
        assert!(has_title(&d, "chemin vers le serveur"));
        assert!(!has_title(&d, "entre le PC et la box"));
    }

    #[test]
    fn heavy_retransmissions_flagged() {
        let mut s = base_session(600);
        add(&mut s, Target::Gateway, 600, |_| Some(2.0));
        s.tcp.push(TcpInfoSample { t_ms: s.started_ms, pkts_retrans: 0, timeouts: 0, smoothed_rtt_ms: 30 });
        s.tcp.push(TcpInfoSample { t_ms: s.ended_ms, pkts_retrans: 100, timeouts: 0, smoothed_rtt_ms: 30 });
        let d = diagnose(&s);
        assert!(has_title(&d, "Retransmissions TCP fréquentes"));
    }

    #[test]
    fn markers_without_anomaly_point_away_from_network() {
        let mut s = base_session(600);
        add(&mut s, Target::Gateway, 600, |_| Some(2.0));
        add(&mut s, Target::Internet, 600, |_| Some(12.0));
        for k in 1..=3u64 {
            s.markers.push(Marker { t_ms: s.started_ms + k * 100_000, note: String::new() });
        }
        let d = diagnose(&s);
        assert_eq!(d.marker_reports.len(), 3);
        assert!(d.marker_reports.iter().all(|m| !m.any_anomaly));
        assert!(has_title(&d, "aucune anomalie réseau"));
    }

    #[test]
    fn marker_on_a_spike_is_flagged() {
        let mut s = base_session(600);
        add(&mut s, Target::Gateway, 600, |k| Some(if (299..=301).contains(&k) { 150.0 } else { 2.0 }));
        s.markers.push(Marker { t_ms: s.started_ms + 302_000, note: String::new() });
        let reports = marker_reports(&s);
        assert_eq!(reports.len(), 1);
        assert!(reports[0].any_anomaly);
    }

    #[test]
    fn low_signal_and_roaming_flagged() {
        let mut s = base_session(600);
        add(&mut s, Target::Gateway, 600, |_| Some(2.0));
        for k in 0..10u64 {
            s.wifi.push(WifiSample {
                t_ms: s.started_ms + k * 2000,
                quality: 35,
                rssi_dbm: None,
                tx_mbps: 72.0,
                rx_mbps: 72.0,
                bssid: if k % 2 == 0 { "aa:aa".into() } else { "bb:bb".into() },
                channel: Some(6),
            });
        }
        let d = diagnose(&s);
        assert!(has_title(&d, "Signal Wi-Fi faible"));
        assert!(has_title(&d, "roaming"));
    }

    #[test]
    fn short_session_warns_about_insufficient_data() {
        let mut s = base_session(20);
        add(&mut s, Target::Gateway, 20, |_| Some(2.0));
        let d = diagnose(&s);
        assert!(has_title(&d, "Données insuffisantes"));
    }
}
