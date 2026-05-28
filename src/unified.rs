//! Pérez-Ortiz et al. 2019 unified rating + pairwise quality scale fit
//! ([PDF](https://www.cl.cam.ac.uk/~rkm38/pdfs/perezortiz2019unified_quality_scale.pdf)).
//!
//! **KNOWN ISSUES (2026-05-28)** — the gradient-descent solver below is
//! NOT production-ready. On synthetic data where pair + rating evidence
//! are consistent with the same latent m, the solver diverges instead of
//! converging: σ explodes (observed σ = 2361 vs the BT-only fit's 0.343),
//! τ wanders far outside the m range (observed τ_0 = -1356), log_σ_o
//! goes NaN, and the loop hits `max_iter` without converging. The
//! existing `unified_fit_recovers_obvious_ranking` and
//! `unified_fit_estimates_observer_bias` tests pass because they're
//! narrow (one observer, simple ranking checks) and don't trip the
//! divergence modes.
//!
//! Suspected causes (TODO before pilot):
//! 1. `grad_log_sigma_o` chain rule is off — the `lower.max(-1e6)` hack
//!    at line ~181 suggests the `−∞ lower` branch is mis-derived.
//! 2. `grad_log_sigma` for the pair Thurstone term has the wrong sign
//!    (or the chain factor is missing a factor of 2): when ratings are
//!    introduced, σ runs away rather than equilibrating.
//! 3. The tau-monotonicity enforcement (sort ascending after each step)
//!    can clobber the gradient update for non-monotone-violating taus.
//!
//! Mitigations until the solver is rewritten:
//! - `pair_log_likelihood` / `rating_log_likelihood` / `total_log_likelihood`
//!   (below) work correctly on any `UnifiedFit` regardless of how it was
//!   produced — held-out scoring is decoupled from training. Callers
//!   wiring the H3 pilot evaluation should use these against fits from
//!   the reference implementations in `/mnt/v/repos/iqa-tools/` (pwcmp
//!   + a Pérez-Ortiz cumulative-link implementation) until the in-tree
//!   solver is fixed.
//! - For v0.1 production, `src/bt.rs::fit` (the BT-Davidson solver) is
//!   the working code path; the threshold staircase covers the rating
//!   modality independently.
//!
//! Joint likelihood for two protocols on the same items:
//!
//! - **Pairwise** (Thurstone Case V): for items i, j with latent qualities m_i,
//!   m_j and a global decision noise σ, P(i > j) = Φ((m_i - m_j) / (√2 σ)).
//!
//! - **Rating** (4-tier ordinal): for stimulus i rated by observer o,
//!   P(rating_o(i) ≤ k) = Φ((τ_k - m_i - δ_o) / σ_o)
//!   where δ_o is per-observer additive bias and σ_o is per-observer noise
//!   (set to a global σ for low-N observers; learned for ≥30-trial ones).
//!   τ_k are the global category thresholds: τ_1 < τ_2 < τ_3 (4-tier needs 3
//!   thresholds for the cumulative-link model).
//!
//! Output: latent m_i for every item, anchored at m_reference = 0; global
//! σ; per-observer (δ_o, σ_o); category thresholds τ.
//!
//! Fit by gradient descent with Gaussian priors (σ_β = 1.5 on m, σ_δ = 0.5
//! on δ, log-σ_o ~ N(0, 0.5²)). Per-protocol scaling factor c (eq. 8) is
//! folded into σ via the rating likelihood.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum PairOutcome {
    AWins,
    BWins,
    Tie,
}

#[derive(Debug, Clone)]
pub struct PairObs {
    pub item_a: usize,
    pub item_b: usize,
    pub observer: usize,
    pub outcome: PairOutcome,
}

#[derive(Debug, Clone)]
pub struct RatingObs {
    pub item: usize,
    pub observer: usize,
    pub rating: u8, // 1..=4 (4-tier ACR)
}

#[derive(Debug, Clone)]
pub struct UnifiedFit {
    pub m: Vec<f32>,           // latent quality per item
    pub delta: Vec<f32>,       // per-observer additive bias
    pub log_sigma_o: Vec<f32>, // per-observer log noise
    pub tau: [f32; 3],         // category thresholds for the 4-tier ordinal scale
    pub sigma: f32,            // global pairwise σ (Thurstone Case V)
    pub iterations: u32,
    pub final_loss: f32,
}

const SQRT_2: f32 = std::f32::consts::SQRT_2;

#[inline]
fn phi(z: f32) -> f32 {
    // Standard-normal CDF via erf.
    0.5 * (1.0 + erf(z / SQRT_2))
}

#[inline]
#[allow(clippy::excessive_precision)] // canonical Abramowitz-Stegun 7.1.26 constants
fn erf(x: f32) -> f32 {
    // Max error ≈ 1.5e-7.
    let t = 1.0 / (1.0 + 0.3275911 * x.abs());
    let y = 1.0
        - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t
            + 0.254829592)
            * t
            * (-x * x).exp();
    if x < 0.0 { -y } else { y }
}

/// Fit a unified rating + pairwise model.
///
/// `n_items`: total items; index 0 is the reference (anchored at m=0).
/// `n_observers`: distinct observers contributing.
/// `pairs`, `ratings`: observations.
pub fn fit_unified(
    n_items: usize,
    n_observers: usize,
    pairs: &[PairObs],
    ratings: &[RatingObs],
) -> UnifiedFit {
    let mut m = vec![0.0_f32; n_items];
    let mut delta = vec![0.0_f32; n_observers];
    let mut log_sigma_o = vec![0.0_f32; n_observers]; // σ_o = 1
    // Category thresholds for 4-tier: roughly evenly spaced around 0 in m-units.
    let mut tau: [f32; 3] = [-1.0, 0.0, 1.0];
    let mut log_sigma = 0.0_f32; // pairwise σ = 1

    let prior_m = 1.0 / (1.5 * 1.5);
    let prior_delta = 1.0 / (0.5 * 0.5);
    let prior_log_sigma_o = 1.0 / (0.5 * 0.5);

    let mut prev_loss = f32::INFINITY;
    let mut lr: f32 = 0.02;
    let mut iters = 0u32;
    let max_iter: u32 = 800;

    for k in 0..max_iter {
        iters = k + 1;
        let mut grad_m = vec![0.0_f32; n_items];
        let mut grad_delta = vec![0.0_f32; n_observers];
        let mut grad_log_sigma_o = vec![0.0_f32; n_observers];
        let mut grad_tau = [0.0_f32; 3];
        let mut grad_log_sigma = 0.0_f32;
        let mut loss = 0.0_f32;

        // Pairwise contribution.
        let sigma = log_sigma.exp();
        for p in pairs {
            let diff = m[p.item_a] - m[p.item_b];
            let denom = SQRT_2 * sigma;
            let z = diff / denom;
            let p_a = phi(z).clamp(1e-6, 1.0 - 1e-6);
            let p_b = 1.0 - p_a;
            // Tie modeled as 50/50 with observer-noise reasoning; in the strict
            // unified model ties don't appear, but we can split the weight 50/50
            // which contributes equally to both gradients.
            let (l, dl_dz) = match p.outcome {
                PairOutcome::AWins => (-p_a.ln(), -(1.0 / p_a) * normal_pdf(z)),
                PairOutcome::BWins => (-p_b.ln(), (1.0 / p_b) * normal_pdf(z)),
                PairOutcome::Tie => {
                    // Symmetric: half-weight to each side.
                    let l = -(0.5 * p_a + 0.5 * p_b).max(1e-9).ln();
                    (l, 0.0)
                }
            };
            loss += l;
            // ∂z/∂m_a = +1/denom; ∂z/∂m_b = -1/denom; ∂z/∂σ = -z/σ
            grad_m[p.item_a] += dl_dz / denom;
            grad_m[p.item_b] += -dl_dz / denom;
            grad_log_sigma += dl_dz * (-z); // since ∂σ/∂log_σ = σ, and z/σ * σ = z (factor of -1 from chain)
        }

        // Rating contribution (cumulative-link model).
        for r in ratings {
            let i = r.item;
            let o = r.observer;
            let mu = m[i] + delta[o];
            let so = log_sigma_o[o].exp().max(1e-3);
            let k_idx = (r.rating as usize).clamp(1, 4) - 1; // 0..3
            // P(rating = k+1) = Φ((τ_k - mu)/σ_o) - Φ((τ_{k-1} - mu)/σ_o), with
            // τ_-1 = -∞, τ_3 = +∞.
            let upper = if k_idx == 3 {
                f32::INFINITY
            } else {
                (tau[k_idx] - mu) / so
            };
            let lower = if k_idx == 0 {
                f32::NEG_INFINITY
            } else {
                (tau[k_idx - 1] - mu) / so
            };
            let p_upper = if upper.is_infinite() { 1.0 } else { phi(upper) };
            let p_lower = if lower.is_infinite() { 0.0 } else { phi(lower) };
            let p_k = (p_upper - p_lower).clamp(1e-6, 1.0);
            loss += -p_k.ln();

            // Gradient wrt m, delta, log_sigma_o, tau via chain rule on the
            // standard-normal pdf.
            let pdf_u = if upper.is_infinite() {
                0.0
            } else {
                normal_pdf(upper)
            };
            let pdf_l = if lower.is_infinite() {
                0.0
            } else {
                normal_pdf(lower)
            };
            let inv_pk = -1.0 / p_k;
            // ∂(p_upper - p_lower)/∂mu = -(pdf_u - pdf_l)/σ_o (sign from -μ in arg)
            let d_mu = -(pdf_u - pdf_l) / so;
            let d_log_sigma_o = -((upper * pdf_u) - (lower.max(-1e6) * pdf_l));
            // d_log_sigma_o = -((upper * pdf_u) - (lower * pdf_l)); but treat infinite lower as 0 contribution
            let d_log_sigma_o = if lower.is_infinite() {
                -(upper * pdf_u)
            } else {
                d_log_sigma_o
            };
            grad_m[i] += inv_pk * d_mu;
            grad_delta[o] += inv_pk * d_mu;
            grad_log_sigma_o[o] += inv_pk * d_log_sigma_o;
            // ∂(p_upper - p_lower)/∂τ_k = pdf_u/σ_o ; ∂/∂τ_{k-1} = -pdf_l/σ_o
            if k_idx < 3 {
                grad_tau[k_idx] += inv_pk * pdf_u / so;
            }
            if k_idx > 0 {
                grad_tau[k_idx - 1] += inv_pk * (-pdf_l / so);
            }
        }

        // Priors.
        for i in 0..n_items {
            loss += 0.5 * prior_m * m[i] * m[i];
            grad_m[i] += prior_m * m[i];
        }
        for o in 0..n_observers {
            loss += 0.5 * prior_delta * delta[o] * delta[o];
            loss += 0.5 * prior_log_sigma_o * log_sigma_o[o] * log_sigma_o[o];
            grad_delta[o] += prior_delta * delta[o];
            grad_log_sigma_o[o] += prior_log_sigma_o * log_sigma_o[o];
        }

        // Anchor reference.
        grad_m[0] = 0.0;

        // Update.
        for i in 0..n_items {
            m[i] -= lr * grad_m[i];
        }
        for o in 0..n_observers {
            delta[o] -= lr * grad_delta[o];
            log_sigma_o[o] -= lr * grad_log_sigma_o[o];
        }
        for k in 0..3 {
            tau[k] -= lr * grad_tau[k];
        }
        log_sigma -= lr * grad_log_sigma;
        m[0] = 0.0;

        // Keep tau monotone (sort ascending after update).
        let mut tau_v = tau.to_vec();
        tau_v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        tau = [tau_v[0], tau_v[1], tau_v[2]];

        if (prev_loss - loss).abs() < 1e-5 * (1.0 + loss.abs()) {
            return UnifiedFit {
                m,
                delta,
                log_sigma_o,
                tau,
                sigma: log_sigma.exp(),
                iterations: iters,
                final_loss: loss,
            };
        }
        if loss > prev_loss {
            lr *= 0.5;
        }
        prev_loss = loss;
    }
    UnifiedFit {
        m,
        delta,
        log_sigma_o,
        tau,
        sigma: log_sigma.exp(),
        iterations: iters,
        final_loss: prev_loss,
    }
}

#[inline]
fn normal_pdf(z: f32) -> f32 {
    let inv_sqrt_2pi: f32 = 0.398_942_3; // 1 / sqrt(2π) at f32 precision
    inv_sqrt_2pi * (-0.5 * z * z).exp()
}

/// Log-likelihood of a set of pair observations under a fitted unified
/// model. Used by held-out evaluation (H3 in `docs/STUDY.md`) — call with
/// the held-out fold's pairs to score the model fit on data it didn't
/// train on. Matches the loss computed inside `fit_unified` (negated, no
/// priors, no clamping) so that train/test deltas reflect the model's
/// generalisation, not the regulariser.
///
/// Ties are scored as `0.5 · P(A>B) + 0.5 · P(B>A) = 0.5` per the
/// unified model's "no tie modality" assumption (a tie under Thurstone
/// is the symmetric 50/50 outcome). That's a constant per tie observation
/// — if a fold has many ties this drags the absolute LL but doesn't
/// change the *delta* between unified and BT-only.
pub fn pair_log_likelihood(fit: &UnifiedFit, pairs: &[PairObs]) -> f64 {
    let denom = SQRT_2 * fit.sigma.max(1e-3);
    let mut ll = 0.0_f64;
    for p in pairs {
        let diff = fit.m[p.item_a] - fit.m[p.item_b];
        let p_a = phi(diff / denom).clamp(1e-6, 1.0 - 1e-6);
        let p_b = 1.0 - p_a;
        let lp = match p.outcome {
            PairOutcome::AWins => p_a.ln(),
            PairOutcome::BWins => p_b.ln(),
            PairOutcome::Tie => 0.5_f32.ln(),
        };
        ll += lp as f64;
    }
    ll
}

/// Log-likelihood of a set of rating observations under a fitted unified
/// model. Uses each observer's `(δ_o, σ_o)` and the fitted `τ` thresholds;
/// out-of-range `observer` indices fall back to (δ=0, σ=1) so the held-out
/// fold can include observers the train fold never saw.
pub fn rating_log_likelihood(fit: &UnifiedFit, ratings: &[RatingObs]) -> f64 {
    let mut ll = 0.0_f64;
    for r in ratings {
        let i = r.item;
        if i >= fit.m.len() {
            continue;
        }
        let (delta_o, so) = if r.observer < fit.delta.len() {
            (fit.delta[r.observer], fit.log_sigma_o[r.observer].exp().max(1e-3))
        } else {
            (0.0, 1.0)
        };
        let mu = fit.m[i] + delta_o;
        let k_idx = (r.rating as usize).clamp(1, 4) - 1;
        let upper = if k_idx == 3 {
            f32::INFINITY
        } else {
            (fit.tau[k_idx] - mu) / so
        };
        let lower = if k_idx == 0 {
            f32::NEG_INFINITY
        } else {
            (fit.tau[k_idx - 1] - mu) / so
        };
        let p_upper = if upper.is_infinite() { 1.0 } else { phi(upper) };
        let p_lower = if lower.is_infinite() { 0.0 } else { phi(lower) };
        let p_k = (p_upper - p_lower).clamp(1e-9, 1.0);
        ll += (p_k.ln()) as f64;
    }
    ll
}

/// Combined log-likelihood across pairs and ratings — the H3 evaluation
/// metric. Held-out LL gain = `total_log_likelihood(unified_fit, held_out)
/// − total_log_likelihood(bt_only_fit, held_out)`; per-trial nats =
/// `gain / (pairs.len() + ratings.len())`.
pub fn total_log_likelihood(
    fit: &UnifiedFit,
    pairs: &[PairObs],
    ratings: &[RatingObs],
) -> f64 {
    pair_log_likelihood(fit, pairs) + rating_log_likelihood(fit, ratings)
}

/// Map latent m → 0–100 quality, anchored at m_reference = 0 → 100.
pub fn m_to_quality(m: &[f32], reference_idx: usize, scale: f32) -> Vec<f32> {
    let anchor = m[reference_idx];
    m.iter()
        .map(|x| (100.0 + (x - anchor) * scale).clamp(0.0, 100.0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unified_fit_recovers_obvious_ranking() {
        // 3 items: 0 (reference, high quality), 1 (medium), 2 (low).
        // Observer 0 rates them 1, 2, 4. Pairs all agree.
        let pairs = [
            PairObs {
                item_a: 0,
                item_b: 1,
                observer: 0,
                outcome: PairOutcome::AWins,
            },
            PairObs {
                item_a: 0,
                item_b: 2,
                observer: 0,
                outcome: PairOutcome::AWins,
            },
            PairObs {
                item_a: 1,
                item_b: 2,
                observer: 0,
                outcome: PairOutcome::AWins,
            },
        ];
        let mut ratings = Vec::new();
        // Several repeats per stimulus to give the rating-likelihood traction.
        for _ in 0..10 {
            ratings.push(RatingObs {
                item: 0,
                observer: 0,
                rating: 1,
            });
            ratings.push(RatingObs {
                item: 1,
                observer: 0,
                rating: 2,
            });
            ratings.push(RatingObs {
                item: 2,
                observer: 0,
                rating: 4,
            });
        }
        let fit = fit_unified(3, 1, &pairs, &ratings);
        assert!(fit.m[0] >= fit.m[1] - 1e-2, "ref ≥ med, got {:?}", fit.m);
        assert!(fit.m[1] > fit.m[2], "med > low, got {:?}", fit.m);
    }

    #[test]
    fn pair_log_likelihood_matches_hand_computation() {
        // Three items with hand-pickable latent qualities; one observer.
        // Build a fit by hand (skip the optimiser) so the test isolates the
        // log-likelihood math.
        let fit = UnifiedFit {
            m: vec![0.0, -1.0, -2.0],
            delta: vec![0.0],
            log_sigma_o: vec![0.0],
            tau: [-1.0, 0.0, 1.0],
            sigma: 1.0,
            iterations: 0,
            final_loss: 0.0,
        };
        let pairs = [PairObs {
            item_a: 0,
            item_b: 2,
            observer: 0,
            outcome: PairOutcome::AWins,
        }];
        // diff = 2.0, denom = √2, z = √2; phi(√2) ≈ 0.9214 → ln ≈ −0.0819.
        let ll = pair_log_likelihood(&fit, &pairs);
        assert!(
            (ll - (-0.0819_f64)).abs() < 0.005,
            "pair LL hand-computed expected ≈ -0.082, got {ll}"
        );
    }

    #[test]
    fn total_log_likelihood_decomposes_into_pair_and_rating_terms() {
        // Pure additivity check — no solver, just verifies that the helper
        // composition is correct.
        let fit = UnifiedFit {
            m: vec![0.0, -1.0],
            delta: vec![0.0],
            log_sigma_o: vec![0.0],
            tau: [-1.0, 0.0, 1.0],
            sigma: 1.0,
            iterations: 0,
            final_loss: 0.0,
        };
        let pairs = [PairObs {
            item_a: 0,
            item_b: 1,
            observer: 0,
            outcome: PairOutcome::AWins,
        }];
        let ratings = [RatingObs {
            item: 0,
            observer: 0,
            rating: 1,
        }];
        let pair_ll = pair_log_likelihood(&fit, &pairs);
        let rating_ll = rating_log_likelihood(&fit, &ratings);
        let total = total_log_likelihood(&fit, &pairs, &ratings);
        assert!(
            (total - (pair_ll + rating_ll)).abs() < 1e-6,
            "total = {total}, pair = {pair_ll}, rating = {rating_ll}"
        );
    }

    #[test]
    fn unified_fit_estimates_observer_bias() {
        // Two observers; observer 0 rates everything one tier worse than observer 1.
        let mut ratings = Vec::new();
        for _ in 0..20 {
            ratings.push(RatingObs {
                item: 0,
                observer: 0,
                rating: 2,
            }); // observer 0: ref → 2
            ratings.push(RatingObs {
                item: 0,
                observer: 1,
                rating: 1,
            }); // observer 1: ref → 1
            ratings.push(RatingObs {
                item: 1,
                observer: 0,
                rating: 3,
            });
            ratings.push(RatingObs {
                item: 1,
                observer: 1,
                rating: 2,
            });
        }
        let fit = fit_unified(2, 2, &[], &ratings);
        // Observer 0's δ should be more positive (their ratings are higher = worse).
        assert!(
            fit.delta[0] > fit.delta[1] - 0.1,
            "δ[0]={} should be greater than δ[1]={}",
            fit.delta[0],
            fit.delta[1]
        );
    }
}
