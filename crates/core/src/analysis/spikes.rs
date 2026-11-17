//! Detection de pics de latence et de motifs periodiques.

use super::stats::{mad, median};

#[derive(Clone, Debug, PartialEq)]
pub struct Spike {
    /// Debut du pic (ms epoch).
    pub t_ms: u64,
    pub peak_ms: f64,
    /// Nombre d'echantillons consecutifs concernes.
    pub samples: usize,
    pub lost: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Periodicity {
    pub period_s: f64,
    pub spike_count: usize,
}

/// Seuil de pic : mediane + max(15 ms, 4 x MAD robuste).
pub fn spike_threshold(rtts: &[f64]) -> f64 {
    if rtts.is_empty() {
        return f64::INFINITY;
    }
    let med = median(rtts);
    let robust_sigma = mad(rtts) * 1.4826;
    med + (4.0 * robust_sigma).max(15.0)
}

/// Regroupe les echantillons consecutifs anormaux (RTT > seuil ou perte) en pics.
pub fn detect_spikes(series: &[(u64, Option<f32>)]) -> Vec<Spike> {
    let rtts: Vec<f64> = series.iter().filter_map(|(_, r)| r.map(|v| v as f64)).collect();
    let thr = spike_threshold(&rtts);
    let mut out: Vec<Spike> = Vec::new();
    let mut cur: Option<Spike> = None;
    for (t, r) in series {
        let (is_spike, val, lost) = match r {
            Some(v) => ((*v as f64) > thr, *v as f64, false),
            None => (true, 0.0, true),
        };
        if is_spike {
            match cur.as_mut() {
                Some(c) => {
                    c.samples += 1;
                    if lost {
                        c.lost += 1;
                    }
                    if val > c.peak_ms {
                        c.peak_ms = val;
                    }
                }
                None => {
                    cur = Some(Spike {
                        t_ms: *t,
                        peak_ms: val,
                        samples: 1,
                        lost: if lost { 1 } else { 0 },
                    });
                }
            }
        } else if let Some(c) = cur.take() {
            out.push(c);
        }
    }
    if let Some(c) = cur.take() {
        out.push(c);
    }
    out
}

/// Detecte un rythme regulier (ex. scan Wi-Fi en arriere-plan, tache planifiee) :
/// au moins 4 pics dont >= 70 % des intervalles sont dans +/-15 % de la mediane (>= 5 s).
pub fn detect_periodicity(spikes: &[Spike]) -> Option<Periodicity> {
    if spikes.len() < 4 {
        return None;
    }
    let intervals: Vec<f64> = spikes
        .windows(2)
        .map(|w| w[1].t_ms.saturating_sub(w[0].t_ms) as f64 / 1000.0)
        .collect();
    let med = median(&intervals);
    if med < 5.0 {
        return None;
    }
    let close = intervals.iter().filter(|i| (**i - med).abs() <= 0.15 * med).count();
    if close as f64 / intervals.len() as f64 >= 0.7 {
        Some(Periodicity { period_s: med, spike_count: spikes.len() })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threshold_floor_is_15ms_above_median() {
        let v = vec![10.0; 20];
        assert!((spike_threshold(&v) - 25.0).abs() < 1e-9);
        assert!(spike_threshold(&[]).is_infinite());
    }

    #[test]
    fn clusters_consecutive_spikes() {
        let vals = [10.0, 10.0, 10.0, 100.0, 10.0, 10.0, 10.0, 100.0, 100.0, 10.0];
        let series: Vec<(u64, Option<f32>)> =
            vals.iter().enumerate().map(|(i, v)| (i as u64 * 1000, Some(*v as f32))).collect();
        let sp = detect_spikes(&series);
        assert_eq!(sp.len(), 2);
        assert_eq!(sp[0].t_ms, 3000);
        assert_eq!(sp[0].samples, 1);
        assert_eq!(sp[1].t_ms, 7000);
        assert_eq!(sp[1].samples, 2);
        assert!((sp[1].peak_ms - 100.0).abs() < 1e-9);
    }

    #[test]
    fn losses_count_as_spikes() {
        let mut series: Vec<(u64, Option<f32>)> = (0..10).map(|i| (i * 1000, Some(10.0))).collect();
        series[4].1 = None;
        series[5].1 = None;
        let sp = detect_spikes(&series);
        assert_eq!(sp.len(), 1);
        assert_eq!(sp[0].lost, 2);
    }

    #[test]
    fn detects_regular_period() {
        let spikes: Vec<Spike> = (0..5)
            .map(|i| Spike { t_ms: i * 60_000, peak_ms: 80.0, samples: 1, lost: 0 })
            .collect();
        let p = detect_periodicity(&spikes).unwrap();
        assert!((p.period_s - 60.0).abs() < 1e-9);
        assert_eq!(p.spike_count, 5);
    }

    #[test]
    fn irregular_spikes_are_not_periodic() {
        let times = [0u64, 7_000, 50_000, 61_000, 200_000];
        let spikes: Vec<Spike> = times
            .iter()
            .map(|t| Spike { t_ms: *t, peak_ms: 80.0, samples: 1, lost: 0 })
            .collect();
        assert!(detect_periodicity(&spikes).is_none());
    }

    #[test]
    fn too_few_spikes_is_none() {
        let spikes: Vec<Spike> = (0..3)
            .map(|i| Spike { t_ms: i * 60_000, peak_ms: 80.0, samples: 1, lost: 0 })
            .collect();
        assert!(detect_periodicity(&spikes).is_none());
    }
}
