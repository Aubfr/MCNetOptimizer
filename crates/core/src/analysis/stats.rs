//! Statistiques de latence : percentiles, gigue, pertes.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    pub total: usize,
    pub received: usize,
    pub lost: usize,
    pub loss_pct: f64,
    pub min: f64,
    pub mean: f64,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
    pub stddev: f64,
    /// Gigue : moyenne des ecarts absolus entre RTT consecutifs recus (ms).
    pub jitter: f64,
}

impl Stats {
    pub fn has_data(&self) -> bool {
        self.received > 0
    }
}

/// Percentile (interpolation lineaire) sur un tableau DEJA trie. `p` dans [0, 100].
pub fn percentile_sorted(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    let rank = (p / 100.0) * (sorted.len() as f64 - 1.0);
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    let frac = rank - lo as f64;
    sorted[lo] + (sorted[hi] - sorted[lo]) * frac
}

fn sorted_copy(values: &[f64]) -> Vec<f64> {
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    v
}

pub fn median(values: &[f64]) -> f64 {
    percentile_sorted(&sorted_copy(values), 50.0)
}

/// Deviation absolue mediane (non mise a l'echelle).
pub fn mad(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let m = median(values);
    let devs: Vec<f64> = values.iter().map(|v| (v - m).abs()).collect();
    median(&devs)
}

/// Convertit une serie (t, rtt f32) en serie f64 sans les temps.
pub fn to_f64(series: &[(u64, Option<f32>)]) -> Vec<Option<f64>> {
    series.iter().map(|(_, r)| r.map(|v| v as f64)).collect()
}

pub fn compute_stats(series: &[Option<f64>]) -> Stats {
    let total = series.len();
    let rtts: Vec<f64> = series.iter().filter_map(|x| *x).collect();
    let received = rtts.len();
    let lost = total - received;
    let mut st = Stats {
        total,
        received,
        lost,
        loss_pct: if total > 0 { lost as f64 * 100.0 / total as f64 } else { 0.0 },
        ..Default::default()
    };
    if received == 0 {
        return st;
    }
    let sorted = sorted_copy(&rtts);
    st.min = sorted[0];
    st.max = sorted[received - 1];
    st.mean = rtts.iter().sum::<f64>() / received as f64;
    st.p50 = percentile_sorted(&sorted, 50.0);
    st.p95 = percentile_sorted(&sorted, 95.0);
    st.p99 = percentile_sorted(&sorted, 99.0);
    let var = rtts.iter().map(|v| (v - st.mean).powi(2)).sum::<f64>() / received as f64;
    st.stddev = var.sqrt();
    if received > 1 {
        let s: f64 = rtts.windows(2).map(|w| (w[1] - w[0]).abs()).sum();
        st.jitter = s / (received as f64 - 1.0);
    }
    st
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LossInfo {
    /// Pertes isolees (1 paquet).
    pub isolated: usize,
    /// Rafales (2 pertes consecutives ou plus).
    pub bursts: usize,
    pub longest: usize,
}

pub fn loss_bursts(series: &[Option<f64>]) -> LossInfo {
    let mut info = LossInfo::default();
    let mut run = 0usize;
    let flush = |run: usize, info: &mut LossInfo| {
        if run == 1 {
            info.isolated += 1;
        } else if run >= 2 {
            info.bursts += 1;
        }
        if run > info.longest {
            info.longest = run;
        }
    };
    for x in series {
        if x.is_none() {
            run += 1;
        } else {
            flush(run, &mut info);
            run = 0;
        }
    }
    flush(run, &mut info);
    info
}

/// Coefficient de correlation de Pearson. None si variance nulle ou < 3 points.
pub fn pearson(x: &[f64], y: &[f64]) -> Option<f64> {
    let n = x.len().min(y.len());
    if n < 3 {
        return None;
    }
    let mx = x[..n].iter().sum::<f64>() / n as f64;
    let my = y[..n].iter().sum::<f64>() / n as f64;
    let mut sxy = 0.0;
    let mut sxx = 0.0;
    let mut syy = 0.0;
    for i in 0..n {
        let dx = x[i] - mx;
        let dy = y[i] - my;
        sxy += dx * dy;
        sxx += dx * dx;
        syy += dy * dy;
    }
    if sxx <= 0.0 || syy <= 0.0 {
        return None;
    }
    Some(sxy / (sxx * syy).sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn percentile_interpolates() {
        let v = [10.0, 20.0, 30.0];
        assert!(close(percentile_sorted(&v, 0.0), 10.0));
        assert!(close(percentile_sorted(&v, 50.0), 20.0));
        assert!(close(percentile_sorted(&v, 95.0), 29.0));
        assert!(close(percentile_sorted(&v, 100.0), 30.0));
    }

    #[test]
    fn basic_stats() {
        let s = compute_stats(&[Some(10.0), Some(20.0), Some(30.0), None]);
        assert_eq!(s.total, 4);
        assert_eq!(s.received, 3);
        assert_eq!(s.lost, 1);
        assert!(close(s.loss_pct, 25.0));
        assert!(close(s.min, 10.0));
        assert!(close(s.max, 30.0));
        assert!(close(s.mean, 20.0));
        assert!(close(s.p50, 20.0));
        assert!(close(s.p95, 29.0));
        assert!(close(s.jitter, 10.0));
        assert!((s.stddev - 8.164965809).abs() < 1e-6);
    }

    #[test]
    fn empty_and_all_lost() {
        let s = compute_stats(&[]);
        assert_eq!(s.total, 0);
        assert!(!s.has_data());
        let s = compute_stats(&[None, None]);
        assert_eq!(s.lost, 2);
        assert!(close(s.loss_pct, 100.0));
        assert!(!s.has_data());
    }

    #[test]
    fn constant_series_has_zero_jitter() {
        let v: Vec<Option<f64>> = (0..50).map(|_| Some(15.0)).collect();
        let s = compute_stats(&v);
        assert!(close(s.jitter, 0.0));
        assert!(close(s.stddev, 0.0));
    }

    #[test]
    fn median_and_mad() {
        assert!(close(median(&[1.0, 3.0, 2.0]), 2.0));
        // valeurs 1,2,3,4,100 -> mediane 3 ; ecarts 2,1,0,1,97 -> mediane 1
        assert!(close(mad(&[1.0, 2.0, 3.0, 4.0, 100.0]), 1.0));
    }

    #[test]
    fn loss_burst_classification() {
        let s = vec![
            Some(1.0),
            None,
            Some(1.0),
            None,
            None,
            Some(1.0),
            None,
            None,
            None,
        ];
        let l = loss_bursts(&s);
        assert_eq!(l.isolated, 1);
        assert_eq!(l.bursts, 2);
        assert_eq!(l.longest, 3);
    }

    #[test]
    fn pearson_perfect_and_degenerate() {
        let r = pearson(&[1.0, 2.0, 3.0, 4.0], &[2.0, 4.0, 6.0, 8.0]).unwrap();
        assert!(close(r, 1.0));
        let r = pearson(&[1.0, 2.0, 3.0, 4.0], &[8.0, 6.0, 4.0, 2.0]).unwrap();
        assert!(close(r, -1.0));
        assert!(pearson(&[1.0, 1.0, 1.0], &[1.0, 2.0, 3.0]).is_none());
        assert!(pearson(&[1.0, 2.0], &[1.0, 2.0]).is_none());
    }
}
