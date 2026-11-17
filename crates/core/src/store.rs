//! Persistance : sessions (JSON), parametres, export CSV. Ecritures atomiques.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{de::DeserializeOwned, Serialize};

use crate::config::Settings;
use crate::model::{Session, SessionGroup, SessionMeta, Target};
use crate::util::fmt_time;

pub struct Store {
    pub dir: PathBuf,
}

pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let json = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp).map_err(|e| format!("{}: {}", tmp.display(), e))?;
        f.write_all(&json).map_err(|e| e.to_string())?;
        f.flush().map_err(|e| e.to_string())?;
    }
    fs::rename(&tmp, path).map_err(|e| format!("{}: {}", path.display(), e))
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    let data = fs::read(path).map_err(|e| format!("{}: {}", path.display(), e))?;
    serde_json::from_slice(&data).map_err(|e| format!("{}: {}", path.display(), e))
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

impl Store {
    pub fn new(dir: PathBuf) -> Result<Store, String> {
        fs::create_dir_all(dir.join("sessions")).map_err(|e| format!("{}: {}", dir.display(), e))?;
        Ok(Store { dir })
    }

    /// %APPDATA%\McNetOptimizer (ou le dossier courant en dernier recours).
    pub fn default_dir() -> PathBuf {
        match std::env::var_os("APPDATA") {
            Some(a) => PathBuf::from(a).join("McNetOptimizer"),
            None => PathBuf::from("McNetOptimizerData"),
        }
    }

    fn sessions_dir(&self) -> PathBuf {
        self.dir.join("sessions")
    }

    fn session_path(&self, id: &str) -> PathBuf {
        self.sessions_dir().join(format!("{}.json", id))
    }

    fn meta_path(&self, id: &str) -> PathBuf {
        self.sessions_dir().join(format!("{}.meta.json", id))
    }

    pub fn settings_path(&self) -> PathBuf {
        self.dir.join("settings.json")
    }

    pub fn load_settings(&self) -> Settings {
        read_json::<Settings>(&self.settings_path()).unwrap_or_default()
    }

    pub fn save_settings(&self, s: &Settings) -> Result<(), String> {
        write_json_atomic(&self.settings_path(), s)
    }

    pub fn save_session(&self, s: &Session) -> Result<(), String> {
        if !valid_id(&s.id) {
            return Err(format!("Identifiant de session invalide : {}", s.id));
        }
        write_json_atomic(&self.session_path(&s.id), s)?;
        write_json_atomic(&self.meta_path(&s.id), &s.meta())
    }

    pub fn load_session(&self, id: &str) -> Result<Session, String> {
        if !valid_id(id) {
            return Err("Identifiant invalide".to_string());
        }
        read_json(&self.session_path(id))
    }

    /// Liste des sessions (la plus recente d'abord). Reconstruit le meta si absent.
    pub fn list(&self) -> Vec<SessionMeta> {
        let mut out: Vec<SessionMeta> = Vec::new();
        let rd = match fs::read_dir(self.sessions_dir()) {
            Ok(r) => r,
            Err(_) => return out,
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if let Some(id) = name.strip_suffix(".meta.json") {
                if let Ok(m) = read_json::<SessionMeta>(&e.path()) {
                    out.push(m);
                } else if valid_id(id) {
                    if let Ok(s) = self.load_session(id) {
                        out.push(s.meta());
                    }
                }
            }
        }
        out.sort_by(|a, b| b.started_ms.cmp(&a.started_ms));
        out
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        if !valid_id(id) {
            return Err("Identifiant invalide".to_string());
        }
        let _ = fs::remove_file(self.meta_path(id));
        fs::remove_file(self.session_path(id)).map_err(|e| e.to_string())
    }

    pub fn update_label(&self, id: &str, label: &str, group: SessionGroup) -> Result<(), String> {
        let mut s = self.load_session(id)?;
        s.label = label.to_string();
        s.group = group;
        self.save_session(&s)
    }

    /// Exporte les echantillons en CSV (UTF-8) dans <dir>/exports/<id>.csv.
    pub fn export_csv(&self, s: &Session) -> Result<PathBuf, String> {
        if !valid_id(&s.id) {
            return Err("Identifiant invalide".to_string());
        }
        let dir = self.dir.join("exports");
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join(format!("{}.csv", s.id));
        let mut out = String::from("time_local,t_ms,target,rtt_ms\n");
        let mut samples = s.samples.clone();
        samples.sort_by_key(|x| x.t_ms);
        for x in &samples {
            let tname = match x.target {
                Target::Gateway => "gateway",
                Target::Internet => "internet",
                Target::Server => "server",
                Target::Game => "game",
            };
            let rtt = match x.rtt_ms {
                Some(v) => format!("{:.3}", v),
                None => "loss".to_string(),
            };
            out.push_str(&format!("{},{},{},{}\n", fmt_time(x.t_ms, "%Y-%m-%d %H:%M:%S%.3f"), x.t_ms, tname, rtt));
        }
        fs::write(&path, out).map_err(|e| e.to_string())?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Sample;

    fn temp_store(tag: &str) -> Store {
        let dir = std::env::temp_dir().join(format!(
            "mcnet-test-{}-{}-{}",
            tag,
            std::process::id(),
            crate::util::now_ms()
        ));
        Store::new(dir).unwrap()
    }

    fn sample_session(id: &str, start: u64) -> Session {
        let mut s = Session::default();
        s.id = id.to_string();
        s.label = "test".to_string();
        s.started_ms = start;
        s.ended_ms = start + 5000;
        s.samples.push(Sample { t_ms: start, target: Target::Gateway, rtt_ms: Some(2.5) });
        s.samples.push(Sample { t_ms: start + 1000, target: Target::Gateway, rtt_ms: None });
        s
    }

    #[test]
    fn save_load_list_delete() {
        let st = temp_store("sld");
        st.save_session(&sample_session("20260101-000001", 1000)).unwrap();
        st.save_session(&sample_session("20260101-000002", 2000)).unwrap();
        let list = st.list();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, "20260101-000002");
        let back = st.load_session("20260101-000001").unwrap();
        assert_eq!(back.samples.len(), 2);
        st.delete("20260101-000001").unwrap();
        assert_eq!(st.list().len(), 1);
        let _ = fs::remove_dir_all(&st.dir);
    }

    #[test]
    fn update_label_persists_group() {
        let st = temp_store("lbl");
        st.save_session(&sample_session("s1", 10)).unwrap();
        st.update_label("s1", "avec tweak", SessionGroup::B).unwrap();
        let s = st.load_session("s1").unwrap();
        assert_eq!(s.group, SessionGroup::B);
        assert_eq!(s.label, "avec tweak");
        assert_eq!(st.list()[0].group, SessionGroup::B);
        let _ = fs::remove_dir_all(&st.dir);
    }

    #[test]
    fn invalid_ids_rejected() {
        let st = temp_store("inv");
        assert!(st.load_session("../evil").is_err());
        assert!(st.delete("a/b").is_err());
        let mut s = sample_session("x", 1);
        s.id = "bad id".to_string();
        assert!(st.save_session(&s).is_err());
        let _ = fs::remove_dir_all(&st.dir);
    }

    #[test]
    fn settings_roundtrip_and_default() {
        let st = temp_store("set");
        assert_eq!(st.load_settings(), Settings::default());
        let mut s = Settings::default();
        s.hotkey_fn = 10;
        st.save_settings(&s).unwrap();
        assert_eq!(st.load_settings().hotkey_fn, 10);
        let _ = fs::remove_dir_all(&st.dir);
    }

    #[test]
    fn csv_export_contains_rows() {
        let st = temp_store("csv");
        let s = sample_session("csv1", 1_700_000_000_000);
        let p = st.export_csv(&s).unwrap();
        let txt = fs::read_to_string(&p).unwrap();
        assert!(txt.starts_with("time_local,t_ms,target,rtt_ms"));
        assert!(txt.contains(",gateway,2.500"));
        assert!(txt.contains(",gateway,loss"));
        let _ = fs::remove_dir_all(&st.dir);
    }
}
