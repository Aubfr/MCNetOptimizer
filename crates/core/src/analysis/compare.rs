//! Comparaison A/B de groupes de sessions (protocole A,B,A,B).
//!
//! Methode : les echantillons d'une session sont fortement auto-correles, un test
//! directement sur les echantillons surestimerait la significativite. On agrege donc
//! chaque session en blocs de 30 s (mediane par bloc) et on compare les distributions
//! de blocs avec un test de Mann-Whitney (non parametrique) + taille d'effet (delta de Cliff).
//! Une amelioration n'est declaree que si p < 0,01, effet >= 0,147, ecart de p95 >= 1 ms,
//! et au moins 2 sessions par groupe.

use std::collections::BTreeMap;

use crate::model::{Session, Target};

use super::stats::{compute_stats, median, to_f64, Stats};

pub const BLOCK_MS: u64 = 30_000;
pub const MIN_BLOCKS: usize = 20;
pub const MIN_SESSIONS: usize = 2;
pub const ALPHA: f64 = 0.01;
pub const MIN_EFFECT: f64 = 0.147;
pub const MIN_P95_DELTA_MS: f64 = 1.0;

/// erfc par l'approximation d'Abramowitz-Stegun 7.1.26 (erreur < 1,5e-7).
pub fn erfc(x: f64) -> f64 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.3275911 * z);
    let poly = ((((1.061405429 * t - 1.453152027) * t + 1.421413741) * t - 0.284496736) * t
        + 0.254829592)
        * t;
    let r = poly * (-z * z).exp();
    if x >= 0.0 {
        r
    } else {
        2.0 - r
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MwResult {
    pub u: f64,
    pub z: f64,
    /// p-value bilaterale (approximation normale, correction de continuite et des ex-aequo).
    pub p: f64,
    /// Delta de Cliff = P(a > b) - P(a < b), dans [-1, 1].
    pub cliffs_delta: f64,
}

pub fn mann_whitney(a: &[f64], b: &[f64]) -> Option<MwResult> {
    let n1 = a.len();
    let n2 = b.len();
    if n1 < 2 || n2 < 2 {
        return None;
    }
    let mut all: Vec<(f64, bool)> = a
        .iter()
        .map(|&v| (v, true))
        .chain(b.iter().map(|&v| (v, false)))
        .collect();
    all.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(std::cmp::Ordering::Equal));
    let n = all.len();
    let mut r1 = 0.0;
    let mut tie_sum = 0.0;
    let mut i = 0usize;
    while i < n {
        let mut j = i;
        while j + 1 < n && all[j + 1].0 == all[i].0 {
            j += 1;
        }
        let cnt = (j - i + 1) as f64;
        let avg_rank = ((i + 1 + j + 1) as f64) / 2.0;
        for item in all.iter().take(j + 1).skip(i) {
            if item.1 {
                r1 += avg_rank;
            }
        }
        tie_sum += cnt * cnt * cnt - cnt;
        i = j + 1;
    }
    let n1f = n1 as f64;
    let n2f = n2 as f64;
    let nf = n as f64;
    let u1 = r1 - n1f * (n1f + 1.0) / 2.0;
    let mean = n1f * n2f / 2.0;
    let var = n1f * n2f / 12.0 * ((nf + 1.0) - tie_sum / (nf * (nf - 1.0)));
    let (z, p) = if var <= 0.0 {
        (0.0, 1.0)
    } else {
        let diff = u1 - mean;
        let cc = if diff > 0.0 {
            -0.5
        } else if diff < 0.0 {
            0.5
        } else {
            0.0
        };
        let z = (diff + cc) / var.sqrt();
        (z, erfc(z.abs() / std::f64::consts::SQRT_2))
    };
    let delta = 2.0 * u1 / (n1f * n2f) - 1.0;
    Some(MwResult { u: u1, z, p, cliffs_delta: delta })
}

/// Medianes par bloc de `block_ms` (blocs avec >= 3 echantillons recus).
pub fn block_medians(series: &[(u64, Option<f32>)], block_ms: u64) -> Vec<f64> {
    if series.is_empty() || block_ms == 0 {
        return Vec::new();
    }
    let t0 = series[0].0;
    let mut blocks: BTreeMap<u64, Vec<f64>> = BTreeMap::new();
    for (t, r) in series {
        if let Some(v) = r {
            blocks.entry(t.saturating_sub(t0) / block_ms).or_default().push(*v as f64);
        }
    }
    blocks.values().filter(|v| v.len() >= 3).map(|v| median(v)).collect()
}

#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    /// Donnees insuffisantes pour conclure (message explicatif).
    Insufficient(String),
    /// Aucune difference demontree.
    NoDifference,
    /// B est demontre plus rapide que A.
    BetterB,
    /// B est demontre plus lent que A.
    WorseB,
}

impl Verdict {
    pub fn text(&self) -> String {
        match self {
            Verdict::Insufficient(m) => format!("Insuffisant : {}", m),
            Verdict::NoDifference => "Aucune amélioration démontrée".to_string(),
            Verdict::BetterB => "B est démontré plus rapide que A".to_string(),
            Verdict::WorseB => "B est démontré plus lent que A".to_string(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct TargetComparison {
    pub target: Target,
    pub a: Stats,
    pub b: Stats,
    pub p50_delta: f64,
    pub p95_delta: f64,
    pub jitter_delta: f64,
    pub loss_delta: f64,
    pub blocks_a: usize,
    pub blocks_b: usize,
    pub mw: Option<MwResult>,
    /// (paires ou B est plus rapide, nombre de paires)
    pub pairs_b_faster: (usize, usize),
    pub verdict: Verdict,
}

fn pooled(sessions: &[&Session], target: Target) -> (Vec<Option<f64>>, Vec<f64>) {
    let mut all: Vec<Option<f64>> = Vec::new();
    let mut blocks: Vec<f64> = Vec::new();
    for s in sessions {
        let series = s.series(target);
        all.extend(to_f64(&series));
        blocks.extend(block_medians(&series, BLOCK_MS));
    }
    (all, blocks)
}

pub fn compare_target(a: &[&Session], b: &[&Session], target: Target) -> TargetComparison {
    let (all_a, blocks_a) = pooled(a, target);
    let (all_b, blocks_b) = pooled(b, target);
    let sa = compute_stats(&all_a);
    let sb = compute_stats(&all_b);
    let mw = mann_whitney(&blocks_a, &blocks_b);

    let mut sa_sorted: Vec<&Session> = a.to_vec();
    let mut sb_sorted: Vec<&Session> = b.to_vec();
    sa_sorted.sort_by_key(|s| s.started_ms);
    sb_sorted.sort_by_key(|s| s.started_ms);
    let n_pairs = sa_sorted.len().min(sb_sorted.len());
    let mut faster = 0usize;
    for i in 0..n_pairs {
        let pa = compute_stats(&to_f64(&sa_sorted[i].series(target)));
        let pb = compute_stats(&to_f64(&sb_sorted[i].series(target)));
        if pa.has_data() && pb.has_data() && pb.p50 < pa.p50 {
            faster += 1;
        }
    }

    let p50_delta = sb.p50 - sa.p50;
    let p95_delta = sb.p95 - sa.p95;

    let verdict = if !sa.has_data() || !sb.has_data() {
        Verdict::Insufficient("aucune donnée pour cette cible dans un des groupes".to_string())
    } else if a.len() < MIN_SESSIONS || b.len() < MIN_SESSIONS {
        Verdict::Insufficient(format!(
            "il faut au moins {} sessions par groupe (A : {}, B : {}) pour juger la reproductibilité",
            MIN_SESSIONS,
            a.len(),
            b.len()
        ))
    } else if blocks_a.len() < MIN_BLOCKS || blocks_b.len() < MIN_BLOCKS {
        Verdict::Insufficient(format!(
            "pas assez de blocs de 30 s (A : {}, B : {}, minimum {})",
            blocks_a.len(),
            blocks_b.len(),
            MIN_BLOCKS
        ))
    } else if let Some(m) = &mw {
        if m.p < ALPHA && m.cliffs_delta.abs() >= MIN_EFFECT && p95_delta.abs() >= MIN_P95_DELTA_MS {
            // delta de Cliff > 0 : A tend a etre plus grand que B, donc B plus rapide.
            if m.cliffs_delta > 0.0 {
                Verdict::BetterB
            } else {
                Verdict::WorseB
            }
        } else {
            Verdict::NoDifference
        }
    } else {
        Verdict::Insufficient("test statistique impossible".to_string())
    };

    TargetComparison {
        target,
        a: sa.clone(),
        b: sb.clone(),
        p50_delta,
        p95_delta,
        jitter_delta: sb.jitter - sa.jitter,
        loss_delta: sb.loss_pct - sa.loss_pct,
        blocks_a: blocks_a.len(),
        blocks_b: blocks_b.len(),
        mw,
        pairs_b_faster: (faster, n_pairs),
        verdict,
    }
}

/// Compare A et B pour toutes les cibles ayant des donnees dans au moins un groupe.
pub fn compare_groups(a: &[&Session], b: &[&Session]) -> Vec<TargetComparison> {
    let mut out = Vec::new();
    for t in Target::ALL {
        let has = a.iter().chain(b.iter()).any(|s| s.samples.iter().any(|x| x.target == t));
        if has {
            out.push(compare_target(a, b, t));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Sample;

    fn make_session(start_ms: u64, n: usize, base: f32) -> Session {
        let mut s = Session::default();
        s.started_ms = start_ms;
        s.ended_ms = start_ms + n as u64 * 1000;
        for i in 0..n {
            s.samples.push(Sample {
                t_ms: start_ms + i as u64 * 1000,
                target: Target::Gateway,
                rtt_ms: Some(base + (i % 3) as f32),
            });
        }
        s
    }

    #[test]
    fn erfc_known_values() {
        assert!((erfc(0.0) - 1.0).abs() < 1e-6);
        assert!((erfc(1.0) - 0.157299).abs() < 1e-4);
        assert!((erfc(-1.0) - 1.842701).abs() < 1e-4);
    }

    #[test]
    fn mw_fully_separated() {
        let a = [1.0, 2.0, 3.0, 4.0, 5.0];
        let b = [6.0, 7.0, 8.0, 9.0, 10.0];
        let r = mann_whitney(&a, &b).unwrap();
        assert!((r.u - 0.0).abs() < 1e-9);
        assert!((r.cliffs_delta + 1.0).abs() < 1e-9);
        assert!(r.p < 0.02 && r.p > 0.005, "p = {}", r.p);
    }

    #[test]
    fn mw_identical_samples() {
        let a = [1.0, 2.0, 3.0, 4.0, 5.0];
        let r = mann_whitney(&a, &a).unwrap();
        assert!(r.cliffs_delta.abs() < 1e-9);
        assert!(r.p > 0.99);
    }

    #[test]
    fn mw_needs_two_per_group() {
        assert!(mann_whitney(&[1.0], &[2.0, 3.0]).is_none());
    }

    #[test]
    fn block_medians_groups_by_30s() {
        // 60 echantillons de 1 s -> 2 blocs de 30
        let series: Vec<(u64, Option<f32>)> = (0..60u64)
            .map(|i| (i * 1000, Some(if i < 30 { 10.0 } else { 20.0 })))
            .collect();
        let b = block_medians(&series, BLOCK_MS);
        assert_eq!(b.len(), 2);
        assert!((b[0] - 10.0).abs() < 1e-9);
        assert!((b[1] - 20.0).abs() < 1e-9);
    }

    #[test]
    fn clear_improvement_is_detected() {
        let a = vec![make_session(0, 1200, 20.0), make_session(10_000_000, 1200, 20.0)];
        let b = vec![make_session(5_000_000, 1200, 10.0), make_session(15_000_000, 1200, 10.0)];
        let ar: Vec<&Session> = a.iter().collect();
        let br: Vec<&Session> = b.iter().collect();
        let c = compare_target(&ar, &br, Target::Gateway);
        assert_eq!(c.verdict, Verdict::BetterB);
        assert!(c.p95_delta < -5.0);
        assert_eq!(c.pairs_b_faster, (2, 2));
    }

    #[test]
    fn identical_groups_show_no_difference() {
        let a = vec![make_session(0, 1200, 15.0), make_session(10_000_000, 1200, 15.0)];
        let b = vec![make_session(5_000_000, 1200, 15.0), make_session(15_000_000, 1200, 15.0)];
        let ar: Vec<&Session> = a.iter().collect();
        let br: Vec<&Session> = b.iter().collect();
        let c = compare_target(&ar, &br, Target::Gateway);
        assert_eq!(c.verdict, Verdict::NoDifference);
    }

    #[test]
    fn single_session_per_group_is_insufficient() {
        let a = vec![make_session(0, 1200, 20.0)];
        let b = vec![make_session(5_000_000, 1200, 10.0)];
        let ar: Vec<&Session> = a.iter().collect();
        let br: Vec<&Session> = b.iter().collect();
        let c = compare_target(&ar, &br, Target::Gateway);
        assert!(matches!(c.verdict, Verdict::Insufficient(_)));
    }

    #[test]
    fn short_sessions_are_insufficient() {
        let a = vec![make_session(0, 120, 20.0), make_session(1_000_000, 120, 20.0)];
        let b = vec![make_session(500_000, 120, 10.0), make_session(1_500_000, 120, 10.0)];
        let ar: Vec<&Session> = a.iter().collect();
        let br: Vec<&Session> = b.iter().collect();
        let c = compare_target(&ar, &br, Target::Gateway);
        assert!(matches!(c.verdict, Verdict::Insufficient(_)));
    }
}
