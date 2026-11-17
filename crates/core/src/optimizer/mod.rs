//! Optimiseur transactionnel et reversible.
//!
//! Cycle d'un reglage : detecter -> snapshot -> journal(Pending) -> appliquer -> verifier
//! -> journal(Active). En cas d'echec : restauration immediate. Au demarrage, `recover()`
//! annule tout reglage reste "Pending" (plantage pendant l'application).
//! Aucun reglage n'est applique sans action explicite de l'utilisateur.

pub mod tweaks;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::store::{read_json, write_json_atomic};
use crate::util::now_ms;

#[derive(Clone, Debug)]
pub struct TweakInfo {
    pub id: &'static str,
    pub title: &'static str,
    pub what: &'static str,
    pub why: &'static str,
    pub risk: &'static str,
    pub requires_admin: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Applicability {
    Applicable { current: String },
    NotNeeded { reason: String },
    Unavailable { reason: String },
}

pub trait Tweak: Send + Sync {
    fn info(&self) -> TweakInfo;
    /// Commandes / appels exacts qui seront executes (affiches avant application).
    fn plan(&self) -> String;
    fn detect(&self) -> Applicability;
    /// Lit l'etat actuel et le serialise (utilise par revert).
    fn snapshot(&self) -> Result<String, String>;
    fn apply(&self) -> Result<(), String>;
    fn verify(&self) -> Result<(), String>;
    fn revert(&self, snapshot: &str) -> Result<(), String>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryState {
    Pending,
    Active,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JournalEntry {
    pub id: String,
    pub snapshot: String,
    pub state: EntryState,
    pub applied_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Journal {
    pub entries: Vec<JournalEntry>,
}

#[derive(Clone, Debug)]
pub struct TweakStatus {
    pub info: TweakInfo,
    pub plan: String,
    pub applicability: Applicability,
    pub active: bool,
}

pub struct Optimizer {
    tweaks: Vec<Box<dyn Tweak>>,
    journal_path: PathBuf,
    lock: Mutex<()>,
}

impl Optimizer {
    pub fn new(dir: &Path, tweaks: Vec<Box<dyn Tweak>>) -> Optimizer {
        let _ = std::fs::create_dir_all(dir);
        Optimizer { tweaks, journal_path: dir.join("tweaks-journal.json"), lock: Mutex::new(()) }
    }

    pub fn with_default_tweaks(dir: &Path) -> Optimizer {
        Optimizer::new(dir, tweaks::default_tweaks())
    }

    fn load_journal(&self) -> Journal {
        read_json::<Journal>(&self.journal_path).unwrap_or_default()
    }

    fn save_journal(&self, j: &Journal) -> Result<(), String> {
        write_json_atomic(&self.journal_path, j)
    }

    fn find(&self, id: &str) -> Result<&dyn Tweak, String> {
        self.tweaks
            .iter()
            .find(|t| t.info().id == id)
            .map(|b| &**b)
            .ok_or_else(|| format!("Réglage inconnu : {}", id))
    }

    pub fn active_ids(&self) -> Vec<String> {
        self.load_journal()
            .entries
            .into_iter()
            .filter(|e| e.state == EntryState::Active)
            .map(|e| e.id)
            .collect()
    }

    pub fn status(&self) -> Vec<TweakStatus> {
        let active = self.active_ids();
        self.tweaks
            .iter()
            .map(|t| TweakStatus {
                info: t.info(),
                plan: t.plan(),
                applicability: t.detect(),
                active: active.iter().any(|a| a == t.info().id),
            })
            .collect()
    }

    pub fn apply(&self, id: &str) -> Result<String, String> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let tw = self.find(id)?;
        let mut j = self.load_journal();
        if j.entries.iter().any(|e| e.id == id) {
            return Err("Ce réglage est déjà appliqué.".to_string());
        }
        match tw.detect() {
            Applicability::Applicable { .. } => {}
            Applicability::NotNeeded { reason } => return Err(format!("Inutile : {}", reason)),
            Applicability::Unavailable { reason } => return Err(format!("Indisponible : {}", reason)),
        }
        let snap = tw.snapshot()?;
        j.entries.push(JournalEntry {
            id: id.to_string(),
            snapshot: snap.clone(),
            state: EntryState::Pending,
            applied_ms: now_ms(),
        });
        self.save_journal(&j)?;

        let fail = |this: &Optimizer, mut j: Journal, msg: String| -> String {
            let rb = tw.revert(&snap);
            j.entries.retain(|e| e.id != id);
            let _ = this.save_journal(&j);
            match rb {
                Ok(()) => format!("{} — l'état initial a été restauré.", msg),
                Err(e) => format!("{} — ATTENTION, la restauration a échoué : {}", msg, e),
            }
        };

        if let Err(e) = tw.apply() {
            return Err(fail(self, j, format!("Échec de l'application : {}", e)));
        }
        if let Err(e) = tw.verify() {
            return Err(fail(self, j, format!("Vérification échouée : {}", e)));
        }
        for e in j.entries.iter_mut() {
            if e.id == id {
                e.state = EntryState::Active;
            }
        }
        self.save_journal(&j)?;
        Ok(format!("« {} » appliqué et vérifié.", tw.info().title))
    }

    pub fn revert(&self, id: &str) -> Result<(), String> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let tw = self.find(id)?;
        let mut j = self.load_journal();
        let entry = j
            .entries
            .iter()
            .find(|e| e.id == id)
            .cloned()
            .ok_or_else(|| "Ce réglage n'est pas appliqué.".to_string())?;
        tw.revert(&entry.snapshot)?;
        j.entries.retain(|e| e.id != id);
        self.save_journal(&j)
    }

    pub fn revert_all(&self) -> Vec<(String, Result<(), String>)> {
        let ids: Vec<String> = self.load_journal().entries.into_iter().map(|e| e.id).collect();
        ids.into_iter()
            .map(|id| {
                let r = self.revert(&id);
                (id, r)
            })
            .collect()
    }

    /// Annule les reglages restes "Pending" (plantage en cours d'application).
    pub fn recover(&self) -> Vec<String> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut j = self.load_journal();
        let mut msgs = Vec::new();
        let pending: Vec<JournalEntry> =
            j.entries.iter().filter(|e| e.state == EntryState::Pending).cloned().collect();
        for e in pending {
            match self.find(&e.id) {
                Ok(tw) => match tw.revert(&e.snapshot) {
                    Ok(()) => msgs.push(format!("Réglage « {} » interrompu : état initial restauré.", e.id)),
                    Err(err) => msgs.push(format!("Restauration de « {} » impossible : {}", e.id, err)),
                },
                Err(err) => msgs.push(err),
            }
            j.entries.retain(|x| x.id != e.id);
        }
        if !msgs.is_empty() {
            let _ = self.save_journal(&j);
        }
        msgs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI32, Ordering};
    use std::sync::Arc;

    struct Mock {
        value: Arc<AtomicI32>,
        fail_apply: bool,
        fail_verify: bool,
        available: bool,
    }

    impl Tweak for Mock {
        fn info(&self) -> TweakInfo {
            TweakInfo {
                id: "mock",
                title: "Mock",
                what: "",
                why: "",
                risk: "",
                requires_admin: false,
            }
        }
        fn plan(&self) -> String {
            "set value = 1".to_string()
        }
        fn detect(&self) -> Applicability {
            if !self.available {
                Applicability::Unavailable { reason: "absent".to_string() }
            } else if self.value.load(Ordering::SeqCst) == 1 {
                Applicability::NotNeeded { reason: "déjà à 1".to_string() }
            } else {
                Applicability::Applicable { current: self.value.load(Ordering::SeqCst).to_string() }
            }
        }
        fn snapshot(&self) -> Result<String, String> {
            Ok(self.value.load(Ordering::SeqCst).to_string())
        }
        fn apply(&self) -> Result<(), String> {
            if self.fail_apply {
                // Echec apres modification partielle : le rollback doit tout restaurer.
                self.value.store(99, Ordering::SeqCst);
                return Err("boom".to_string());
            }
            self.value.store(1, Ordering::SeqCst);
            Ok(())
        }
        fn verify(&self) -> Result<(), String> {
            if self.fail_verify {
                Err("pas à 1".to_string())
            } else {
                Ok(())
            }
        }
        fn revert(&self, snapshot: &str) -> Result<(), String> {
            let v: i32 = snapshot.parse().map_err(|_| "snapshot invalide".to_string())?;
            self.value.store(v, Ordering::SeqCst);
            Ok(())
        }
    }

    fn opt(tag: &str, value: &Arc<AtomicI32>, fail_apply: bool, fail_verify: bool, available: bool) -> Optimizer {
        let dir = std::env::temp_dir().join(format!("mcnet-opt-{}-{}-{}", tag, std::process::id(), now_ms()));
        Optimizer::new(
            &dir,
            vec![Box::new(Mock { value: value.clone(), fail_apply, fail_verify, available })],
        )
    }

    #[test]
    fn apply_then_revert_restores_state() {
        let v = Arc::new(AtomicI32::new(5));
        let o = opt("ok", &v, false, false, true);
        o.apply("mock").unwrap();
        assert_eq!(v.load(Ordering::SeqCst), 1);
        assert_eq!(o.active_ids(), vec!["mock".to_string()]);
        assert!(o.status()[0].active);
        o.revert("mock").unwrap();
        assert_eq!(v.load(Ordering::SeqCst), 5);
        assert!(o.active_ids().is_empty());
    }

    #[test]
    fn double_apply_is_refused() {
        let v = Arc::new(AtomicI32::new(5));
        let o = opt("dbl", &v, false, false, true);
        o.apply("mock").unwrap();
        assert!(o.apply("mock").is_err());
    }

    #[test]
    fn failed_apply_rolls_back() {
        let v = Arc::new(AtomicI32::new(5));
        let o = opt("fa", &v, true, false, true);
        let err = o.apply("mock").unwrap_err();
        assert!(err.contains("restauré"), "{}", err);
        assert_eq!(v.load(Ordering::SeqCst), 5);
        assert!(o.active_ids().is_empty());
    }

    #[test]
    fn failed_verify_rolls_back() {
        let v = Arc::new(AtomicI32::new(5));
        let o = opt("fv", &v, false, true, true);
        assert!(o.apply("mock").is_err());
        assert_eq!(v.load(Ordering::SeqCst), 5);
        assert!(o.active_ids().is_empty());
    }

    #[test]
    fn unavailable_and_not_needed_are_refused() {
        let v = Arc::new(AtomicI32::new(5));
        let o = opt("un", &v, false, false, false);
        assert!(o.apply("mock").unwrap_err().contains("Indisponible"));
        let v2 = Arc::new(AtomicI32::new(1));
        let o2 = opt("nn", &v2, false, false, true);
        assert!(o2.apply("mock").unwrap_err().contains("Inutile"));
        assert!(o2.apply("inconnu").is_err());
    }

    #[test]
    fn recover_reverts_pending_entries() {
        let v = Arc::new(AtomicI32::new(1)); // etat "modifie" laisse par un plantage
        let o = opt("rec", &v, false, false, true);
        let j = Journal {
            entries: vec![JournalEntry {
                id: "mock".to_string(),
                snapshot: "5".to_string(),
                state: EntryState::Pending,
                applied_ms: 0,
            }],
        };
        o.save_journal(&j).unwrap();
        let msgs = o.recover();
        assert_eq!(msgs.len(), 1);
        assert_eq!(v.load(Ordering::SeqCst), 5);
        assert!(o.load_journal().entries.is_empty());
    }

    #[test]
    fn revert_all_clears_everything() {
        let v = Arc::new(AtomicI32::new(7));
        let o = opt("all", &v, false, false, true);
        o.apply("mock").unwrap();
        let r = o.revert_all();
        assert_eq!(r.len(), 1);
        assert!(r[0].1.is_ok());
        assert_eq!(v.load(Ordering::SeqCst), 7);
    }
}
