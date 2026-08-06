//! Asking an observer about work they have already done.
//!
//! # The problem this solves
//!
//! Almost nobody clicks "End session". They answer some trials and shut the
//! tab. So a debrief attached to the end of a session is a debrief that mostly
//! never happens, and one attached to the `sessions` row is asking about a
//! container the observer never experienced as a unit — that row is opened when
//! the app boots and closed by nothing.
//!
//! What somebody actually experienced is a **bout**: a contiguous run of
//! answers with no long gap. Bouts are computed from `responses.responded_at`,
//! so they exist whether or not anyone signed off, and the prompt moves to the
//! next time they open the app: "last time you did 21 comparisons — anything we
//! should know?" If they DO click End session, the same prompt is raised there
//! instead, where it is immediate rather than recalled.
//!
//! # What is asked, and what is deliberately not
//!
//! A fixed list of **circumstances**, never a self-rating of quality. The
//! difference is which of the two an observer actually knows. "I didn't realise
//! I could answer can't-tell" is a fact about what they understood, and it maps
//! to a concrete analysis: their tie rate is artificially zero and their forced
//! choices on threshold pairs were guesses. "Rate your attention 1–5" is an
//! outcome self-judgement — poorly calibrated in general, and it invites
//! answering whatever seems safest rather than whatever is true.
//!
//! Every reason here is also **checkable against the instruments**. We already
//! record `switch_count`, `dwell_ms`, `ms_on_*`, `zoom_factor` and the golden
//! pass rate, so "I was rushing" either corroborates them or does not. That is
//! what makes accepting a self-report safe: it never has to be trusted blindly.
//!
//! See `docs/OBSERVER-FEEDBACK.md` §7 for the full design.

use crate::db::now_ms;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

/// Gap that ends a bout.
///
/// Deliberately much longer than the leaderboard's `IDLE_CAP_MS` (5 min), which
/// answers a different question. That cap exists so a break is not BILLED; this
/// one decides whether two runs of answers were one sitting in the observer's
/// memory. Somebody who makes a cup of tea and comes back was not in two
/// sittings, and asking them about each half separately would be strange.
pub const BOUT_GAP_MS: i64 = 45 * 60 * 1000;

/// Bouts shorter than this are not worth asking about.
///
/// Three answers is not a sitting anybody has an impression of, and a prompt
/// after it is pure friction — the observer opens the app and is immediately
/// asked to reflect on almost nothing.
pub const MIN_BOUT_RESPONSES: usize = 5;

/// Past this, a self-report is recall rather than observation.
///
/// Somebody asked about an evening three weeks ago will produce something, and
/// what they produce is reconstruction. Better to have no answer than a
/// confident wrong one, so an older bout is never prompted for.
pub const MAX_BOUT_AGE_MS: i64 = 14 * 24 * 60 * 60 * 1000;

/// A reason an observer can select. Fixed list: see the module docs for why it
/// is not free text and not a rating.
pub struct Reason {
    /// Stored value. Stable — analysis keys on it.
    pub key: &'static str,
    /// What the observer reads.
    pub label: &'static str,
    /// What it licenses in analysis. Shown to operators, not to observers.
    pub analysis: &'static str,
}

pub const REASONS: &[Reason] = &[
    Reason {
        key: "missed_cant_tell",
        label: "I didn't realise I could answer \"can't tell\"",
        analysis: "Tie rate is artificially 0; forced choices on threshold pairs were \
                   guesses. Compare their tie rate before and after this bout.",
    },
    Reason {
        key: "learning",
        label: "I was still working out how the task went at first",
        analysis: "A learning effect is real and modelable — truncating from the start of \
                   the bout is defensible in a way that dropping scattered trials is not.",
    },
    Reason {
        key: "conditions",
        label: "Difficult viewing conditions — glare, tiny screen, tired eyes",
        analysis: "Corroborates what the conditions columns already recorded. Check \
                   against ambient_light and viewport size rather than taking it alone.",
    },
    Reason {
        key: "rushed",
        label: "I was rushing or distracted",
        analysis: "Checkable: dwell_ms and switch_count either agree or they do not. A \
                   self-report that disagrees with the instruments is itself informative.",
    },
    Reason {
        key: "other_person",
        label: "Someone else was using my device",
        analysis: "An IDENTITY problem, not a quality one — these responses belong to a \
                   different observer and cannot be pooled with the rest.",
    },
    Reason {
        key: "looked_broken",
        label: "Something looked broken or wouldn't load",
        analysis: "Check the encodings served in this range for corruption before \
                   treating the judgements as being about compression.",
    },
];

pub fn reason_is_known(key: &str) -> bool {
    REASONS.iter().any(|r| r.key == key)
}

/// A stretch of work an observer might be asked about.
#[derive(Debug, Clone, Serialize)]
pub struct Bout {
    pub start_ms: i64,
    pub end_ms: i64,
    pub responses: usize,
    /// Comparisons only, for the sentence shown to the observer — "21
    /// comparisons" is what they remember doing, not "21 responses".
    pub comparisons: usize,
}

/// Everything the debrief prompt needs.
#[derive(Debug, Clone, Serialize)]
pub struct PendingDebrief {
    pub bout: Bout,
    pub reasons: Vec<ReasonOption>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReasonOption {
    pub key: &'static str,
    pub label: &'static str,
}

fn reason_options() -> Vec<ReasonOption> {
    REASONS
        .iter()
        .map(|r| ReasonOption {
            key: r.key,
            label: r.label,
        })
        .collect()
}

/// Split an observer's answers into bouts, newest last.
///
/// Pure so it can be tested without a database — the boundary rule is the whole
/// substance of this module and it deserves to be exercised directly.
pub fn bouts_of(answered: &[(i64, bool)]) -> Vec<Bout> {
    let mut out: Vec<Bout> = Vec::new();
    for &(at, is_pair) in answered {
        match out.last_mut() {
            Some(b) if at - b.end_ms <= BOUT_GAP_MS => {
                b.end_ms = at;
                b.responses += 1;
                if is_pair {
                    b.comparisons += 1;
                }
            }
            _ => out.push(Bout {
                start_ms: at,
                end_ms: at,
                responses: 1,
                comparisons: usize::from(is_pair),
            }),
        }
    }
    out
}

/// The most recent bout this observer has not been asked about, if any.
///
/// Returns `None` — meaning "do not prompt" — when the bout is too short, too
/// old, still in progress, or already has a row (including a skipped one).
/// `include_current` is true when the observer is finishing up right now, so the
/// bout they are in still counts. On a return visit it must be false: prompting
/// about work somebody is still doing is a mid-session interruption, which is
/// the one thing this design exists to avoid.
pub async fn pending(
    pool: &SqlitePool,
    observer_id: &str,
    include_current: bool,
) -> anyhow::Result<Option<PendingDebrief>> {
    let rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT r.responded_at, t.kind FROM responses r \
         JOIN trials t   ON t.id = r.trial_id \
         JOIN sessions s ON s.id = t.session_id \
         WHERE s.observer_id = ? ORDER BY r.responded_at ASC",
    )
    .bind(observer_id)
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok(None);
    }
    let answered: Vec<(i64, bool)> = rows.into_iter().map(|(at, k)| (at, k == "pair")).collect();
    let bouts = bouts_of(&answered);

    let now = now_ms();
    let done: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT bout_start_ms, bout_end_ms FROM session_debriefs WHERE observer_id = ?",
    )
    .bind(observer_id)
    .fetch_all(pool)
    .await?;

    for bout in bouts.iter().rev() {
        if bout.responses < MIN_BOUT_RESPONSES {
            continue;
        }
        if now - bout.end_ms > MAX_BOUT_AGE_MS {
            // Older than this and everything before it is older still.
            break;
        }
        // Still in progress unless the observer is signing off deliberately.
        if !include_current && now - bout.end_ms < BOUT_GAP_MS {
            continue;
        }
        // Overlap rather than equality: the bout's end moves as more answers
        // land, so a debrief taken at "End session" and the same bout seen on a
        // later visit will not have identical bounds. Any overlap means this
        // stretch has been asked about.
        let asked = done
            .iter()
            .any(|&(s, e)| bout.start_ms <= e && s <= bout.end_ms);
        if asked {
            continue;
        }
        return Ok(Some(PendingDebrief {
            bout: bout.clone(),
            reasons: reason_options(),
        }));
    }
    Ok(None)
}

#[derive(Debug, Deserialize)]
pub struct SubmitDebrief {
    pub observer_id: String,
    pub bout_start_ms: i64,
    pub bout_end_ms: i64,
    pub responses: i64,
    #[serde(default)]
    pub reasons: Vec<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub skipped: bool,
    /// `return` or `end`.
    pub prompted_at: String,
}

/// Record a debrief (or a skip).
pub async fn submit(pool: &SqlitePool, req: &SubmitDebrief) -> anyhow::Result<()> {
    // Unknown keys are dropped rather than stored. A key that no analysis knows
    // how to read is not data; keeping it would put a value in the column that
    // silently means nothing, and somebody would later count it.
    let mut keys: Vec<&str> = req
        .reasons
        .iter()
        .map(String::as_str)
        .filter(|k| reason_is_known(k))
        .collect();
    keys.sort_unstable();
    keys.dedup();

    let note = req
        .note
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        // Bounded: this is a free-text box on an anonymous endpoint.
        .map(|s| s.chars().take(2000).collect::<String>());

    let prompted = if req.prompted_at == "end" {
        "end"
    } else {
        "return"
    };

    sqlx::query(
        "INSERT INTO session_debriefs \
         (id, observer_id, bout_start_ms, bout_end_ms, responses, reasons, note, skipped, \
          prompted_at, submitted_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(&req.observer_id)
    .bind(req.bout_start_ms)
    .bind(req.bout_end_ms)
    .bind(req.responses)
    .bind(keys.join(","))
    .bind(note)
    .bind(i64::from(req.skipped))
    .bind(prompted)
    .bind(now_ms())
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60 * 1000;

    #[test]
    fn a_continuous_run_is_one_bout() {
        let answered: Vec<(i64, bool)> = (0..10).map(|i| (i * 2 * MIN, true)).collect();
        let b = bouts_of(&answered);
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].responses, 10);
        assert_eq!(b[0].comparisons, 10);
    }

    #[test]
    fn a_long_gap_starts_a_new_bout() {
        let mut answered: Vec<(i64, bool)> = (0..5).map(|i| (i * 2 * MIN, true)).collect();
        // Next evening.
        answered.extend((0..4).map(|i| (24 * 60 * MIN + i * 2 * MIN, true)));
        let b = bouts_of(&answered);
        assert_eq!(b.len(), 2);
        assert_eq!(b[0].responses, 5);
        assert_eq!(b[1].responses, 4);
    }

    #[test]
    fn a_tea_break_does_not_split_a_sitting() {
        // 20 minutes is a break, not a second session. The leaderboard's 5-minute
        // idle cap answers a different question — what to bill — and using it
        // here would ask somebody about each half of one evening separately.
        let answered = vec![
            (0, true),
            (2 * MIN, true),
            (22 * MIN, true),
            (24 * MIN, true),
        ];
        assert_eq!(bouts_of(&answered).len(), 1);
    }

    #[test]
    fn comparisons_are_counted_separately_from_responses() {
        // The sentence shown to the observer says "21 comparisons", because that
        // is what they remember doing. A 4-tier rating is a response but not a
        // comparison.
        let answered = vec![(0, true), (MIN, false), (2 * MIN, true)];
        let b = bouts_of(&answered);
        assert_eq!(b[0].responses, 3);
        assert_eq!(b[0].comparisons, 2);
    }

    #[test]
    fn every_reason_key_is_unique_and_carries_an_analysis() {
        // The analysis note is what makes a reason worth collecting: a checkbox
        // nobody knows how to read is friction with no payoff.
        let mut keys: Vec<&str> = REASONS.iter().map(|r| r.key).collect();
        let n = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), n, "duplicate reason key");
        for r in REASONS {
            assert!(!r.analysis.is_empty(), "{} has no analysis note", r.key);
            assert!(!r.label.is_empty());
        }
    }

    #[test]
    fn no_reason_asks_the_observer_to_rate_themselves() {
        // Circumstances, never quality. An observer knows whether they realised
        // the tie button existed; they do not know how good their judgements
        // were, and asking invites answering whatever seems safest.
        for r in REASONS {
            let l = r.label.to_ascii_lowercase();
            for banned in ["rate", "how well", "score", "good job", "accurate"] {
                assert!(
                    !l.contains(banned),
                    "{} reads as a self-rating: {}",
                    r.key,
                    r.label
                );
            }
        }
    }
}
