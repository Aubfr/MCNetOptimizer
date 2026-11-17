//! Structures de donnees partagees (mesures, sessions, environnement).

use serde::{Deserialize, Serialize};

/// Cible d'une mesure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Target {
    /// Box / routeur (premier saut) : isole le Wi-Fi et le reseau local.
    Gateway,
    /// Hote Internet de reference (ICMP) : isole la ligne / le FAI.
    Internet,
    /// Serveur de jeu, sonde TCP (poignee de main) a faible cadence.
    Server,
    /// RTT reel de la connexion TCP de javaw.exe (statistiques Windows, sans lire le contenu).
    Game,
}

impl Target {
    pub const ALL: [Target; 4] = [Target::Gateway, Target::Internet, Target::Server, Target::Game];

    pub fn label(self) -> &'static str {
        match self {
            Target::Gateway => "Box / passerelle",
            Target::Internet => "Internet",
            Target::Server => "Serveur (sonde TCP)",
            Target::Game => "Connexion de jeu",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            Target::Gateway => "Box",
            Target::Internet => "Internet",
            Target::Server => "Serveur",
            Target::Game => "Jeu",
        }
    }
}

/// Un echantillon de latence. `rtt_ms = None` signifie perte / timeout.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Sample {
    pub t_ms: u64,
    pub target: Target,
    pub rtt_ms: Option<f32>,
}

/// Marqueur "ca a lag" pose par l'utilisateur.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Marker {
    pub t_ms: u64,
    #[serde(default)]
    pub note: String,
}

/// Charge reseau locale (tous adaptateurs actifs), en kbit/s.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct LoadSample {
    pub t_ms: u64,
    pub down_kbps: f32,
    pub up_kbps: f32,
}

/// Compteurs TCP reels de la connexion de jeu (cumules depuis l'activation).
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct TcpInfoSample {
    pub t_ms: u64,
    pub pkts_retrans: u32,
    pub timeouts: u32,
    pub smoothed_rtt_ms: u32,
}

/// Etat du Wi-Fi a un instant donne.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WifiSample {
    pub t_ms: u64,
    pub quality: u8,
    pub rssi_dbm: Option<i32>,
    pub tx_mbps: f32,
    pub rx_mbps: f32,
    pub bssid: String,
    pub channel: Option<u32>,
}

/// Description de la liaison Wi-Fi courante.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WifiInfo {
    pub ssid: String,
    pub bssid: String,
    pub adapter: String,
    /// Qualite du signal 0..100 (API WLAN de Windows).
    pub quality: u32,
    pub rssi_dbm: Option<i32>,
    pub channel: Option<u32>,
    /// "2,4 GHz" ou "5/6 GHz" (deduit du canal).
    pub band: String,
    pub rx_mbps: f32,
    pub tx_mbps: f32,
    pub phy: String,
}

/// Groupe pour le protocole A/B.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionGroup {
    #[default]
    None,
    A,
    B,
}

impl SessionGroup {
    pub fn label(self) -> &'static str {
        match self {
            SessionGroup::None => "-",
            SessionGroup::A => "A",
            SessionGroup::B => "B",
        }
    }
}

/// Contexte technique d'une session.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EnvInfo {
    #[serde(default)]
    pub wifi: Option<WifiInfo>,
    #[serde(default)]
    pub gateway: Option<String>,
    #[serde(default)]
    pub server: Option<String>,
    #[serde(default)]
    pub admin: bool,
    #[serde(default)]
    pub active_tweaks: Vec<String>,
    #[serde(default)]
    pub game_detected: bool,
    #[serde(default)]
    pub estats_status: String,
    #[serde(default)]
    pub app_version: String,
}

/// Une session de mesure complete.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub group: SessionGroup,
    pub started_ms: u64,
    #[serde(default)]
    pub ended_ms: u64,
    #[serde(default)]
    pub env: EnvInfo,
    #[serde(default)]
    pub samples: Vec<Sample>,
    #[serde(default)]
    pub markers: Vec<Marker>,
    #[serde(default)]
    pub load: Vec<LoadSample>,
    #[serde(default)]
    pub tcp: Vec<TcpInfoSample>,
    #[serde(default)]
    pub wifi: Vec<WifiSample>,
}

impl Session {
    /// Serie temporelle triee d'une cible.
    pub fn series(&self, target: Target) -> Vec<(u64, Option<f32>)> {
        let mut v: Vec<(u64, Option<f32>)> = self
            .samples
            .iter()
            .filter(|s| s.target == target)
            .map(|s| (s.t_ms, s.rtt_ms))
            .collect();
        v.sort_by_key(|x| x.0);
        v
    }

    /// Duree en secondes (jusqu'a la fin, ou jusqu'au dernier echantillon).
    pub fn duration_s(&self) -> f64 {
        let end = if self.ended_ms > 0 {
            self.ended_ms
        } else {
            self.samples.iter().map(|s| s.t_ms).max().unwrap_or(self.started_ms)
        };
        end.saturating_sub(self.started_ms) as f64 / 1000.0
    }

    pub fn meta(&self) -> SessionMeta {
        SessionMeta {
            id: self.id.clone(),
            label: self.label.clone(),
            group: self.group,
            started_ms: self.started_ms,
            duration_s: self.duration_s(),
            sample_count: self.samples.len(),
            marker_count: self.markers.len(),
            tweaks: self.env.active_tweaks.clone(),
        }
    }
}

/// Resume leger d'une session (fichier .meta.json).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub group: SessionGroup,
    pub started_ms: u64,
    #[serde(default)]
    pub duration_s: f64,
    #[serde(default)]
    pub sample_count: usize,
    #[serde(default)]
    pub marker_count: usize,
    #[serde(default)]
    pub tweaks: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn series_is_sorted_and_filtered() {
        let mut s = Session::default();
        s.samples.push(Sample { t_ms: 3000, target: Target::Gateway, rtt_ms: Some(3.0) });
        s.samples.push(Sample { t_ms: 1000, target: Target::Gateway, rtt_ms: Some(1.0) });
        s.samples.push(Sample { t_ms: 2000, target: Target::Internet, rtt_ms: Some(9.0) });
        let g = s.series(Target::Gateway);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].0, 1000);
        assert_eq!(g[1].0, 3000);
    }

    #[test]
    fn duration_uses_ended_or_last_sample() {
        let mut s = Session::default();
        s.started_ms = 1000;
        s.samples.push(Sample { t_ms: 11_000, target: Target::Gateway, rtt_ms: None });
        assert!((s.duration_s() - 10.0).abs() < 1e-9);
        s.ended_ms = 31_000;
        assert!((s.duration_s() - 30.0).abs() < 1e-9);
    }

    #[test]
    fn session_json_roundtrip() {
        let mut s = Session::default();
        s.id = "x".into();
        s.group = SessionGroup::B;
        s.samples.push(Sample { t_ms: 1, target: Target::Server, rtt_ms: Some(42.5) });
        let j = serde_json::to_string(&s).unwrap();
        let back: Session = serde_json::from_str(&j).unwrap();
        assert_eq!(back.group, SessionGroup::B);
        assert_eq!(back.samples.len(), 1);
        assert_eq!(back.samples[0].target, Target::Server);
    }
}
