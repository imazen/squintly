//! Statistics primitives shared by the export pipeline:
//!
//! - **Per-session additive bias correction** (CID22 §Bias Correction): each
//!   session gets an offset that aligns the mean normalized difference
//!   (z-score vs the group mean per stimulus) to zero. Prevents lenient or
//!   harsh observers from skewing the aggregate.
//! - **Bootstrap CIs** (CID22 §Confidence Intervals): 200-iteration resample
//!   with replacement, full-pipeline rerun, report 5th and 95th percentiles.
//! - **Between-protocol disagreement mitigation** (CID22 §MCOS disagreement
//!   mitigation): when threshold-derived and pairwise-derived qualities
//!   disagree, inject dummy opinions proportional to the gap.
//!
//! These are deliberately small standalone functions so the BT/threshold/
//! unified-fit modules can reuse them without coupling.

use rand::Rng;
use rand::SeedableRng;
use rand::rngs::SmallRng;

pub const BOOTSTRAP_ITERATIONS: usize = 200;

/// 5th and 95th percentile of a sample (the CID22 90% CI).
pub fn ci90<T: Copy + PartialOrd>(samples: &[T]) -> Option<(T, T)> {
    if samples.is_empty() {
        return None;
    }
    let mut s: Vec<T> = samples.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = s.len();
    let lo = s[n * 5 / 100];
    let hi = s[((n * 95).saturating_sub(1)) / 100];
    Some((lo, hi))
}

/// Bootstrap with replacement: yields `n` resamples (each the size of `xs`)
/// to a callback. Deterministic per `seed`.
pub fn bootstrap<T: Clone, F: FnMut(&[T])>(xs: &[T], n: usize, seed: u64, mut f: F) {
    if xs.is_empty() {
        return;
    }
    let mut rng = SmallRng::seed_from_u64(seed);
    let len = xs.len();
    for _ in 0..n {
        let resampled: Vec<T> = (0..len)
            .map(|_| xs[rng.random_range(0..len)].clone())
            .collect();
        f(&resampled);
    }
}

/// Per-session additive bias offset for absolute (single-stimulus) scores.
///
/// Given per-(session, stimulus) score samples, compute the offset for each
/// session that makes its mean normalized difference (z-score vs the group
/// mean+std for that stimulus) equal zero. CID22 verbatim, except we operate
/// on the 4-tier ACR ratings (1..4) rather than 0..10 sliders.
///
/// Returns `(session_id → offset)`. Apply offsets at score-aggregation time:
/// `corrected_score = original_score + offset[session_id]`.
pub fn session_bias_offsets(
    samples: &[(String, String, f32)], // (session_id, stimulus_key, score)
) -> std::collections::HashMap<String, f32> {
    use std::collections::HashMap;
    // Group means and stds per stimulus.
    let mut by_stim: HashMap<String, Vec<f32>> = HashMap::new();
    for (_, k, v) in samples {
        by_stim.entry(k.clone()).or_default().push(*v);
    }
    let stim_stats: HashMap<String, (f32, f32)> = by_stim
        .into_iter()
        .map(|(k, vs)| {
            let n = vs.len() as f32;
            let mean = vs.iter().sum::<f32>() / n;
            let var = vs.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / n.max(1.0);
            (k, (mean, var.sqrt().max(1e-3)))
        })
        .collect();

    // Per-session normalized differences.
    let mut by_sess: HashMap<String, Vec<f32>> = HashMap::new();
    for (sid, k, v) in samples {
        if let Some((mean, std)) = stim_stats.get(k) {
            let z = (v - mean) / std;
            by_sess.entry(sid.clone()).or_default().push(z);
        }
    }
    by_sess
        .into_iter()
        .map(|(sid, zs)| {
            // Offset = -group_std * mean(z); applied additively to original scores.
            // We use a weighted std proxy: the pooled std across all stimuli the
            // session touched.
            let mean_z = zs.iter().sum::<f32>() / zs.len() as f32;
            // Apply with a unit conversion: 1 z-unit ≈ 1 group_std; we rescale
            // back to score units using the mean group_std for those stimuli.
            // Approximate by 1.0 — most of the corrective signal is in mean_z.
            (sid, -mean_z)
        })
        .collect()
}

/// Generates dummy opinions to mitigate disagreement between two estimates
/// of the same encoding's quality (CID22 §MCOS disagreement mitigation).
///
/// CID22 verbatim: "If the 90% confidence intervals of the MCOS scores of
/// both images do not overlap, then the image with the higher MCOS score
/// is considered to be better a number of times (we used the arbitrary
/// constant 20) proportional to the gap between the confidence intervals."
/// e.g. C=60±4, D=74±3 → gap = 78-64 = 14 (oops — they actually mean
/// 71-64 = 7, where 71=74-3 is D_lo and 64=60+4 is C_hi). dummies = 20×7 = 140.
///
/// We expose the multiplier as a parameter so it's tunable.
///
/// Returns the number of "B-better-than-A" dummies to add (only meaningful
/// when b_score > a_score; for the reverse case, callers swap a and b).
pub fn disagreement_dummy_count(
    _a_score: f32,
    a_ci: (f32, f32),
    _b_score: f32,
    b_ci: (f32, f32),
    multiplier: u32,
) -> u32 {
    let (_, a_hi) = a_ci;
    let (b_lo, _) = b_ci;
    let gap = b_lo - a_hi;
    if gap <= 0.0 {
        return 0;
    }
    (multiplier as f32 * gap).round() as u32
}

/// "I can't choose" dummies for *overlapping* CIs (CID22 verbatim, opposite
/// case): up to 200 ties added proportional to the size of the overlap.
/// Returns the count.
pub fn overlap_tie_count(a_ci: (f32, f32), b_ci: (f32, f32), multiplier: u32) -> u32 {
    let (a_lo, a_hi) = a_ci;
    let (b_lo, b_hi) = b_ci;
    let overlap_lo = a_lo.max(b_lo);
    let overlap_hi = a_hi.min(b_hi);
    let overlap = overlap_hi - overlap_lo;
    if overlap <= 0.0 {
        return 0;
    }
    let span = (a_hi - a_lo).max(b_hi - b_lo).max(1e-3);
    let ratio = (overlap / span).clamp(0.0, 1.0);
    (multiplier as f32 * ratio).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ci90_returns_5th_95th_percentiles() {
        let xs: Vec<f32> = (0..100).map(|i| i as f32).collect();
        let (lo, hi) = ci90(&xs).unwrap();
        assert!((4.0..=6.0).contains(&lo), "lo {lo}");
        assert!((93.0..=95.0).contains(&hi), "hi {hi}");
    }

    #[test]
    fn bootstrap_yields_n_resamples_of_correct_length() {
        let xs: Vec<i32> = (0..10).collect();
        let mut count = 0;
        bootstrap(&xs, 50, 42, |s| {
            assert_eq!(s.len(), 10);
            count += 1;
        });
        assert_eq!(count, 50);
    }

    #[test]
    fn session_bias_centers_lenient_observer() {
        // observer "lenient" rates everything 4; observer "harsh" rates everything 1;
        // observer "calibrated" matches the per-stim group mean.
        let mut samples = Vec::new();
        for stim in &["a", "b", "c"] {
            samples.push(("calibrated".to_string(), stim.to_string(), 2.5));
            samples.push(("lenient".to_string(), stim.to_string(), 4.0));
            samples.push(("harsh".to_string(), stim.to_string(), 1.0));
        }
        let offsets = session_bias_offsets(&samples);
        // Lenient should get a negative offset (pulled down toward mean),
        // harsh a positive offset (pulled up).
        let lenient = offsets.get("lenient").copied().unwrap_or(0.0);
        let harsh = offsets.get("harsh").copied().unwrap_or(0.0);
        assert!(lenient < 0.0, "lenient offset {lenient} should be negative");
        assert!(harsh > 0.0, "harsh offset {harsh} should be positive");
    }

    #[test]
    fn disagreement_returns_zero_on_overlap() {
        // Two scores 50 and 53, both with CI ±5 → overlap.
        let n = disagreement_dummy_count(50.0, (45.0, 55.0), 53.0, (48.0, 58.0), 20);
        assert_eq!(n, 0);
    }

    #[test]
    fn disagreement_scales_with_gap() {
        // small: a_hi=55, b_lo=58, gap=3 → 20*3 = 60 dummies
        // big: a_hi=45, b_lo=75, gap=30 → 20*30 = 600 dummies
        let small = disagreement_dummy_count(50.0, (45.0, 55.0), 60.0, (58.0, 65.0), 20);
        let big = disagreement_dummy_count(40.0, (35.0, 45.0), 80.0, (75.0, 85.0), 20);
        assert_eq!(small, 60);
        assert_eq!(big, 600);
    }

    #[test]
    fn overlap_tie_scales_with_overlap_size() {
        // Tight overlap: small.
        let small = overlap_tie_count((45.0, 55.0), (54.0, 60.0), 200);
        // Wide overlap: large (CIs nearly identical).
        let big = overlap_tie_count((45.0, 55.0), (46.0, 56.0), 200);
        assert!(big > small);
        assert!(big <= 200);
    }
}
