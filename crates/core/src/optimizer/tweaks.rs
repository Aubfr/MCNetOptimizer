//! Reglages reels et reversibles. Chacun est justifie, detecte avant d'etre propose,
//! verifie apres application et annulable. Aucun reglage TCP global, aucune ecriture
//! de registre generique, aucun service Windows touche.

use serde::{Deserialize, Serialize};

use super::{Applicability, Tweak, TweakInfo};
use crate::parse;
use crate::platform;

pub fn default_tweaks() -> Vec<Box<dyn Tweak>> {
    vec![
        Box::new(WifiPowerSavingPlan),
        Box::new(WifiAdapterPower),
        Box::new(JavaPriority),
    ]
}

/// Echappe une valeur pour un litteral PowerShell entre apostrophes.
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

// ---------------------------------------------------------------------------
// 1. Mode d'economie d'energie Wi-Fi du plan d'alimentation (secteur)
// ---------------------------------------------------------------------------

const WIFI_SUBGROUP: &str = "19cbb8fa-5279-450e-9fac-8a3d5fedd0c1";
const WIFI_POWER_SAVING: &str = "12bbebe6-58d6-4636-95bb-3217ef867c1a";

pub struct WifiPowerSavingPlan;

#[derive(Serialize, Deserialize)]
struct PlanSnapshot {
    scheme: String,
    ac: u32,
}

fn active_scheme() -> Result<String, String> {
    let out = platform::run_hidden("powercfg", &["/getactivescheme"])?;
    parse::extract_guid(&out.stdout).ok_or_else(|| "Plan d'alimentation introuvable.".to_string())
}

fn read_wifi_ac(scheme: &str) -> Result<u32, String> {
    let out = platform::run_hidden("powercfg", &["/query", scheme, WIFI_SUBGROUP, WIFI_POWER_SAVING])?;
    if out.code != 0 {
        return Err("Paramètre Wi-Fi absent de ce plan d'alimentation.".to_string());
    }
    parse::powercfg_ac_dc(&out.stdout)
        .map(|(ac, _)| ac)
        .ok_or_else(|| "Valeur illisible dans la sortie de powercfg.".to_string())
}

fn set_wifi_ac(scheme: &str, value: u32) -> Result<(), String> {
    let v = value.to_string();
    let o = platform::run_hidden(
        "powercfg",
        &["/setacvalueindex", scheme, WIFI_SUBGROUP, WIFI_POWER_SAVING, &v],
    )?;
    if o.code != 0 {
        return Err(format!("powercfg /setacvalueindex a échoué : {}", o.stderr.trim()));
    }
    let o = platform::run_hidden("powercfg", &["/setactive", scheme])?;
    if o.code != 0 {
        return Err(format!("powercfg /setactive a échoué : {}", o.stderr.trim()));
    }
    Ok(())
}

impl Tweak for WifiPowerSavingPlan {
    fn info(&self) -> TweakInfo {
        TweakInfo {
            id: "wifi_plan_power",
            title: "Wi-Fi : performances maximales (plan d'alimentation, secteur)",
            what: "Règle « Mode d'économie d'énergie » de la carte sans fil sur « Performances maximales » dans le plan d'alimentation actif, sur secteur uniquement.",
            why: "En mode économie, la carte Wi-Fi peut s'endormir entre deux paquets et les réveiller crée de la gigue. À valider par mesure A/B : l'effet dépend de la carte et du pilote.",
            risk: "Très faible : légère hausse de consommation sur secteur. Sans effet sur batterie. Annulable (valeur d'origine restaurée).",
            requires_admin: false,
        }
    }

    fn plan(&self) -> String {
        format!(
            "powercfg /setacvalueindex <plan actif> {} {} 0  puis  powercfg /setactive <plan actif>",
            WIFI_SUBGROUP, WIFI_POWER_SAVING
        )
    }

    fn detect(&self) -> Applicability {
        let scheme = match active_scheme() {
            Ok(s) => s,
            Err(e) => return Applicability::Unavailable { reason: e },
        };
        match read_wifi_ac(&scheme) {
            Ok(0) => Applicability::NotNeeded { reason: "déjà en performances maximales".to_string() },
            Ok(v) => Applicability::Applicable { current: format!("mode d'économie niveau {}", v) },
            Err(e) => Applicability::Unavailable { reason: e },
        }
    }

    fn snapshot(&self) -> Result<String, String> {
        let scheme = active_scheme()?;
        let ac = read_wifi_ac(&scheme)?;
        serde_json::to_string(&PlanSnapshot { scheme, ac }).map_err(|e| e.to_string())
    }

    fn apply(&self) -> Result<(), String> {
        let scheme = active_scheme()?;
        set_wifi_ac(&scheme, 0)
    }

    fn verify(&self) -> Result<(), String> {
        let scheme = active_scheme()?;
        match read_wifi_ac(&scheme)? {
            0 => Ok(()),
            v => Err(format!("valeur relue : {} (attendu 0)", v)),
        }
    }

    fn revert(&self, snapshot: &str) -> Result<(), String> {
        let s: PlanSnapshot = serde_json::from_str(snapshot).map_err(|e| e.to_string())?;
        set_wifi_ac(&s.scheme, s.ac)
    }
}

// ---------------------------------------------------------------------------
// 2. Gestion d'alimentation de l'adaptateur Wi-Fi (Windows peut l'eteindre)
// ---------------------------------------------------------------------------

pub struct WifiAdapterPower;

#[derive(Serialize, Deserialize)]
struct AdapterSnapshot {
    name: String,
    allow_off: String,
}

const FIND_ADAPTER: &str = "$a = Get-NetAdapter -Physical -ErrorAction SilentlyContinue | Where-Object { $_.PhysicalMediaType -like '*802.11*' -and $_.Status -eq 'Up' } | Select-Object -First 1; if (-not $a) { 'NOADAPTER'; exit 0 }; $p = Get-NetAdapterPowerManagement -Name $a.Name -ErrorAction SilentlyContinue; \"$($a.Name)|$($p.AllowComputerToTurnOffDevice)\"";

fn read_adapter() -> Result<(String, String), String> {
    let out = platform::powershell(FIND_ADAPTER)?;
    if out.trim() == "NOADAPTER" {
        return Err("Aucun adaptateur Wi-Fi actif détecté.".to_string());
    }
    let line = out.lines().last().unwrap_or("").trim().to_string();
    match line.split_once('|') {
        Some((n, v)) if !n.is_empty() => Ok((n.to_string(), v.trim().to_string())),
        _ => Err(format!("Sortie inattendue : {}", line)),
    }
}

fn set_adapter_allow_off(name: &str, value: &str) -> Result<(), String> {
    let script = format!(
        "Set-NetAdapterPowerManagement -Name {} -AllowComputerToTurnOffDevice {} -ErrorAction Stop",
        ps_quote(name),
        value
    );
    platform::powershell(&script).map(|_| ())
}

impl Tweak for WifiAdapterPower {
    fn info(&self) -> TweakInfo {
        TweakInfo {
            id: "wifi_adapter_power",
            title: "Wi-Fi : empêcher Windows d'éteindre la carte pour économiser l'énergie",
            what: "Désactive « Autoriser l'ordinateur à éteindre ce périphérique pour économiser l'énergie » sur la carte Wi-Fi active.",
            why: "Windows peut mettre la carte en veille et la réveiller, ce qui peut causer des pics de latence ou de brèves coupures. À valider par mesure A/B.",
            risk: "Faible : consommation légèrement plus élevée. Peut provoquer une brève coupure Wi-Fi (quelques secondes) lors de l'application et de l'annulation : à faire hors partie. Nécessite les droits administrateur.",
            requires_admin: true,
        }
    }

    fn plan(&self) -> String {
        "Set-NetAdapterPowerManagement -Name <carte Wi-Fi> -AllowComputerToTurnOffDevice Disabled".to_string()
    }

    fn detect(&self) -> Applicability {
        if !platform::is_admin() {
            return Applicability::Unavailable {
                reason: "droits administrateur requis (relancez l'outil en administrateur)".to_string(),
            };
        }
        match read_adapter() {
            Ok((name, v)) => match v.as_str() {
                "Enabled" => Applicability::Applicable { current: format!("{} : extinction autorisée", name) },
                "Disabled" => Applicability::NotNeeded { reason: format!("{} : déjà désactivé", name) },
                other => Applicability::Unavailable {
                    reason: format!("{} : gestion d'alimentation non prise en charge (« {} »)", name, other),
                },
            },
            Err(e) => Applicability::Unavailable { reason: e },
        }
    }

    fn snapshot(&self) -> Result<String, String> {
        let (name, v) = read_adapter()?;
        serde_json::to_string(&AdapterSnapshot { name, allow_off: v }).map_err(|e| e.to_string())
    }

    fn apply(&self) -> Result<(), String> {
        let (name, _) = read_adapter()?;
        set_adapter_allow_off(&name, "Disabled")
    }

    fn verify(&self) -> Result<(), String> {
        // La carte peut redemarrer : on laisse quelques secondes avant de relire.
        let mut last = String::new();
        for _ in 0..6 {
            std::thread::sleep(std::time::Duration::from_secs(2));
            match read_adapter() {
                Ok((_, v)) if v == "Disabled" => return Ok(()),
                Ok((_, v)) => last = v,
                Err(e) => last = e,
            }
        }
        Err(format!("valeur relue : {}", last))
    }

    fn revert(&self, snapshot: &str) -> Result<(), String> {
        let s: AdapterSnapshot = serde_json::from_str(snapshot).map_err(|e| e.to_string())?;
        if s.allow_off != "Enabled" && s.allow_off != "Disabled" {
            return Ok(());
        }
        set_adapter_allow_off(&s.name, &s.allow_off)
    }
}

// ---------------------------------------------------------------------------
// 3. Priorite du processus Minecraft (au-dessus de la normale, jamais "temps reel")
// ---------------------------------------------------------------------------

pub struct JavaPriority;

#[derive(Serialize, Deserialize)]
struct PrioritySnapshot {
    /// (pid, classe de priorite d'origine)
    procs: Vec<(u32, u32)>,
}

fn game_pids() -> Vec<u32> {
    platform::find_pids(&["javaw.exe", "java.exe"])
}

impl Tweak for JavaPriority {
    fn info(&self) -> TweakInfo {
        TweakInfo {
            id: "java_priority",
            title: "Minecraft : priorité « supérieure à la normale »",
            what: "Passe le processus javaw.exe en priorité « Supérieure à la normale » (jamais « Haute » ni « Temps réel »).",
            why: "Quand le CPU est chargé (navigateur, OBS, Discord), un thread de jeu retardé envoie ses actions en retard et le frame time devient irrégulier. Ce n'est pas du réseau, mais cela se confond avec du lag. À valider par mesure.",
            risk: "Très faible. Ne touche ni la mémoire ni les fichiers du jeu, aucun accès au contenu du processus. S'annule à la fermeture du jeu ou par restauration de la priorité d'origine.",
            requires_admin: false,
        }
    }

    fn plan(&self) -> String {
        "SetPriorityClass(javaw.exe, ABOVE_NORMAL_PRIORITY_CLASS) — droit PROCESS_SET_INFORMATION uniquement".to_string()
    }

    fn detect(&self) -> Applicability {
        let pids = game_pids();
        if pids.is_empty() {
            return Applicability::Unavailable { reason: "Minecraft (javaw.exe) n'est pas lancé".to_string() };
        }
        let all_ok = pids
            .iter()
            .all(|p| platform::get_priority_class(*p).map(|c| c == platform::PRIORITY_ABOVE_NORMAL || c > platform::PRIORITY_ABOVE_NORMAL).unwrap_or(false));
        if all_ok {
            Applicability::NotNeeded { reason: "priorité déjà élevée".to_string() }
        } else {
            Applicability::Applicable { current: format!("{} processus Java en priorité normale", pids.len()) }
        }
    }

    fn snapshot(&self) -> Result<String, String> {
        let procs: Vec<(u32, u32)> = game_pids()
            .into_iter()
            .filter_map(|p| platform::get_priority_class(p).map(|c| (p, c)))
            .collect();
        if procs.is_empty() {
            return Err("Aucun processus Java lisible.".to_string());
        }
        serde_json::to_string(&PrioritySnapshot { procs }).map_err(|e| e.to_string())
    }

    fn apply(&self) -> Result<(), String> {
        let mut any = false;
        for p in game_pids() {
            platform::set_priority_class(p, platform::PRIORITY_ABOVE_NORMAL)?;
            any = true;
        }
        if any {
            Ok(())
        } else {
            Err("javaw.exe a disparu.".to_string())
        }
    }

    fn verify(&self) -> Result<(), String> {
        for p in game_pids() {
            match platform::get_priority_class(p) {
                Some(c) if c == platform::PRIORITY_ABOVE_NORMAL => {}
                other => return Err(format!("PID {} : priorité relue {:?}", p, other)),
            }
        }
        Ok(())
    }

    fn revert(&self, snapshot: &str) -> Result<(), String> {
        let s: PrioritySnapshot = serde_json::from_str(snapshot).map_err(|e| e.to_string())?;
        let alive = game_pids();
        for (pid, class) in s.procs {
            // On ne restaure que si le PID est toujours un processus Java (evite la reutilisation de PID).
            if alive.contains(&pid) {
                let _ = platform::set_priority_class(pid, class);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ps_quote_escapes_apostrophes() {
        assert_eq!(ps_quote("Wi-Fi"), "'Wi-Fi'");
        assert_eq!(ps_quote("O'Brien"), "'O''Brien'");
    }

    #[test]
    fn every_tweak_has_unique_id_and_documentation() {
        let t = default_tweaks();
        let mut ids: Vec<&str> = t.iter().map(|x| x.info().id).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n);
        for tw in &t {
            let i = tw.info();
            assert!(!i.title.is_empty() && !i.why.is_empty() && !i.risk.is_empty());
            assert!(!tw.plan().is_empty());
        }
    }

    #[test]
    fn snapshots_roundtrip() {
        let s = serde_json::to_string(&PlanSnapshot { scheme: "abc".into(), ac: 3 }).unwrap();
        let back: PlanSnapshot = serde_json::from_str(&s).unwrap();
        assert_eq!(back.ac, 3);
        let s = serde_json::to_string(&PrioritySnapshot { procs: vec![(1, 32)] }).unwrap();
        let back: PrioritySnapshot = serde_json::from_str(&s).unwrap();
        assert_eq!(back.procs, vec![(1, 32)]);
    }

    #[test]
    fn detect_never_panics_on_runner() {
        // Execute les vraies detections (powercfg / PowerShell / processus) : doivent repondre
        // proprement meme sans Wi-Fi ni Minecraft.
        for tw in default_tweaks() {
            let _ = tw.detect();
        }
    }

    #[test]
    fn priority_tweak_unavailable_without_minecraft() {
        // Sur un runner CI, javaw.exe n'est pas lance.
        if game_pids().is_empty() {
            assert!(matches!(JavaPriority.detect(), Applicability::Unavailable { .. }));
        }
    }
}
