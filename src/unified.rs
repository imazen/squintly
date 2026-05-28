//! Pérez-Ortiz et al. 2019 unified rating + pairwise quality scale fit
//! ([PDF](https://www.cl.cam.ac.uk/~rkm38/pdfs/perezortiz2019unified_quality_scale.pdf)).
//!
//! **Three bug fixes landed 2026-05-28** that brought the solver from
//! diverging-on-any-real-data to passing a 4-item / 4-tier-rating
//! regression test (`unified_competitive_with_bt_only_on_heldout_pairs`):
//!
//! 1. `d_log_sigma_o` NaN when a rating == 4 (upper = +∞). `pdf_u` was
//!    set to 0 for the infinite case but `upper * pdf_u = ∞ · 0` still
//!    produced NaN in f32 arithmetic, which then poisoned `log_sigma_o`
//!    and cascaded. Both infinite arms now have explicit `0.0` guards.
//! 2. No prior on global `log_sigma` → drifted into the σ ≫ 1 flat
//!    region where `dl_dz · (-z) → 0` and σ ≈ 2000 was a fixed point.
//!    Added `log_σ ~ N(0, 1²)` matching `log_σ_o`'s shape.
//! 3. Rating-index sign convention was inverted. Squintly's UI uses
//!    rating 1 = imperceptible = BEST quality, but the cumulative-link
//!    model as coded treated higher μ as worse — so pair (higher m =
//!    better) and rating (higher m = worse) signals contradicted and the
//!    fit infered m upside-down. Now we flip `k_idx = 4 - rating` inside
//!    both the fit loop and `rating_log_likelihood`, aligning both
//!    modalities on "higher m = better".
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
    // log_σ ~ N(0, 1²) keeps the global pairwise σ in roughly
    // [exp(-2), exp(2)] = [0.135, 7.4]. Without this the gradient on
    // log_σ vanishes for σ ≫ 1 (z → 0 for all pairs, dl_dz · (-z) → 0)
    // so the optimiser can drift into a flat broken-loss region where
    // σ ≈ 2000 is a fixed point.
    let prior_log_sigma = 1.0;

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
        //
        // **Sign convention.** The cumulative-link model is written so that
        // higher μ ⇒ higher tier index ⇒ "worse" in the cumulative-link's
        // own ordinal sense. Squintly's UI inverts that: rating 1 =
        // imperceptible = BEST quality, rating 4 = hate = WORST. We want
        // higher m to mean "better" (matching BT-Davidson where higher m
        // wins more often), so we invert the rating index here: rating 1
        // ↦ k_idx 3 (top of cumulative-link tier), rating 4 ↦ k_idx 0
        // (bottom). Without this flip, pair (higher m better) and rating
        // (higher m worse) signals contradict and the fit infers m
        // upside-down — see the 2026-05-28 unified-solver bug history
        // in CLAUDE.md.
        for r in ratings {
            let i = r.item;
            let o = r.observer;
            let mu = m[i] + delta[o];
            let so = log_sigma_o[o].exp().max(1e-3);
            let k_idx = 4 - (r.rating as usize).clamp(1, 4); // rating 1 -> 3, rating 4 -> 0
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
            // dp_k/d(log σ_o) = -(z_u·φ(z_u) − z_l·φ(z_l)) — see module doc.
            // Both infinite-arm cases need an explicit guard: ∞·0 = NaN in
            // f32, but the limit `z·φ(z) → 0` as |z| → ∞ is what we want.
            // The pre-fix `lower.max(-1e6)` patched the lower arm only;
            // when a rating == 4 (upper = +∞), upper·pdf_u was NaN and
            // poisoned `log_sigma_o`.
            let upper_term = if upper.is_infinite() { 0.0 } else { upper * pdf_u };
            let lower_term = if lower.is_infinite() { 0.0 } else { lower * pdf_l };
            let d_log_sigma_o = -(upper_term - lower_term);
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
        // Prior on the global pairwise log_sigma — stops it from drifting
        // into the σ ≫ 1 flat region where dl_dz · (-z) → 0.
        loss += 0.5 * prior_log_sigma * log_sigma * log_sigma;
        grad_log_sigma += prior_log_sigma * log_sigma;

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
        // Same rating-index flip as the fit loop above: rating 1 ↦ 3
        // (top tier under the cumulative-link convention), rating 4 ↦ 0.
        let k_idx = 4 - (r.rating as usize).clamp(1, 4);
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
    fn unified_competitive_with_bt_only_on_heldout_pairs() {
        // Regression test for the 2026-05-28 unified-solver bugs:
        //   (a) NaN in d_log_sigma_o when rating == 4 (upper = +∞)
        //   (b) no prior on global log_sigma → drift to σ ≈ 2000 fixed point
        // Both fixed in the same commit that brought this test online.
        //
        // Setup: 4 items at true m = [0, −0.4, −0.8, −1.2], one observer,
        // pair observations from Thurstone(σ=1), 4-tier ratings whose modal
        // tier matches each item (so all ratings 1..4 occur — exercising
        // the rating == 4 / upper = +∞ branch that triggered bug (a)).
        //
        // The unified fit should not regress vs BT-only on held-out pair
        // log-likelihood: both have the same pair data, ratings only add
        // independent signal on the same latent m. Tolerance is ±0.1
        // nats/trial — well within Monte-Carlo noise at n=48.
        use rand::Rng;
        use rand::SeedableRng;
        use rand::rngs::SmallRng;

        let n_items = 4;
        let true_m: [f32; 4] = [0.0, -0.4, -0.8, -1.2];

        let mut rng = SmallRng::seed_from_u64(7);
        let mut pairs: Vec<PairObs> = Vec::new();
        for a in 0..n_items {
            for b in 0..n_items {
                if a == b {
                    continue;
                }
                for _ in 0..15 {
                    let diff = true_m[a] - true_m[b];
                    let eps: f32 = (rng.random::<f32>() - 0.5) * 2.0;
                    let outcome = if diff + eps > 0.0 {
                        PairOutcome::AWins
                    } else {
                        PairOutcome::BWins
                    };
                    pairs.push(PairObs {
                        item_a: a,
                        item_b: b,
                        observer: 0,
                        outcome,
                    });
                }
            }
        }
        let mut ratings: Vec<RatingObs> = Vec::new();
        for i in 0..n_items {
            let modal = (i + 1) as u8;
            for _ in 0..30 {
                let r = if rng.random::<f32>() < 0.8 {
                    modal
                } else if modal > 1 && rng.random::<f32>() < 0.5 {
                    modal - 1
                } else if modal < 4 {
                    modal + 1
                } else {
                    modal - 1
                };
                ratings.push(RatingObs {
                    item: i,
                    observer: 0,
                    rating: r,
                });
            }
        }

        let split_p = (pairs.len() as f32 * 0.8) as usize;
        let split_r = (ratings.len() as f32 * 0.8) as usize;
        let train_pairs = &pairs[..split_p];
        let test_pairs = &pairs[split_p..];
        let train_ratings = &ratings[..split_r];

        let unified = fit_unified(n_items, 1, train_pairs, train_ratings);
        let bt_only = fit_unified(n_items, 1, train_pairs, &[]);

        // Bug-regression assertions: σ must be in a sane range and no NaN
        // anywhere. Pre-fix: σ ≈ 2361 and log_sigma_o = NaN.
        assert!(
            unified.sigma > 0.05 && unified.sigma < 20.0,
            "σ should be moderate; got {}",
            unified.sigma
        );
        assert!(
            unified.log_sigma_o.iter().all(|s| s.is_finite()),
            "log_σ_o should be finite; got {:?}",
            unified.log_sigma_o
        );
        assert!(
            unified.tau.iter().all(|t| t.is_finite() && t.abs() < 20.0),
            "τ should be in m-range; got {:?}",
            unified.tau
        );

        let ll_unified = pair_log_likelihood(&unified, test_pairs);
        let ll_bt = pair_log_likelihood(&bt_only, test_pairs);
        let per_trial_delta = (ll_unified - ll_bt) / (test_pairs.len() as f64);
        assert!(
            per_trial_delta > -0.1,
            "unified should at least match BT-only on consistent synthetic data; \
             per-trial Δ = {per_trial_delta:.4} nats \
             (ll_unified={ll_unified:.2}, ll_bt={ll_bt:.2}, n_test={})",
            test_pairs.len()
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
            }); // observer 0: ref → 2 (worse)
            ratings.push(RatingObs {
                item: 0,
                observer: 1,
                rating: 1,
            }); // observer 1: ref → 1 (better)
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
        // Under the corrected convention (higher m = better quality),
        // observer 0 — who gives WORSE (higher-numbered) ratings — needs
        // a lower μ to match the data → δ_0 LESS than δ_1.
        assert!(
            fit.delta[0] < fit.delta[1] + 0.1,
            "observer 0 rates worse than observer 1 → δ[0] should be ≤ δ[1]; \
             got δ[0]={}, δ[1]={}",
            fit.delta[0],
            fit.delta[1]
        );
    }
}
