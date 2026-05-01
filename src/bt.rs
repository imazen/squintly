//! Bradley–Terry-with-ties (Davidson 1970).
//!
//! For a fixed source, fit log-strengths β_i and a tie parameter η = log(ν) by
//! gradient descent on the negative log-likelihood with a Gaussian prior on β.
//! Anchored at β_reference = 0.
//!
//! Probabilities, with mid = exp((β_a + β_b)/2) and z = exp(β_a) + exp(β_b) + ν·mid:
//!     P(a > b) = exp(β_a) / z
//!     P(a ~ b) = ν·mid / z
//!     P(a < b) = exp(β_b) / z
//!
//! Gradients (per-comparison contribution to L = −log P(outcome)):
//!     ∂L/∂β_a, ∂L/∂β_b, ∂L/∂η — see `derive_grad` below.
//!
//! Step decay on loss-up; converges in tens to low-hundreds of iterations for the
//! few-thousand-comparisons-per-source regime we expect. v0.2 can swap in L-BFGS.

#[derive(Debug, Clone, Copy)]
pub enum Outcome {
    AWins,
    BWins,
    Tie,
}

#[derive(Debug, Clone)]
pub struct Comparison {
    pub a: usize,
    pub b: usize,
    pub outcome: Outcome,
}

pub struct Fit {
    pub beta: Vec<f32>,
    pub eta: f32,
    pub iterations: u32,
    pub final_loss: f32,
}

#[inline]
fn derive_grad(bi: f32, bj: f32, eta: f32, outcome: Outcome) -> (f32, f32, f32, f32) {
    let ei = bi.exp();
    let ej = bj.exp();
    let nu = eta.exp();
    let mid = ((bi + bj) * 0.5).exp();
    let z = ei + ej + nu * mid;
    let p_i = ei / z;
    let p_j = ej / z;
    let p_t = (nu * mid) / z;

    let (loss_term, dl_dba, dl_dbb, dl_deta) = match outcome {
        Outcome::AWins => {
            let l = -p_i.max(1e-9).ln();
            (l, -1.0 + p_i + 0.5 * p_t, p_j + 0.5 * p_t, p_t)
        }
        Outcome::BWins => {
            let l = -p_j.max(1e-9).ln();
            (l, p_i + 0.5 * p_t, -1.0 + p_j + 0.5 * p_t, p_t)
        }
        Outcome::Tie => {
            let l = -p_t.max(1e-9).ln();
            (
                l,
                -0.5 + p_i + 0.5 * p_t,
                -0.5 + p_j + 0.5 * p_t,
                -1.0 + p_t,
            )
        }
    };
    (loss_term, dl_dba, dl_dbb, dl_deta)
}

/// Inject CID22-style monotonicity dummy opinions: for each ordered pair
/// `(low, high)` in `monotone_pairs`, append `n_dummy` synthetic comparisons
/// where `high` beats `low`. Per CID22 §Monotonicity (Sneyers et al. 2023),
/// this is the single highest-leverage rigor lever — KRCC dropped 0.99 → 0.56
/// in their dataset when omitted.
pub fn with_monotonicity(
    base: &[Comparison],
    monotone_pairs: &[(usize, usize)],
    n_dummy: u32,
) -> Vec<Comparison> {
    let mut out = base.to_vec();
    for &(lo, hi) in monotone_pairs {
        for _ in 0..n_dummy {
            out.push(Comparison {
                a: hi,
                b: lo,
                outcome: Outcome::AWins,
            });
        }
    }
    out
}

pub fn fit(n_items: usize, comparisons: &[Comparison], anchor: usize, prior_sigma: f32) -> Fit {
    let mut beta = vec![0.0f32; n_items];
    let mut eta = 0.0f32;
    let prior_inv_var = 1.0 / (prior_sigma * prior_sigma);
    let max_iter: u32 = 600;

    let mut prev_loss = f32::INFINITY;
    let mut lr: f32 = 0.05;
    let mut iter = 0u32;
    for k in 0..max_iter {
        iter = k + 1;
        let mut grad_beta = vec![0.0f32; n_items];
        let mut grad_eta = 0.0f32;
        let mut loss = 0.0f32;

        for c in comparisons {
            let (l, ga, gb, ge) = derive_grad(beta[c.a], beta[c.b], eta, c.outcome);
            loss += l;
            grad_beta[c.a] += ga;
            grad_beta[c.b] += gb;
            grad_eta += ge;
        }
        for i in 0..n_items {
            loss += 0.5 * prior_inv_var * beta[i] * beta[i];
            grad_beta[i] += prior_inv_var * beta[i];
        }
        grad_beta[anchor] = 0.0;

        for i in 0..n_items {
            beta[i] -= lr * grad_beta[i];
        }
        eta -= lr * grad_eta;
        beta[anchor] = 0.0;

        if (prev_loss - loss).abs() < 1e-5 * (1.0 + loss.abs()) {
            return Fit {
                beta,
                eta,
                iterations: iter,
                final_loss: loss,
            };
        }
        if loss > prev_loss {
            lr *= 0.5;
        }
        prev_loss = loss;
    }
    Fit {
        beta,
        eta,
        iterations: iter,
        final_loss: prev_loss,
    }
}

/// Map β to a 0–100 quality score, anchored so reference → 100.
/// 10 β-units ≈ 100 quality points; downstream zentrain handles final calibration.
pub fn beta_to_quality(beta: &[f32], reference_idx: usize) -> Vec<f32> {
    let anchor = beta[reference_idx];
    beta.iter()
        .map(|b| (100.0 + (b - anchor) * 10.0).clamp(0.0, 100.0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_recovers_obvious_ranking() {
        let comps: Vec<Comparison> = (0..40)
            .flat_map(|_| {
                vec![
                    Comparison {
                        a: 0,
                        b: 1,
                        outcome: Outcome::AWins,
                    },
                    Comparison {
                        a: 1,
                        b: 2,
                        outcome: Outcome::AWins,
                    },
                    Comparison {
                        a: 0,
                        b: 2,
                        outcome: Outcome::AWins,
                    },
                ]
            })
            .collect();
        let fit = fit(3, &comps, 0, 2.0);
        assert!(fit.beta[0] >= fit.beta[1] - 1e-3);
        assert!(fit.beta[1] > fit.beta[2]);
    }

    #[test]
    fn monotonicity_dummies_pin_ordering_against_noise() {
        // Two same-codec items 0 (low q) and 1 (high q). Real opinions are
        // noisy/contradictory: 5 say low wins, 5 say high wins, 5 ties.
        let mut comps: Vec<Comparison> = Vec::new();
        for _ in 0..5 {
            comps.push(Comparison {
                a: 0,
                b: 1,
                outcome: Outcome::AWins,
            });
            comps.push(Comparison {
                a: 0,
                b: 1,
                outcome: Outcome::BWins,
            });
            comps.push(Comparison {
                a: 0,
                b: 1,
                outcome: Outcome::Tie,
            });
        }
        let raw = fit(2, &comps, 0, 1.5);
        // Without monotonicity, fit is ~symmetric.
        assert!((raw.beta[0] - raw.beta[1]).abs() < 0.5);

        // With CID22-style 200 dummies pinning hi > lo:
        let with_mono = with_monotonicity(&comps, &[(0, 1)], 200);
        let pinned = fit(2, &with_mono, 0, 1.5);
        assert!(
            pinned.beta[1] > pinned.beta[0] + 1.0,
            "monotonicity should make β[1]≫β[0]; got {:?}",
            pinned.beta
        );
    }

    #[test]
    fn ties_pull_eta_up() {
        // Many ties between equal items should push ν upward.
        let comps: Vec<Comparison> = (0..50)
            .map(|_| Comparison {
                a: 0,
                b: 1,
                outcome: Outcome::Tie,
            })
            .collect();
        let fit = fit(2, &comps, 0, 5.0);
        assert!(fit.eta > 0.5, "got eta {}", fit.eta);
    }
}
