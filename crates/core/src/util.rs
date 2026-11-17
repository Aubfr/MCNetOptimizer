//! Petits utilitaires (horloge, formatage).

use std::time::{SystemTime, UNIX_EPOCH};

/// Millisecondes depuis l'epoch Unix.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Formate un horodatage (ms epoch) en heure locale selon un format chrono.
pub fn fmt_time(ms: u64, fmt: &str) -> String {
    match chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms as i64) {
        Some(d) => d.with_timezone(&chrono::Local).format(fmt).to_string(),
        None => String::from("?"),
    }
}

/// Identifiant de session lisible : AAAAMMJJ-HHMMSS (heure locale).
pub fn session_id(ms: u64) -> String {
    fmt_time(ms, "%Y%m%d-%H%M%S")
}

/// Duree lisible : 1h 02m 03s / 4m 05s / 6s.
pub fn fmt_duration(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{}h {:02}m {:02}s", h, m, s)
    } else if m > 0 {
        format!("{}m {:02}s", m, s)
    } else {
        format!("{}s", s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_format() {
        assert_eq!(fmt_duration(6), "6s");
        assert_eq!(fmt_duration(65), "1m 05s");
        assert_eq!(fmt_duration(3723), "1h 02m 03s");
    }

    #[test]
    fn now_is_after_2020() {
        assert!(now_ms() > 1_577_836_800_000);
    }
}
