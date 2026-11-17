//! Parametres utilisateur persistants.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MonitorConfig {
    /// Serveur sonde si aucune connexion de jeu n'est detectee.
    pub server_host: String,
    pub server_port: u16,
    /// Utiliser l'adresse reelle de la connexion de javaw.exe quand elle existe.
    pub auto_detect_server: bool,
    /// Hote Internet de reference (IPv4).
    pub internet_ip: String,
    /// Cadence des sondes ICMP (box, Internet).
    pub icmp_interval_ms: u64,
    /// Cadence de la sonde TCP vers le serveur (bornee a >= 3000 ms pour rester discret).
    pub tcp_interval_ms: u64,
}

impl Default for MonitorConfig {
    fn default() -> Self {
        MonitorConfig {
            server_host: "mc.hypixel.net".to_string(),
            server_port: 25565,
            auto_detect_server: true,
            internet_ip: "1.1.1.1".to_string(),
            icmp_interval_ms: 1000,
            tcp_interval_ms: 5000,
        }
    }
}

impl MonitorConfig {
    pub fn sanitized(&self) -> MonitorConfig {
        let mut c = self.clone();
        c.icmp_interval_ms = c.icmp_interval_ms.clamp(500, 5000);
        c.tcp_interval_ms = c.tcp_interval_ms.clamp(3000, 60_000);
        if c.server_port == 0 {
            c.server_port = 25565;
        }
        c
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub monitor: MonitorConfig,
    /// Touche de fonction du marqueur (1..=12).
    pub hotkey_fn: u32,
    pub hotkey_ctrl: bool,
    pub hotkey_alt: bool,
    pub hotkey_shift: bool,
    pub hotkey_enabled: bool,
    pub beep_on_marker: bool,
    /// Annuler automatiquement les optimisations a la fermeture.
    pub revert_on_exit: bool,
    /// Fenetre des statistiques en direct (secondes).
    pub stats_window_s: u32,
    /// Fenetre du graphique (secondes).
    pub chart_window_s: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            monitor: MonitorConfig::default(),
            hotkey_fn: 8,
            hotkey_ctrl: false,
            hotkey_alt: false,
            hotkey_shift: false,
            hotkey_enabled: true,
            beep_on_marker: true,
            revert_on_exit: false,
            stats_window_s: 60,
            chart_window_s: 120,
        }
    }
}

impl Settings {
    /// Modificateurs Win32 correspondants.
    pub fn hotkey_modifiers(&self) -> u32 {
        let mut m = 0;
        if self.hotkey_alt {
            m |= crate::platform::MOD_ALT;
        }
        if self.hotkey_ctrl {
            m |= crate::platform::MOD_CONTROL;
        }
        if self.hotkey_shift {
            m |= crate::platform::MOD_SHIFT;
        }
        m
    }

    pub fn hotkey_label(&self) -> String {
        let mut s = String::new();
        if self.hotkey_ctrl {
            s.push_str("Ctrl+");
        }
        if self.hotkey_alt {
            s.push_str("Alt+");
        }
        if self.hotkey_shift {
            s.push_str("Maj+");
        }
        s.push_str(&format!("F{}", self.hotkey_fn.clamp(1, 12)));
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_clamps_probe_rates() {
        let mut c = MonitorConfig::default();
        c.icmp_interval_ms = 10;
        c.tcp_interval_ms = 100;
        c.server_port = 0;
        let s = c.sanitized();
        assert_eq!(s.icmp_interval_ms, 500);
        assert_eq!(s.tcp_interval_ms, 3000);
        assert_eq!(s.server_port, 25565);
    }

    #[test]
    fn partial_json_uses_defaults() {
        let s: Settings = serde_json::from_str("{\"hotkey_fn\": 9}").unwrap();
        assert_eq!(s.hotkey_fn, 9);
        assert_eq!(s.monitor.server_host, "mc.hypixel.net");
        assert!(s.beep_on_marker);
    }

    #[test]
    fn hotkey_label_and_modifiers() {
        let mut s = Settings::default();
        assert_eq!(s.hotkey_label(), "F8");
        assert_eq!(s.hotkey_modifiers(), 0);
        s.hotkey_ctrl = true;
        s.hotkey_alt = true;
        assert_eq!(s.hotkey_label(), "Ctrl+Alt+F8");
        assert_eq!(s.hotkey_modifiers(), 0x0003);
    }
}
