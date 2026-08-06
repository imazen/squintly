//! How close the study is to answering its question, and what the answer looks
//! like so far.
//!
//! The default study asks whether SSIMULACRA2 ranks non-photo encodings as well
//! as it ranks photographs (imazen/squintly#4). This module computes both sides
//! of that and — more importantly — the things that decide whether the number
//! means anything.
//!
//! # The three numbers, in the order they must be read
//!
//! 1. **The noise ceiling.** `Study::p_repeat` re-serves pairs an observer has
//!    already answered; their agreement with themselves bounds what ANY metric
//!    could score against them. A metric cannot agree with a person more than
//!    that person agrees with themselves.
//! 2. **The metric's agreement**, ρ — the fraction of comparisons where the
//!    metric ordered the pair the way the observer did.
//! 3. **ρ / ceiling.** This is the reportable figure. "ssim2 scored 0.7" reads
//!    completely differently against a ceiling of 0.95 than against 0.72, and
//!    the whole point of the photo control arm is that humans may simply be
//!    noisier on one content class than another.
//!
//! Reporting ρ without the ceiling is the specific mistake this module exists
//! to prevent, so [`MetricAgreement`] cannot be constructed without one.
//!
//! # What it will not do
//!
//! - Correlate a metric whose direction is unknown. The sign would be a guess,
//!   and a flipped sign reads exactly like the finding the study is trying to
//!   make. See `metrics::Direction`.
//! - Report a ceiling from fewer than [`MIN_REPEATS_FOR_CEILING`] repeat pairs.
//! - Silently treat ties as agreement or disagreement. A tie is an outcome in
//!   Davidson's model, not noise, so it is counted separately and excluded from
//!   the agreement denominator — a metric is never asked to predict a tie,
//!   since no threshold on a continuous score corresponds to "these look the
//!   same to a person".

use crate::metrics::{Direction, direction_of};
use serde::Serialize;
use sqlx::SqlitePool;

/// Below this many repeated pairs, self-agreement is too noisy to bound
/// anything — at 5 repeats one flip moves the ceiling by 20 points.
pub const MIN_REPEATS_FOR_CEILING: usize = 10;

/// Below this many usable comparisons, ρ is not worth printing.
pub const MIN_PAIRS_FOR_RHO: usize = 20;

/// One observer's agreement with themselves on pairs they answered twice.
#[derive(Debug, Clone, Serialize)]
pub struct NoiseCeiling {
    /// Pairs answered more than once by the same observer.
    pub repeat_pairs: usize,
    /// Of those, how many got the same answer both times.
    pub agreed: usize,
    /// `agreed / repeat_pairs`, or `None` below [`MIN_REPEATS_FOR_CEILING`].
    ///
    /// `None` is not 0 and not 1: it means the study has not yet measured how
    /// consistent its observers are, so nothing can be said about how well a
    /// metric could possibly do.
    pub ceiling: Option<f64>,
}

/// How well one metric's ordering matches the observers'.
#[derive(Debug, Clone, Serialize)]
pub struct MetricAgreement {
    pub metric: String,
    pub direction: &'static str,
    /// Comparisons where both encodings have a score for this metric AND the
    /// observer expressed a preference (not a tie).
    pub comparisons: usize,
    /// Of those, how many the metric ordered the same way.
    pub agreed: usize,
    /// `agreed / comparisons`. `None` below [`MIN_PAIRS_FOR_RHO`].
    pub rho: Option<f64>,
    /// `rho / ceiling` — the reportable figure. `None` whenever either input is.
    pub rho_over_ceiling: Option<f64>,
    /// Comparisons the observer called a tie where the metric had both scores.
    /// Excluded from `comparisons`; reported because a metric that separates
    /// pairs people cannot tell apart is a real finding about the metric.
    pub ties: usize,
    /// Pairs skipped because one or both encodings had no score for this
    /// metric. The gap between this and `comparisons` is how much of the
    /// collected data the metric can actually be judged on.
    pub uncovered: usize,
}

/// Everything the report needs, in one shot.
#[derive(Debug, Clone, Serialize)]
pub struct Disposition {
    pub study_id: String,
    /// Total comparisons answered in this study.
    pub comparisons: usize,
    /// Distinct encoding pairs seen at least once.
    pub distinct_pairs: usize,
    pub observers: usize,
    pub min_viable_ratings: u32,
    pub ideal_ratings: u32,
    pub ceiling: NoiseCeiling,
    /// Golden-pair pass rate, the attention check. `None` when none served.
    pub golden_pass_rate: Option<f64>,
    pub golden_trials: usize,
    /// One row per ingested metric that has any coverage in this study.
    pub metrics: Vec<MetricAgreement>,
    /// Metrics that were ingested but cannot be scored, and why. Present so the
    /// report can say "ssim2 is not here because nothing is joined" rather than
    /// silently omitting the study's own headline metric.
    pub unusable: Vec<UnusableMetric>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnusableMetric {
    pub metric: String,
    pub reason: String,
}

/// One answered comparison, reduced to what the agreement calculation needs.
struct Comparison {
    a: String,
    b: String,
    /// `Some(true)` if the observer preferred A, `Some(false)` for B, `None`
    /// for a tie.
    prefers_a: Option<bool>,
}

/// (observer, unordered encoding pair) -> every answer they gave for it, each
/// normalised to "first" / "second" of the SORTED pair rather than to the A/B
/// slot it happened to be served in.
///
/// The normalisation is the whole point. `sampling::counterbalance_pair`
/// randomises which encoding lands in slot A, so the same pair comes back with
/// the slots flipped about half the time. Comparing raw `choice` strings would
/// then score a perfectly consistent observer as inconsistent on every flipped
/// repeat — halving the measured noise ceiling and making every rho/ceiling
/// look about twice as good as it is.
type RepeatIndex = std::collections::BTreeMap<(String, (String, String)), Vec<Option<String>>>;

/// One row of the comparison query: (a, b, choice, observer, is_golden,
/// expected_choice, original_choice).
///
/// A named tuple struct rather than the bare 7-tuple, which clippy rejects as
/// "very complex" and which is genuinely unreadable at the destructuring site —
/// two `Option<String>` in a row where swapping them compiles fine and silently
/// scores attention checks against the wrong column. Same reason
/// `exclusion::StudyObs` exists.
#[derive(sqlx::FromRow)]
struct AnsweredPair(
    String,
    String,
    String,
    String,
    i64,
    Option<String>,
    Option<String>,
);

/// Compute the disposition for one study.
pub async fn compute(pool: &SqlitePool, study_id: &str) -> anyhow::Result<Disposition> {
    let study = crate::studies::by_id(study_id)
        .ok_or_else(|| anyhow::anyhow!("unknown study `{study_id}`"))?;

    // The answer that counts is `choice`; the attention checks score
    // `COALESCE(original_choice, choice)` instead, so undo cannot defeat a
    // honeypot. Two different questions, deliberately two different columns.
    let rows: Vec<AnsweredPair> = sqlx::query_as(
        "SELECT t.a_encoding_id, t.b_encoding_id, r.choice, s.observer_id, \
                    t.is_golden, t.expected_choice, r.original_choice \
             FROM trials t \
             JOIN responses r ON r.trial_id = t.id \
             JOIN sessions  s ON s.id = t.session_id \
             WHERE s.study_id = ? AND t.kind = 'pair' \
               AND t.a_encoding_id IS NOT NULL AND t.b_encoding_id IS NOT NULL",
    )
    .bind(study_id)
    .fetch_all(pool)
    .await?;

    let mut comparisons = Vec::new();
    let mut observers = std::collections::BTreeSet::new();
    let mut pairs = std::collections::BTreeSet::new();
    // (observer, unordered pair) -> the answers they gave, in slot-independent
    // form. Counterbalancing randomises which encoding lands in slot A, so
    // comparing raw `choice` strings across two servings of the same pair would
    // score a consistent observer as inconsistent half the time.
    let mut seen: RepeatIndex = std::collections::BTreeMap::new();
    let mut golden_total = 0usize;
    let mut golden_passed = 0usize;

    for AnsweredPair(a, b, choice, observer, is_golden, expected, original) in rows {
        observers.insert(observer.clone());
        let key = if a <= b {
            (a.clone(), b.clone())
        } else {
            (b.clone(), a.clone())
        };
        pairs.insert(key.clone());

        if is_golden == 1 {
            if let Some(exp) = expected.as_deref() {
                golden_total += 1;
                // The FIRST answer is what the attention check scores. An
                // observer who fails a honeypot, notices, and takes it back has
                // still failed it — otherwise undo defeats the control.
                let scored = original.as_deref().unwrap_or(choice.as_str());
                if scored == exp {
                    golden_passed += 1;
                }
            }
        }

        let prefers_a = match choice.as_str() {
            "a" => Some(true),
            "b" => Some(false),
            // Everything else — a tie, a "can't tell", a rating on a single —
            // is not a preference. Treated as a tie rather than dropped so the
            // count is visible.
            _ => None,
        };
        // Normalise to the sorted pair so the repeat check is slot-independent.
        let winner = prefers_a.map(|pa| {
            let picked = if pa { &a } else { &b };
            picked == &key.0
        });
        seen.entry((observer, key)).or_default().push(match winner {
            Some(true) => Some("first".to_string()),
            Some(false) => Some("second".to_string()),
            None => None,
        });

        comparisons.push(Comparison { a, b, prefers_a });
    }

    // Self-agreement: for each (observer, pair) answered more than once, did
    // the answers match? Counted over PAIRS rather than answers, so a pair
    // served three times contributes one observation, not three.
    let mut repeat_pairs = 0usize;
    let mut agreed = 0usize;
    for answers in seen.values() {
        if answers.len() < 2 {
            continue;
        }
        repeat_pairs += 1;
        if answers.windows(2).all(|w| w[0] == w[1]) {
            agreed += 1;
        }
    }
    let ceiling = NoiseCeiling {
        repeat_pairs,
        agreed,
        ceiling: (repeat_pairs >= MIN_REPEATS_FOR_CEILING)
            .then(|| agreed as f64 / repeat_pairs as f64),
    };

    // Metric scores for every encoding this study has served.
    let metric_rows: Vec<(String, String, f64)> = sqlx::query_as(
        "SELECT m.metric, m.encoding_id, m.value FROM encoding_metrics m \
         WHERE EXISTS (SELECT 1 FROM trials t JOIN sessions s ON s.id = t.session_id \
                       WHERE s.study_id = ? \
                         AND (t.a_encoding_id = m.encoding_id OR t.b_encoding_id = m.encoding_id))",
    )
    .bind(study_id)
    .fetch_all(pool)
    .await?;

    let mut by_metric: std::collections::BTreeMap<String, std::collections::HashMap<String, f64>> =
        std::collections::BTreeMap::new();
    for (metric, enc, value) in metric_rows {
        by_metric.entry(metric).or_default().insert(enc, value);
    }

    let mut metrics_out = Vec::new();
    let mut unusable = Vec::new();
    for (metric, scores) in by_metric {
        let dir = direction_of(&metric);
        if dir == Direction::Unknown {
            unusable.push(UnusableMetric {
                metric,
                reason: "direction unknown — correlating it would guess the sign, and a \
                         flipped sign is indistinguishable from the finding this study \
                         is trying to make"
                    .into(),
            });
            continue;
        }

        let mut n = 0usize;
        let mut hit = 0usize;
        let mut ties = 0usize;
        let mut uncovered = 0usize;
        for c in &comparisons {
            let (Some(&sa), Some(&sb)) = (scores.get(&c.a), scores.get(&c.b)) else {
                uncovered += 1;
                continue;
            };
            let Some(prefers_a) = c.prefers_a else {
                ties += 1;
                continue;
            };
            if sa == sb {
                // The metric cannot order this pair. Not agreement, not
                // disagreement — it has no opinion, so it does not belong in
                // the denominator.
                uncovered += 1;
                continue;
            }
            // Direction decides what "better" means. This is the line the
            // Unknown guard above exists to protect.
            let metric_prefers_a = match dir {
                Direction::HigherIsBetter => sa > sb,
                Direction::LowerIsBetter => sa < sb,
                Direction::Unknown => unreachable!("filtered above"),
            };
            n += 1;
            if metric_prefers_a == prefers_a {
                hit += 1;
            }
        }

        let rho = (n >= MIN_PAIRS_FOR_RHO).then(|| hit as f64 / n as f64);
        metrics_out.push(MetricAgreement {
            metric,
            direction: dir.as_str(),
            comparisons: n,
            agreed: hit,
            rho,
            // Both or neither. A ρ printed without its ceiling is the one
            // number this module exists to stop being reported.
            rho_over_ceiling: match (rho, ceiling.ceiling) {
                (Some(r), Some(c)) if c > 0.0 => Some(r / c),
                _ => None,
            },
            ties,
            uncovered,
        });
    }
    metrics_out.sort_by(|a, b| {
        b.rho
            .unwrap_or(-1.0)
            .partial_cmp(&a.rho.unwrap_or(-1.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(Disposition {
        study_id: study_id.to_string(),
        comparisons: comparisons.len(),
        distinct_pairs: pairs.len(),
        observers: observers.len(),
        min_viable_ratings: study.min_viable_ratings,
        ideal_ratings: study.ideal_ratings,
        ceiling,
        golden_pass_rate: (golden_total > 0).then(|| golden_passed as f64 / golden_total as f64),
        golden_trials: golden_total,
        metrics: metrics_out,
        unusable,
    })
}
