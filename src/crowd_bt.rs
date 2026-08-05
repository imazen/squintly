//! Crowd-BT: Bradley–Terry with a per-observer reliability parameter.
//!
//! # Why this screen, and not the two we had
//!
//! `exclusion.rs` runs BT.500 kurtosis-2 (§4.2.1) and correlation-to-peer-mean
//! (§4.4). Both are the wrong instrument for this study's main arm, and the
//! project's own notes already said so:
//!
//!  * `docs/participant-grading.md` §1, verbatim: *"Do NOT use ITU-R BT.500
//!    Annex A β₂ rejection on pairwise data. It is defined for MOS — it computes
//!    per-stimulus mean and SD across observers … Pairwise data has no
//!    per-stimulus rating, so the test is undefined."*
//!  * §4.4's `r_s` correlates an observer against the per-image mean, which
//!    likewise needs ratings. On a forced choice there is no per-image score to
//!    correlate against.
//!
//! And the reference book's own cheat sheet (`ch3-5_sampling_screening_cis.md`
//! §4.6) names the right one: *"Crowdsourcing PC test → **Crowd-BT η_s** if
//! active sampling already in use"*. Squintly's sampler IS active (ASAP, see
//! `asap.rs`), and `ssim2-nonphoto` — the default study — is forced choice. So
//! η is the screen this design actually calls for.
//!
//! # The model
//!
//! Vanilla BT: `P(i ≻ j) = π_i / (π_i + π_j)`. Crowd-BT gives each observer `s`
//! a reliability `η_s ∈ [0, 1]` and lets their report flip the truth with
//! probability `1 − η_s` (§3.5):
//!
//! ```text
//! P(s says i ≻ j) = η_s · π_i/(π_i+π_j) + (1 − η_s) · π_j/(π_i+π_j)
//! ```
//!
//! `η = 1` is a perfectly reliable observer; `η = 0.5` is a coin flip whose
//! judgements carry no information at all; `η < 0.5` is anti-correlated, which
//! in practice means a reversed UI or someone answering the opposite of what
//! they meant — worth knowing about, and invisible to a screen that only asks
//! "is this observer unusual?".
//!
//! # What this is NOT for
//!
//! Not for sampling. The chapter is explicit (§3.5, reference performance) that
//! Crowd-BT's *active* variant scores worse than ASAP on correlation at every
//! budget, because the joint EIG is expensive and the prior on η is hard to set.
//! We keep ASAP for choosing pairs and use Crowd-BT only post hoc, to estimate
//! how much to believe each observer. Those are separate jobs and the chapter
//! recommends the two different tools for them.

/// One recorded comparison.
#[derive(Debug, Clone, Copy)]
pub struct Obs {
    /// Index of the observer who answered.
    pub observer: usize,
    /// Index of the item they preferred.
    pub winner: usize,
    /// Index of the item they did not.
    pub loser: usize,
}

#[derive(Debug, Clone)]
pub struct Fit {
    /// Latent quality per item, on the log scale (`π_i = exp(score_i)`).
    pub scores: Vec<f64>,
    /// Reliability per observer.
    pub eta: Vec<f64>,
}

/// Below this an observer is worse than a coin flip in the aggregate.
///
/// 0.5 is the meaningful boundary, not a tunable: it is the point at which a
/// judgement carries no information about the latent order. The chapter puts it
/// the same way — *"Treat `η_s = 0.5` as the threshold for 'random / useless'"*.
pub const ETA_USELESS: f64 = 0.5;

/// Observations below which η is not estimated for that observer.
///
/// η from three answers is not a reliability estimate, it is three coin flips.
/// The literature's own caution about screening early (`participant-grading.md`
/// §1: "Do NOT reject observers before N≥20 trials") applies with more force
/// here, because η and the scores are fitted jointly and a thin observer drags
/// the scores rather than just mis-scoring themselves.
pub const MIN_OBS_FOR_ETA: usize = 20;

/// Keep η away from the boundaries where the log-likelihood is degenerate.
const ETA_MIN: f64 = 0.05;
const ETA_MAX: f64 = 0.99;

/// Fit scores and per-observer reliability by alternating ascent.
///
/// Alternating rather than joint Newton: the two blocks have very different
/// curvature, and with a handful of observers against thousands of comparisons
/// a joint Hessian is both singular and unnecessary. Each block is a small
/// concave problem given the other, which is the standard construction (§4.1.4
/// uses the same alternating projection for SUREAL).
pub fn fit(obs: &[Obs], n_items: usize, n_observers: usize) -> Fit {
    let mut scores = vec![0.0f64; n_items];
    // Start optimistic. Starting at 0.5 is a fixed point when the scores are
    // flat — every observer looks like a coin flip because nothing is ranked
    // yet — and the fit never leaves it.
    let mut eta = vec![0.9f64; n_observers];

    let counts = observation_counts(obs, n_observers);

    for _ in 0..200 {
        let before = scores.clone();
        update_scores(&mut scores, &eta, obs);
        update_eta(&mut eta, &scores, obs, &counts);
        let delta: f64 = before
            .iter()
            .zip(&scores)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f64::max);
        if delta < 1e-7 {
            break;
        }
    }
    Fit { scores, eta }
}

fn observation_counts(obs: &[Obs], n_observers: usize) -> Vec<usize> {
    let mut counts = vec![0usize; n_observers];
    for o in obs {
        if o.observer < n_observers {
            counts[o.observer] += 1;
        }
    }
    counts
}

/// One gradient step on the scores, holding η fixed.
fn update_scores(scores: &mut [f64], eta: &[f64], obs: &[Obs]) {
    let mut grad = vec![0.0f64; scores.len()];
    for o in obs {
        let (a, b) = (o.winner, o.loser);
        if a >= scores.len() || b >= scores.len() {
            continue;
        }
        let e = eta.get(o.observer).copied().unwrap_or(1.0);
        // p = BT probability the winner really is better.
        let p = 1.0 / (1.0 + (scores[b] - scores[a]).exp());
        // Likelihood of what was OBSERVED, through the flip channel.
        let obs_p = e * p + (1.0 - e) * (1.0 - p);
        if obs_p <= 1e-12 {
            continue;
        }
        // d/d(score_a) of log obs_p. The flip channel scales the usual BT
        // gradient by (2η − 1): an observer at η = 0.5 contributes exactly
        // nothing, which is the whole point of the model.
        let g = (2.0 * e - 1.0) * p * (1.0 - p) / obs_p;
        grad[a] += g;
        grad[b] -= g;
    }
    // Fixed small step; the objective is smooth and this runs to convergence.
    for (s, g) in scores.iter_mut().zip(&grad) {
        *s += 0.01 * g;
    }
    // Gauge fix: BT scores are only identified up to an additive constant.
    let mean = scores.iter().sum::<f64>() / scores.len().max(1) as f64;
    for s in scores.iter_mut() {
        *s -= mean;
    }
}

/// Closed-form-ish update of η given the scores.
///
/// For observer `s`, η maximises `Σ log(η·p + (1−η)·(1−p))` over their answers.
/// One Newton step per round is plenty inside the alternating loop.
fn update_eta(eta: &mut [f64], scores: &[f64], obs: &[Obs], counts: &[usize]) {
    let mut d1 = vec![0.0f64; eta.len()];
    let mut d2 = vec![0.0f64; eta.len()];
    for o in obs {
        if o.observer >= eta.len() || o.winner >= scores.len() || o.loser >= scores.len() {
            continue;
        }
        let p = 1.0 / (1.0 + (scores[o.loser] - scores[o.winner]).exp());
        let e = eta[o.observer];
        let denom = e * p + (1.0 - e) * (1.0 - p);
        if denom <= 1e-12 {
            continue;
        }
        let num = 2.0 * p - 1.0;
        d1[o.observer] += num / denom;
        d2[o.observer] -= (num * num) / (denom * denom);
    }
    for s in 0..eta.len() {
        // An observer with too few answers keeps the prior rather than being
        // handed a number that looks like a measurement.
        if counts[s] < MIN_OBS_FOR_ETA || d2[s] >= -1e-12 {
            continue;
        }
        let step = d1[s] / -d2[s];
        eta[s] = (eta[s] + step).clamp(ETA_MIN, ETA_MAX);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a synthetic study: items with a known true order, and observers who
    /// report it with a given probability.
    fn synth(reliabilities: &[f64], n_items: usize, per_observer: usize) -> Vec<Obs> {
        // Deterministic pseudo-random so the test cannot flake.
        let mut state: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut out = Vec::new();
        for (s, &r) in reliabilities.iter().enumerate() {
            for k in 0..per_observer {
                let i = k % n_items;
                let j = (k / n_items + 1 + i) % n_items;
                if i == j {
                    continue;
                }
                // Truth: the higher index is the better item.
                let (better, worse) = if i > j { (i, j) } else { (j, i) };
                let truthful = next() < r;
                out.push(Obs {
                    observer: s,
                    winner: if truthful { better } else { worse },
                    loser: if truthful { worse } else { better },
                });
            }
        }
        out
    }

    /// The property the screen exists for: a careful observer and a coin-flipper
    /// must come out different, and the coin-flipper must land at chance.
    #[test]
    fn a_random_observer_is_separated_from_a_careful_one() {
        let obs = synth(&[0.95, 0.5], 6, 240);
        let f = fit(&obs, 6, 2);
        assert!(
            f.eta[0] > 0.8,
            "a careful observer should read as reliable, got {}",
            f.eta[0]
        );
        assert!(
            f.eta[1] < 0.65,
            "a coin-flipper should land near chance, got {}",
            f.eta[1]
        );
        assert!(f.eta[0] > f.eta[1] + 0.2, "the two must be separable");
    }

    /// An observer answering the opposite of what they mean — a reversed UI, a
    /// misread instruction — is a distinct failure from being noisy, and only a
    /// signed reliability can see it.
    #[test]
    fn an_anti_correlated_observer_reads_below_chance() {
        let obs = synth(&[0.95, 0.95, 0.05], 6, 240);
        let f = fit(&obs, 6, 3);
        assert!(
            f.eta[2] < ETA_USELESS,
            "an inverted observer must read below {ETA_USELESS}, got {}",
            f.eta[2]
        );
    }

    /// The scores must still come out right when most observers are good — that
    /// is what makes η worth having rather than just a diagnostic.
    #[test]
    fn the_latent_order_survives_one_bad_observer() {
        let obs = synth(&[0.95, 0.95, 0.5], 5, 200);
        let f = fit(&obs, 5, 3);
        for i in 1..5 {
            assert!(
                f.scores[i] > f.scores[i - 1],
                "item {i} should outrank {}: scores {:?}",
                i - 1,
                f.scores
            );
        }
    }

    /// η from a handful of answers is a handful of coin flips, not a
    /// measurement — so a thin observer keeps the prior rather than being handed
    /// a number that looks like one.
    #[test]
    fn a_thin_observer_keeps_the_prior_rather_than_a_fake_number() {
        let mut obs = synth(&[0.95], 6, 240);
        // A second observer with far fewer answers than MIN_OBS_FOR_ETA, all of
        // them wrong — so if the guard were absent, η would be driven to the
        // floor and the difference would be unmistakable.
        let thin = MIN_OBS_FOR_ETA / 4;
        assert!(thin > 0 && thin < MIN_OBS_FOR_ETA);
        for k in 0..thin {
            obs.push(Obs {
                observer: 1,
                winner: k % 5,
                loser: (k % 5) + 1,
            });
        }
        let f = fit(&obs, 6, 2);
        assert!(
            (f.eta[1] - 0.9).abs() < 1e-9,
            "a thin observer should keep the prior, got {}",
            f.eta[1]
        );
        // And the observer who did the work is still measured.
        assert!(f.eta[0] > 0.8, "got {}", f.eta[0]);
    }

    /// A degenerate input must not panic or produce NaN — this runs against
    /// live data where a study can be one observer old.
    #[test]
    fn an_empty_or_tiny_study_is_survivable() {
        let f = fit(&[], 3, 1);
        assert!(f.scores.iter().all(|s| s.is_finite()));
        assert!(f.eta.iter().all(|e| e.is_finite()));

        let one = [Obs {
            observer: 0,
            winner: 1,
            loser: 0,
        }];
        let f = fit(&one, 2, 1);
        assert!(f.scores.iter().all(|s| s.is_finite()));
        assert!(f.eta.iter().all(|e| e.is_finite()));
    }
}
