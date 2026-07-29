//! Participant exclusion **disposition** — the recorded decision about whether
//! an observer's data counts, and why.
//!
//! # Why a disposition and not a delete
//!
//! The IQA literature is consistent that hard-reject screening is the weaker
//! tool. From `~/work/zen/zenpapers/docs/iqa-methods/reference-book/`
//! `ch3-5_sampling_screening_cis.md` §4.2.2, tracing the LIVE-Meta avatar
//! paper's §III-E: BT.500's procedure "has two genuine costs: (1) it loses all
//! data from rejected subjects … and (2) the boundary between 'acceptable' and
//! 'rejected' is sharp — a subject who's only slightly noisy is treated
//! identically to a careful one if they pass the kurtosis-2 gate." SUREAL-style
//! per-subject modelling (§4.1.3) instead corrects each subject's bias and
//! weights them by their inconsistency, keeping every rating.
//!
//! Squintly already does the soft half: `grading.rs` produces a continuous
//! `session_weight` and an A–F grade. What was missing is the *hard* half in a
//! form that is auditable rather than destructive — so this module **computes
//! and records** the screens, and never removes a row. Whether a consumer
//! honours the disposition is a separate switch ([`ExclusionPolicy::enabled`]),
//! which is exactly the "default on/off" the analysis needs to report both
//! "screened" and "unscreened" numbers from one dataset.
//!
//! # The screens, and where each comes from
//!
//! * **§4.4 — correlation to the peer mean.** For each observer,
//!   `r_s = pearson(their ratings, per-stimulus mean over *other* observers)`.
//!   The chapter calls this "your first sieve before z-scoring or SUREAL" on an
//!   un-gated run, and reports the KADID-10k reproduction flagging at
//!   `r_s < 0.25` (0 of 2217 workers, that pipeline being pre-screened).
//! * **§4.2.1 — BT.500 kurtosis-2.** `β₂ = m₄/m₂²` over the observer's own
//!   scores picks the rejection band: `2σ` when `2 ≤ β₂ ≤ 4` (approximately
//!   normal), else `√20 σ ≈ 4.47σ`. Then per stimulus, count how often the
//!   observer falls outside `μ_e ± band·σ_e`, where `μ_e`/`σ_e` are taken over
//!   *other* observers.
//!
//! BT.500's own "too many of their scores" rejection ratio is **not in the
//! corpus** (§4.2.1 marks it `[unverified]`), so [`ExclusionPolicy`] carries an
//! explicit, configurable `outlier_rate_ceiling` rather than a number invented
//! here and dressed up as ITU-R.
//!
//! # Abstention is not exclusion
//!
//! Every screen above is defined against a reference distribution built from
//! *other observers on the same stimulus*. With few peers that reference is
//! noise, and §4.6 is explicit that below ~15 subjects the modelling approach
//! is under-identified. So when the evidence is too thin this returns
//! [`Disposition::InsufficientData`] — never [`Disposition::Excluded`]. That is
//! what makes a single-expert run behave correctly without special-casing it:
//! with no peers there is nothing to be an outlier against, so nobody is
//! excluded, rather than everybody.

use std::collections::HashMap;

/// What we decided about one observer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Disposition {
    /// Screened and kept.
    Included,
    /// Screened and flagged. The data is still stored; consumers that honour
    /// the policy leave it out of the aggregate.
    Excluded,
    /// Not enough overlap with other observers to screen at all. Distinct from
    /// `Included` on purpose: "we checked and they're fine" and "we could not
    /// check" must not look the same in an audit.
    InsufficientData,
}

impl Disposition {
    pub fn as_str(&self) -> &'static str {
        match self {
            Disposition::Included => "included",
            Disposition::Excluded => "excluded",
            Disposition::InsufficientData => "insufficient_data",
        }
    }
}

/// Thresholds for the screens. Every number is explicit because the ones the
/// corpus does not pin down must not masquerade as standard.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExclusionPolicy {
    /// Whether consumers should *act* on an `Excluded` disposition. The screens
    /// run and are recorded either way.
    pub enabled: bool,
    /// Minimum ratings by this observer before any screen is meaningful.
    pub min_ratings: usize,
    /// Minimum other observers on a stimulus before it can contribute a
    /// reference mean. Below this, `μ_e`/`σ_e` are noise.
    pub min_peers_per_stimulus: usize,
    /// Minimum stimuli with enough peers before we will judge at all.
    pub min_comparable: usize,
    /// §4.4: flag below this correlation with the peer mean.
    pub r_s_floor: f64,
    /// §4.2.1: flag above this share of scores outside the BT.500 band. The
    /// recommendation's own ratio is not in the corpus — this is our stated
    /// choice, not a quotation.
    pub outlier_rate_ceiling: f64,
}

impl Default for ExclusionPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            min_ratings: 10,
            // Two peers is the least that gives a defined standard deviation.
            min_peers_per_stimulus: 2,
            min_comparable: 8,
            r_s_floor: 0.25,
            outlier_rate_ceiling: 0.30,
        }
    }
}

impl ExclusionPolicy {
    /// Start from a study's default, then let the environment override it.
    ///
    /// `SQUINTLY_EXCLUSION=on|off` (also `1`/`0`, `true`/`false`). An
    /// unparseable value keeps the study default and warns rather than guessing
    /// — silently screening, or silently not screening, changes every number
    /// downstream.
    pub fn for_study(study_default: bool) -> Self {
        let mut p = Self {
            enabled: study_default,
            ..Self::default()
        };
        if let Ok(v) = std::env::var("SQUINTLY_EXCLUSION") {
            match v.trim().to_ascii_lowercase().as_str() {
                "on" | "1" | "true" | "yes" => p.enabled = true,
                "off" | "0" | "false" | "no" => p.enabled = false,
                other => tracing::warn!(
                    value = %other,
                    study_default,
                    "unparseable SQUINTLY_EXCLUSION; keeping the study default"
                ),
            }
        }
        p
    }
}

/// One observer's screening statistics, plus the decision they imply.
#[derive(Debug, Clone, PartialEq)]
pub struct ObserverScreen {
    pub n_ratings: usize,
    /// Ratings on stimuli that had enough other observers to compare against.
    pub n_comparable: usize,
    pub mean_rating: Option<f64>,
    pub sd_rating: Option<f64>,
    /// BT.500 β₂ = m₄/m₂² over this observer's own scores.
    pub beta2: Option<f64>,
    /// The band β₂ selected: 2.0, or √20 for a non-normal distribution.
    pub band_sigma: Option<f64>,
    pub outlier_count: usize,
    pub outlier_rate: Option<f64>,
    /// §4.4 correlation with the per-stimulus mean over other observers.
    pub r_s: Option<f64>,
    pub disposition: Disposition,
    /// Why, in words, for the audit trail. `None` when included.
    pub reason: Option<String>,
}

/// One rating by the observer under test, alongside the scores *other*
/// observers gave the same stimulus.
#[derive(Debug, Clone)]
pub struct RatingWithPeers {
    pub stimulus: String,
    pub score: f64,
    /// Scores from other observers on this stimulus. Never includes this
    /// observer — the reference must be independent of the subject being
    /// screened, or a lone outlier drags its own reference toward itself.
    pub peers: Vec<f64>,
}

fn mean(xs: &[f64]) -> Option<f64> {
    (!xs.is_empty()).then(|| xs.iter().sum::<f64>() / xs.len() as f64)
}

/// Population standard deviation (BT.500's moments are population moments).
fn sd(xs: &[f64]) -> Option<f64> {
    let m = mean(xs)?;
    if xs.len() < 2 {
        return None;
    }
    let var = xs.iter().map(|x| (x - m).powi(2)).sum::<f64>() / xs.len() as f64;
    Some(var.sqrt())
}

fn pearson(x: &[f64], y: &[f64]) -> Option<f64> {
    if x.len() < 2 || x.len() != y.len() {
        return None;
    }
    let mx = mean(x)?;
    let my = mean(y)?;
    let mut num = 0.0;
    let mut dx = 0.0;
    let mut dy = 0.0;
    for (a, b) in x.iter().zip(y.iter()) {
        num += (a - mx) * (b - my);
        dx += (a - mx).powi(2);
        dy += (b - my).powi(2);
    }
    let denom = (dx * dy).sqrt();
    (denom > 1e-12).then(|| num / denom)
}

/// BT.500 β₂ = m₄ / m₂², the kurtosis of one subject's own score distribution.
///
/// `None` when the subject gave the same score every time: m₂ = 0 makes the
/// ratio undefined, and "answered 3 to everything" is a straight-lining signal
/// that `grading.rs` already owns — not something to launder into a kurtosis.
pub fn beta2(scores: &[f64]) -> Option<f64> {
    let m = mean(scores)?;
    if scores.len() < 2 {
        return None;
    }
    let n = scores.len() as f64;
    let m2 = scores.iter().map(|x| (x - m).powi(2)).sum::<f64>() / n;
    let m4 = scores.iter().map(|x| (x - m).powi(4)).sum::<f64>() / n;
    (m2 > 1e-12).then(|| m4 / (m2 * m2))
}

/// §4.2.1: `2σ` when the score distribution is approximately normal
/// (`2 ≤ β₂ ≤ 4`), else `√20 σ`.
///
/// An undefined β₂ takes the wide band. Erring wide means a subject we could
/// not characterise is less likely to be excluded, which is the direction this
/// module errs in throughout.
pub fn rejection_band(beta2: Option<f64>) -> f64 {
    match beta2 {
        Some(b) if (2.0..=4.0).contains(&b) => 2.0,
        _ => 20f64.sqrt(),
    }
}

/// Screen one observer against their peers.
pub fn screen_observer(policy: &ExclusionPolicy, ratings: &[RatingWithPeers]) -> ObserverScreen {
    let own: Vec<f64> = ratings.iter().map(|r| r.score).collect();
    let b2 = beta2(&own);
    let band = rejection_band(b2);

    // Only stimuli with enough independent peers can say anything.
    let comparable: Vec<&RatingWithPeers> = ratings
        .iter()
        .filter(|r| r.peers.len() >= policy.min_peers_per_stimulus)
        .collect();

    let mut outliers = 0usize;
    let mut mine = Vec::with_capacity(comparable.len());
    let mut theirs = Vec::with_capacity(comparable.len());
    for r in &comparable {
        let Some(mu) = mean(&r.peers) else { continue };
        mine.push(r.score);
        theirs.push(mu);
        // A zero-spread peer set means every peer agreed exactly. Treating that
        // as "any deviation is an outlier" would punish a subject for a
        // one-notch difference on an easy stimulus, so it takes no part in the
        // outlier count.
        if let Some(sigma) = sd(&r.peers) {
            if sigma > 1e-12 && (r.score - mu).abs() > band * sigma {
                outliers += 1;
            }
        }
    }

    let n_comparable = comparable.len();
    let outlier_rate = (n_comparable > 0).then(|| outliers as f64 / n_comparable as f64);
    let r_s = pearson(&mine, &theirs);

    let mut screen = ObserverScreen {
        n_ratings: own.len(),
        n_comparable,
        mean_rating: mean(&own),
        sd_rating: sd(&own),
        beta2: b2,
        band_sigma: Some(band),
        outlier_count: outliers,
        outlier_rate,
        r_s,
        disposition: Disposition::InsufficientData,
        reason: None,
    };

    if own.len() < policy.min_ratings {
        screen.reason = Some(format!(
            "only {} ratings; need {} to screen",
            own.len(),
            policy.min_ratings
        ));
        return screen;
    }
    if n_comparable < policy.min_comparable {
        // The single-expert case lands here: nobody else has rated these
        // stimuli, so there is no reference distribution and therefore no
        // outlier to detect.
        screen.reason = Some(format!(
            "only {} of {} ratings had {}+ other observers to compare against; need {}",
            n_comparable,
            own.len(),
            policy.min_peers_per_stimulus,
            policy.min_comparable
        ));
        return screen;
    }

    let mut reasons = Vec::new();
    if let Some(r) = r_s {
        if r < policy.r_s_floor {
            reasons.push(format!(
                "correlation with the peer mean is {r:.2}, below {:.2} (ch4 §4.4)",
                policy.r_s_floor
            ));
        }
    }
    if let Some(rate) = outlier_rate {
        if rate > policy.outlier_rate_ceiling {
            reasons.push(format!(
                "{:.0}% of scores fall outside the {band:.2}σ BT.500 band, above {:.0}% (ch4 §4.2.1)",
                rate * 100.0,
                policy.outlier_rate_ceiling * 100.0
            ));
        }
    }

    if reasons.is_empty() {
        screen.disposition = Disposition::Included;
    } else {
        screen.disposition = Disposition::Excluded;
        screen.reason = Some(reasons.join("; "));
    }
    screen
}

/// Build per-observer inputs from a flat `(observer, stimulus, score)` table,
/// giving each observer a peer set that excludes themselves.
pub fn group_with_peers(rows: &[(String, String, f64)]) -> HashMap<String, Vec<RatingWithPeers>> {
    let mut by_stimulus: HashMap<&str, Vec<(&str, f64)>> = HashMap::new();
    for (observer, stimulus, score) in rows {
        by_stimulus
            .entry(stimulus.as_str())
            .or_default()
            .push((observer.as_str(), *score));
    }
    let mut out: HashMap<String, Vec<RatingWithPeers>> = HashMap::new();
    for (observer, stimulus, score) in rows {
        let peers = by_stimulus
            .get(stimulus.as_str())
            .map(|v| {
                v.iter()
                    .filter(|(o, _)| *o != observer.as_str())
                    .map(|(_, s)| *s)
                    .collect()
            })
            .unwrap_or_default();
        out.entry(observer.clone())
            .or_default()
            .push(RatingWithPeers {
                stimulus: stimulus.clone(),
                score: *score,
                peers,
            });
    }
    out
}

/// Recompute every observer's disposition from the stored responses.
///
/// Idempotent — wipes the table and re-inserts, matching
/// `grading::rebuild_observer_grades`. Screens run per study: two studies
/// measure different quantities, so pooling their scores would build the
/// reference distribution out of values that were never comparable.
///
/// Only single-stimulus ratings feed the screens. A pairwise choice is an
/// ordinal preference between two encodings, not a score on a common scale;
/// putting "a" and "b" through a kurtosis test would be nonsense. Studies that
/// are pairwise-only therefore land on `insufficient_data` for everyone, which
/// is the honest answer rather than a fabricated one.
pub async fn rebuild_dispositions(
    pool: &sqlx::SqlitePool,
    policy_for_study: impl Fn(&str) -> ExclusionPolicy,
) -> anyhow::Result<u64> {
    use sqlx::Row;

    let rows = sqlx::query(
        "SELECT s.study_id AS study_id, s.observer_id AS observer_id, \
                t.a_encoding_id AS stimulus, r.choice AS choice \
         FROM responses r \
         JOIN trials t ON t.id = r.trial_id \
         JOIN sessions s ON s.id = t.session_id \
         WHERE t.kind = 'single' AND s.observer_id IS NOT NULL",
    )
    .fetch_all(pool)
    .await?;

    // study -> (observer, stimulus, score)
    let mut by_study: HashMap<String, Vec<(String, String, f64)>> = HashMap::new();
    for row in &rows {
        let study: Option<String> = row.try_get("study_id").ok().flatten();
        let observer: String = row.get("observer_id");
        let stimulus: String = row.get("stimulus");
        let choice: String = row.get("choice");
        // Non-numeric choices are pairwise leftovers or skips; they carry no
        // position on the rating scale, so they are skipped rather than coerced.
        let Ok(score) = choice.trim().parse::<f64>() else {
            continue;
        };
        by_study
            .entry(study.unwrap_or_else(|| crate::studies::DEFAULT_STUDY_ID.to_string()))
            .or_default()
            .push((observer, stimulus, score));
    }

    let now = crate::db::now_ms();
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM observer_dispositions")
        .execute(&mut *tx)
        .await?;

    let mut written = 0u64;
    for (study_id, triples) in &by_study {
        let policy = policy_for_study(study_id);
        for (observer, ratings) in group_with_peers(triples) {
            let s = screen_observer(&policy, &ratings);
            sqlx::query(
                "INSERT INTO observer_dispositions \
                 (observer_id, study_id, n_ratings, n_comparable, mean_rating, sd_rating, \
                  beta2, band_sigma, outlier_count, outlier_rate, r_s, disposition, reason, \
                  policy_enabled, computed_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&observer)
            .bind(study_id)
            .bind(s.n_ratings as i64)
            .bind(s.n_comparable as i64)
            .bind(s.mean_rating)
            .bind(s.sd_rating)
            .bind(s.beta2)
            .bind(s.band_sigma)
            .bind(s.outlier_count as i64)
            .bind(s.outlier_rate)
            .bind(s.r_s)
            .bind(s.disposition.as_str())
            .bind(s.reason.as_deref())
            .bind(i64::from(policy.enabled))
            .bind(now)
            .execute(&mut *tx)
            .await?;
            written += 1;
        }
    }
    tx.commit().await?;
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> ExclusionPolicy {
        ExclusionPolicy {
            enabled: true,
            min_ratings: 4,
            min_peers_per_stimulus: 2,
            min_comparable: 4,
            ..ExclusionPolicy::default()
        }
    }

    /// A normal-ish distribution sits in [2,4] and takes the tight band; a
    /// heavy-tailed one must take the wide one (§4.2.1).
    #[test]
    fn the_kurtosis_test_picks_the_band() {
        let normalish = [1.0, 2.0, 2.0, 3.0, 3.0, 2.0, 2.0, 3.0, 1.0, 2.0];
        let b = beta2(&normalish).expect("defined");
        assert!((2.0..=4.0).contains(&b), "β₂ = {b} should look normal");
        assert_eq!(rejection_band(Some(b)), 2.0);

        // Nearly all mass at one value with a lone far outlier — classic
        // heavy tail, β₂ well above 4.
        let heavy = [2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 40.0];
        let b = beta2(&heavy).expect("defined");
        assert!(b > 4.0, "β₂ = {b} should be heavy-tailed");
        assert!((rejection_band(Some(b)) - 20f64.sqrt()).abs() < 1e-9);
    }

    /// A constant scorer has m₂ = 0. That must be undefined, not a divide by
    /// zero, and must take the wide (forgiving) band.
    #[test]
    fn a_constant_scorer_has_no_kurtosis_and_gets_the_wide_band() {
        assert_eq!(beta2(&[3.0, 3.0, 3.0, 3.0]), None);
        assert!((rejection_band(None) - 20f64.sqrt()).abs() < 1e-9);
    }

    /// The reference distribution must exclude the subject being screened —
    /// otherwise a lone outlier pulls its own reference toward itself and
    /// hides.
    #[test]
    fn peers_never_include_the_observer_being_screened() {
        let rows = vec![
            ("a".to_string(), "s1".to_string(), 1.0),
            ("b".to_string(), "s1".to_string(), 2.0),
            ("c".to_string(), "s1".to_string(), 3.0),
        ];
        let grouped = group_with_peers(&rows);
        let a = &grouped["a"][0];
        assert_eq!(a.score, 1.0);
        let mut peers = a.peers.clone();
        peers.sort_by(|x, y| x.partial_cmp(y).unwrap());
        assert_eq!(peers, vec![2.0, 3.0], "must be b and c only");
    }

    #[test]
    fn an_agreeing_observer_is_included() {
        // Six stimuli, three observers in exact agreement.
        let mut rows = Vec::new();
        let truth = [1.0, 2.0, 3.0, 4.0, 2.0, 3.0];
        for (i, t) in truth.iter().enumerate() {
            for who in ["a", "b", "c"] {
                rows.push((who.to_string(), format!("s{i}"), *t));
            }
        }
        let grouped = group_with_peers(&rows);
        let s = screen_observer(&policy(), &grouped["a"]);
        assert_eq!(
            s.disposition,
            Disposition::Included,
            "reason: {:?}",
            s.reason
        );
        assert_eq!(s.n_comparable, 6);
        assert!(s.r_s.unwrap() > 0.9);
    }

    /// The §4.4 sieve: someone whose ratings run opposite to everyone else's.
    #[test]
    fn an_anticorrelated_observer_is_excluded_with_a_reason() {
        let truth = [1.0, 2.0, 3.0, 4.0, 1.0, 2.0, 3.0, 4.0];
        let mut rows = Vec::new();
        for (i, t) in truth.iter().enumerate() {
            for who in ["b", "c", "d"] {
                rows.push((who.to_string(), format!("s{i}"), *t));
            }
            // "a" answers upside down.
            rows.push(("a".to_string(), format!("s{i}"), 5.0 - t));
        }
        let grouped = group_with_peers(&rows);
        let s = screen_observer(&policy(), &grouped["a"]);
        assert_eq!(s.disposition, Disposition::Excluded);
        assert!(s.r_s.unwrap() < 0.0, "r_s = {:?}", s.r_s);
        let reason = s.reason.unwrap();
        assert!(reason.contains("§4.4"), "must cite the screen: {reason}");
    }

    /// The whole point of the module: a single expert with no peers must come
    /// back "could not check", never "excluded".
    #[test]
    fn a_lone_observer_is_never_excluded_only_unscreenable() {
        let rows: Vec<_> = (0..20)
            .map(|i| ("solo".to_string(), format!("s{i}"), (i % 4) as f64 + 1.0))
            .collect();
        let grouped = group_with_peers(&rows);
        let s = screen_observer(&policy(), &grouped["solo"]);
        assert_eq!(
            s.disposition,
            Disposition::InsufficientData,
            "a solo expert has no reference distribution to be an outlier against"
        );
        assert_eq!(s.n_comparable, 0);
        assert!(s.reason.unwrap().contains("other observers"));
    }

    /// Too few ratings is also abstention, not exclusion.
    #[test]
    fn a_barely_started_observer_is_not_excluded() {
        let rows = vec![
            ("a".to_string(), "s1".to_string(), 1.0),
            ("b".to_string(), "s1".to_string(), 1.0),
            ("c".to_string(), "s1".to_string(), 1.0),
        ];
        let grouped = group_with_peers(&rows);
        let s = screen_observer(&policy(), &grouped["a"]);
        assert_eq!(s.disposition, Disposition::InsufficientData);
        assert!(s.reason.unwrap().contains("need"));
    }

    /// Unanimous peers must not turn a one-notch difference into an outlier.
    #[test]
    fn zero_spread_peers_do_not_manufacture_outliers() {
        let mut rows = Vec::new();
        for i in 0..8 {
            for who in ["b", "c"] {
                rows.push((who.to_string(), format!("s{i}"), 2.0));
            }
            rows.push(("a".to_string(), format!("s{i}"), 3.0));
        }
        let grouped = group_with_peers(&rows);
        let s = screen_observer(&policy(), &grouped["a"]);
        assert_eq!(s.outlier_count, 0, "σ_e = 0 must not flag anything");
    }

    /// `enabled` must not change what gets measured — only whether consumers
    /// act on it. Both settings must produce the identical screen.
    #[test]
    fn the_switch_changes_action_not_measurement() {
        let truth = [1.0, 2.0, 3.0, 4.0, 1.0, 2.0, 3.0, 4.0];
        let mut rows = Vec::new();
        for (i, t) in truth.iter().enumerate() {
            for who in ["b", "c", "d"] {
                rows.push((who.to_string(), format!("s{i}"), *t));
            }
            rows.push(("a".to_string(), format!("s{i}"), 5.0 - t));
        }
        let grouped = group_with_peers(&rows);
        let on = screen_observer(
            &ExclusionPolicy {
                enabled: true,
                ..policy()
            },
            &grouped["a"],
        );
        let off = screen_observer(
            &ExclusionPolicy {
                enabled: false,
                ..policy()
            },
            &grouped["a"],
        );
        assert_eq!(on, off, "the disposition is recorded either way");
        assert_eq!(off.disposition, Disposition::Excluded);
    }

    #[test]
    fn the_env_override_beats_the_study_default_and_bad_values_do_not() {
        // SAFETY: one test owning this var; cargo runs tests in parallel threads.
        unsafe { std::env::set_var("SQUINTLY_EXCLUSION", "off") };
        assert!(!ExclusionPolicy::for_study(true).enabled);
        unsafe { std::env::set_var("SQUINTLY_EXCLUSION", "on") };
        assert!(ExclusionPolicy::for_study(false).enabled);
        unsafe { std::env::set_var("SQUINTLY_EXCLUSION", "banana") };
        assert!(
            ExclusionPolicy::for_study(true).enabled,
            "an unparseable value must keep the study default, not silently flip it"
        );
        unsafe { std::env::remove_var("SQUINTLY_EXCLUSION") };
        assert!(ExclusionPolicy::for_study(true).enabled);
        assert!(!ExclusionPolicy::for_study(false).enabled);
    }
}
