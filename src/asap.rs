//! ASAP — Active Sampling for Pairwise comparisons (Mikhailiuk 2020).
//!
//! For a fixed source, given the current set of pair observations, choose the
//! next pair (a, b) to maximize the expected information gain (EIG) about
//! the latent quality vector β. EIG ≈ entropy of the binary outcome under
//! the current belief: H(p) = -p log p - (1-p) log (1-p), maximized at
//! p = 0.5 (least-decided pair). With the current MAP estimate, p =
//! Φ((β_a - β_b) / (√2 σ)).
//!
//! Compared to random sampling, ASAP empirically needs 30-50% fewer
//! comparisons for equivalent CI width on the BT scale. We implement the
//! simple Gaussian-Laplace approximation; the message-passing variant in
//! the paper gives only marginal improvement at much higher implementation
//! cost.

use rand::Rng;

const SQRT_2: f32 = std::f32::consts::SQRT_2;

#[inline]
#[allow(clippy::excessive_precision)] // canonical Abramowitz-Stegun 7.1.26 constants
fn phi(z: f32) -> f32 {
    let t = 1.0 / (1.0 + 0.3275911 * (z / SQRT_2).abs());
    let y = 1.0
        - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t
            + 0.254829592)
            * t
            * (-(z / SQRT_2).powi(2)).exp();
    let erf = if z >= 0.0 { y } else { -y };
    0.5 * (1.0 + erf)
}

#[inline]
fn binary_entropy(p: f32) -> f32 {
    let p = p.clamp(1e-6, 1.0 - 1e-6);
    -(p * p.ln() + (1.0 - p) * (1.0 - p).ln())
}

/// Expected information gain of comparing items `a` and `b` under current
/// belief β with global decision noise σ. We compute the binary-outcome
/// entropy at the MAP estimate; the full EIG would integrate over the
/// posterior, but for tightly-fit β the difference is small.
pub fn eig(beta: &[f32], sigma: f32, a: usize, b: usize) -> f32 {
    let z = (beta[a] - beta[b]) / (SQRT_2 * sigma.max(1e-3));
    binary_entropy(phi(z))
}

/// Pick the pair (a, b) maximizing EIG among `candidates`. Ties broken by
/// random choice (`rng`) to avoid systematic drift toward low-index items.
pub fn pick_max_eig<R: Rng + ?Sized>(
    beta: &[f32],
    sigma: f32,
    candidates: &[(usize, usize)],
    rng: &mut R,
) -> Option<(usize, usize)> {
    if candidates.is_empty() {
        return None;
    }
    let mut best = -f32::INFINITY;
    let mut best_pairs: Vec<(usize, usize)> = Vec::new();
    for &(a, b) in candidates {
        let g = eig(beta, sigma, a, b);
        if (g - best).abs() < 1e-6 {
            best_pairs.push((a, b));
        } else if g > best {
            best = g;
            best_pairs.clear();
            best_pairs.push((a, b));
        }
    }
    if best_pairs.is_empty() {
        return None;
    }
    Some(best_pairs[rng.random_range(0..best_pairs.len())])
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::SmallRng;

    #[test]
    fn eig_peaks_when_pair_is_indistinguishable() {
        // Equal items → EIG ≈ ln 2 (max for binary entropy).
        let g = eig(&[0.0, 0.0], 1.0, 0, 1);
        assert!((g - 2f32.ln()).abs() < 1e-3);
    }

    #[test]
    fn eig_drops_for_obvious_pairs() {
        // β_a >> β_b → outcome is near-certain → low EIG.
        let g = eig(&[3.0, -3.0], 1.0, 0, 1);
        assert!(g < 0.05, "got {g}");
    }

    #[test]
    fn pick_max_eig_chooses_least_decided_pair() {
        let beta = vec![5.0, 4.9, -5.0];
        // (0,1) is contested, (0,2) and (1,2) are obvious.
        let cands = [(0, 1), (0, 2), (1, 2)];
        let mut r = SmallRng::seed_from_u64(0);
        let picked = pick_max_eig(&beta, 1.0, &cands, &mut r).unwrap();
        assert_eq!(picked, (0, 1));
    }
}
