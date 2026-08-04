//! Participant grading & outlier management.
//!
//! v0.1 scope: per-trial inline flags + session-end aggregate grade. The
//! cross-session pwcmp leave-one-out fit, Pérez-Ortiz 2019 (δ_o, σ_o) per-observer
//! ACR fit, and CID22 normalised-disagreement aggregation are v0.2 batch jobs and
//! live in TODOs at the bottom of this file.
//!
//! Citations:
//! - CID22 (Sneyers et al. 2024): first-3 discard, default-button trap, golden-fail rules.
//! - KonIQ-10k (Lin/Hosu/Saupe 2018): 70% golden pass floor, line-clicker ratio 2.0.
//! - Meade & Craig 2012: even-odd consistency, RT floor as soft signal, IMC pattern.
//! - pwcmp (Pérez-Ortiz & Mantiuk): leave-one-out IQR-normalised flagging at dist_L > 1.5.
//! - BT.500-14 §A.1: β₂ kurtosis-based subject screening (4-tier ACR sanity check only).
//!
//! See `docs/participant-grading.md`.

use std::collections::HashMap;

use anyhow::Result;
use sqlx::Row;
use sqlx::SqlitePool;

use crate::db::now_ms;

/// Per-trial flags, computed when a response is recorded.
#[derive(Debug, Default, Clone)]
pub struct ResponseFlags {
    pub flags: Vec<&'static str>,
}

impl ResponseFlags {
    pub fn join(&self) -> Option<String> {
        if self.flags.is_empty() {
            None
        } else {
            Some(self.flags.join(","))
        }
    }
}

pub struct InlineGradeInput<'a> {
    pub kind: &'a str, // "single" | "pair"
    pub dwell_ms: i64,
    pub reveal_count: i64,
    pub choice: &'a str,
    pub is_golden: bool,
    pub expected_choice: Option<&'a str>,
    pub image_displayed_w_css: f64,
    pub intrinsic_w: i64,
    pub dpr: f64,
    /// Visible slice of the stimulus, and whether the observer explored it.
    /// The stimulus renders at a hard 1:1 device pixels and is panned rather
    /// than shrunk, so "could they see what they were rating" is a question
    /// about the visible *area*, not about display scale.
    pub image_displayed_h_css: f64,
    pub visible_w_css: f64,
    pub visible_h_css: f64,
    pub pan_count: i64,
}

pub fn compute_response_flags(input: &InlineGradeInput<'_>) -> ResponseFlags {
    let mut out = ResponseFlags::default();

    let rt_floor = if input.kind == "pair" { 600 } else { 800 };
    if input.dwell_ms < rt_floor {
        out.flags.push("rt_too_fast");
    }
    if input.dwell_ms > 60_000 {
        out.flags.push("rt_too_slow");
    }
    if input.kind == "pair" && input.reveal_count == 0 {
        out.flags.push("no_reveal");
    }
    if input.is_golden {
        if let Some(expected) = input.expected_choice {
            if expected != input.choice {
                out.flags.push("golden_fail");
            }
        }
    }
    // `viewport_clipped` used to mean "the browser shrank this below half its
    // intrinsic width". Under mandatory 1:1 display that can never happen —
    // displayed_w * dpr == intrinsic_w by construction — so the old test was
    // dead code and the gate silently passed everything.
    //
    // The concern it encoded is still real, just relocated: an oversized
    // stimulus is now only partly on screen. Flag a response where the observer
    // saw less than half the stimulus AND never dragged to see the rest — they
    // rated a crop they did not choose. Panning at all clears it; exploring is
    // the intended behaviour, not a defect.
    if input.image_displayed_w_css > 0.0 && input.image_displayed_h_css > 0.0 {
        let seen_w = (input.visible_w_css / input.image_displayed_w_css).clamp(0.0, 1.0);
        let seen_h = (input.visible_h_css / input.image_displayed_h_css).clamp(0.0, 1.0);
        if seen_w * seen_h < 0.5 && input.pan_count == 0 {
            out.flags.push("viewport_clipped");
        }
    }
    out
}

/// Hard-gate signals that should immediately terminate a session. Computed from
/// the most recent N responses.
#[derive(Debug, Default)]
pub struct HardGate {
    pub default_button_fast_rate: f32,
    pub consecutive_golden_fails: u32,
    pub mobile_desktop_mismatch: bool,
}

impl HardGate {
    pub fn should_terminate(&self) -> bool {
        self.default_button_fast_rate > 0.20
            || self.consecutive_golden_fails >= 3
            || self.mobile_desktop_mismatch
    }
}

/// Aggregate one session's responses into the sessions row's grading columns.
/// Called at session-end. Drops the first 3 trials per CID22.
pub async fn grade_session(pool: &SqlitePool, session_id: &str) -> Result<SessionGrade> {
    let rows = sqlx::query(
        // `COALESCE(r.original_choice, r.choice)` — attention checks score the
        // FIRST answer. Otherwise undo defeats the honeypot: fail it, notice,
        // take it back. Ordinary trials are unaffected, since `original_choice`
        // is NULL unless a revision happened.
        "SELECT t.kind, t.is_golden, t.expected_choice, \
                COALESCE(r.original_choice, r.choice), r.dwell_ms, r.reveal_count, \
                r.response_flags, t.served_at \
         FROM trials t JOIN responses r ON r.trial_id = t.id \
         WHERE t.session_id = ? \
         ORDER BY t.served_at",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        return Ok(SessionGrade::default());
    }

    let mut g = SessionGrade {
        n_trials: rows.len() as i64,
        ..SessionGrade::default()
    };

    let post_warmup: Vec<_> = rows.iter().skip(3).collect();

    let mut goldens_total = 0i64;
    let mut goldens_pass = 0i64;
    let mut rt_below = 0i64;
    let mut no_reveal = 0i64;
    let mut pair_count = 0i64;
    let mut single_choices = Vec::<i32>::new();
    let mut button_counts: HashMap<String, i64> = HashMap::new();
    let mut max_streak = 0i64;
    let mut cur_streak = 0i64;
    let mut last_choice: Option<String> = None;

    for row in &post_warmup {
        let kind: String = row.get(0);
        let is_golden: i64 = row.get(1);
        let expected: Option<String> = row.get(2);
        let choice: String = row.get(3);
        let dwell_ms: i64 = row.get(4);
        let reveal_count: i64 = row.get(5);

        if is_golden == 1 {
            goldens_total += 1;
            if expected.as_deref() == Some(choice.as_str()) {
                goldens_pass += 1;
            }
        }
        let rt_floor = if kind == "pair" { 600 } else { 800 };
        if dwell_ms < rt_floor {
            rt_below += 1;
        }
        if kind == "pair" {
            pair_count += 1;
            if reveal_count == 0 {
                no_reveal += 1;
            }
        }
        if kind == "single" {
            if let Ok(v) = choice.parse::<i32>() {
                single_choices.push(v);
            }
        }
        *button_counts.entry(choice.clone()).or_insert(0) += 1;
        if last_choice.as_deref() == Some(choice.as_str()) {
            cur_streak += 1;
        } else {
            cur_streak = 1;
        }
        max_streak = max_streak.max(cur_streak);
        last_choice = Some(choice);
    }

    g.golden_pass_rate = if goldens_total > 0 {
        Some(goldens_pass as f32 / goldens_total as f32)
    } else {
        None
    };
    g.rt_below_floor_count = rt_below;
    g.no_reveal_count = no_reveal;
    g.n_pair_trials = pair_count;
    g.straight_line_max = max_streak;

    // KonIQ line-clicker ratio: max_button_count / sum_of_others
    let max_count = *button_counts.values().max().unwrap_or(&0);
    let other_sum: i64 = button_counts.values().sum::<i64>() - max_count;
    g.straight_line_ratio = if other_sum > 0 {
        Some(max_count as f32 / other_sum as f32)
    } else {
        Some(f32::INFINITY)
    };

    // Even-odd Spearman on 4-tier choices (proxy: Pearson r since the tiers are
    // already an ordinal scale of 1..4).
    if single_choices.len() >= 8 {
        let evens: Vec<f64> = single_choices
            .iter()
            .step_by(2)
            .map(|&v| v as f64)
            .collect();
        let odds: Vec<f64> = single_choices
            .iter()
            .skip(1)
            .step_by(2)
            .map(|&v| v as f64)
            .collect();
        let n = evens.len().min(odds.len());
        let r = pearson(&evens[..n], &odds[..n]);
        g.even_odd_r = r.map(|r| r as f32);
    }

    let weight = composite_weight(&g);
    g.session_weight = weight;
    g.session_grade = grade_letter(weight).to_string();

    sqlx::query(
        "UPDATE sessions SET session_grade = ?, session_weight = ?, golden_pass_rate = ?, \
         straight_line_max = ?, straight_line_ratio = ?, rt_below_floor_count = ?, \
         no_reveal_count = ?, even_odd_r = ?, n_trials = ?, n_pair_trials = ?, graded_at = ? \
         WHERE id = ?",
    )
    .bind(&g.session_grade)
    .bind(g.session_weight)
    .bind(g.golden_pass_rate)
    .bind(g.straight_line_max)
    .bind(g.straight_line_ratio)
    .bind(g.rt_below_floor_count)
    .bind(g.no_reveal_count)
    .bind(g.even_odd_r)
    .bind(g.n_trials)
    .bind(g.n_pair_trials)
    .bind(now_ms())
    .bind(session_id)
    .execute(pool)
    .await?;

    Ok(g)
}

#[derive(Debug, Default, Clone)]
pub struct SessionGrade {
    pub n_trials: i64,
    pub n_pair_trials: i64,
    pub golden_pass_rate: Option<f32>,
    pub straight_line_max: i64,
    pub straight_line_ratio: Option<f32>,
    pub rt_below_floor_count: i64,
    pub no_reveal_count: i64,
    pub even_odd_r: Option<f32>,
    pub session_weight: f32,
    pub session_grade: String,
}

fn composite_weight(g: &SessionGrade) -> f32 {
    let golden_score = match g.golden_pass_rate {
        Some(r) if r >= 0.70 => 1.0,
        Some(r) => ((r - 0.40) / 0.30).clamp(0.0, 1.0),
        None => 0.7, // no goldens: cap at C-grade
    };
    let line_score = match g.straight_line_ratio {
        Some(r) if r <= 1.5 => 1.0,
        Some(r) if r.is_finite() => ((2.5 - r) / 1.0).clamp(0.0, 1.0),
        Some(_) => 0.0,
        None => 1.0,
    };
    let rt_floor_frac = if g.n_trials > 0 {
        g.rt_below_floor_count as f32 / g.n_trials as f32
    } else {
        0.0
    };
    let rt_score = if rt_floor_frac <= 0.10 {
        1.0
    } else {
        ((0.30 - rt_floor_frac) / 0.20).clamp(0.0, 1.0)
    };
    let even_odd_score = match g.even_odd_r {
        Some(r) => ((r - 0.10) / 0.40).clamp(0.0, 1.0),
        None => 0.8, // not enough 4-tier trials to compute: don't punish
    };
    let no_reveal_score = if g.n_pair_trials >= 3 {
        let frac = g.no_reveal_count as f32 / g.n_pair_trials as f32;
        if frac <= 0.20 {
            1.0
        } else {
            ((0.50 - frac) / 0.30).clamp(0.0, 1.0)
        }
    } else {
        1.0
    };
    let parts = [
        golden_score,
        line_score,
        rt_score,
        even_odd_score,
        no_reveal_score,
    ];
    // Geometric mean — any zero zeroes the weight, by design (Meade & Craig: any one
    // sub-score sufficient to flag a session is itself flagging).
    let prod: f32 = parts.iter().product();
    prod.powf(1.0 / parts.len() as f32)
}

fn grade_letter(weight: f32) -> &'static str {
    if weight >= 0.85 {
        "A"
    } else if weight >= 0.70 {
        "B"
    } else if weight >= 0.50 {
        "C"
    } else if weight >= 0.25 {
        "D"
    } else {
        "F"
    }
}

fn pearson(x: &[f64], y: &[f64]) -> Option<f64> {
    if x.len() < 2 {
        return None;
    }
    let n = x.len() as f64;
    let mx = x.iter().sum::<f64>() / n;
    let my = y.iter().sum::<f64>() / n;
    let mut num = 0.0;
    let mut dx = 0.0;
    let mut dy = 0.0;
    for (a, b) in x.iter().zip(y.iter()) {
        let xa = a - mx;
        let yb = b - my;
        num += xa * yb;
        dx += xa * xa;
        dy += yb * yb;
    }
    let denom = (dx * dy).sqrt();
    if denom < 1e-12 {
        None
    } else {
        Some(num / denom)
    }
}

/// Rebuild every observer's `observer_grades` row from their session
/// history. Idempotent — wipes the table first, then re-inserts. Runs
/// nightly from a background task (see `main.rs::spawn_grading_batch`)
/// and can be triggered manually via the admin endpoint.
///
/// Aggregates across each observer's graded sessions:
/// - `n_sessions`, `n_trials` — totals.
/// - `golden_pass_rate` — trial-weighted mean (each session's rate weighted
///   by its `n_trials`, so a long lousy session doesn't get drowned by a
///   short clean session).
/// - `even_odd_r` — session-count-weighted mean (each session has equal
///   evidence about within-session consistency, regardless of trial count).
/// - `weight` — geometric mean of session weights (matches the per-session
///   composite_weight aggregation — any one bad session drags hard,
///   matching the "any one signal can flag" philosophy).
/// - `quality_grade` — derived from `weight` via `grade_letter`.
///
/// Promotes observers to the trusted pool when `weight ≥ 0.70`
/// (grade ≥ B) AND `n_trials ≥ 50`, demotes otherwise. The 50-trial
/// floor is what keeps a single A-grade session from instantly trusting
/// a brand-new observer.
///
/// The pwcmp leave-one-out log-likelihood (`pwcmp_log_lik` / `pwcmp_dist_l`)
/// and CID22 normalised-disagreement (`cid22_*`) columns are left NULL
/// until those fits land — they need a global BT solve over all observers'
/// pair data and are tracked separately.
pub async fn rebuild_observer_grades(pool: &SqlitePool) -> Result<u64> {
    let now = now_ms();

    // Pull every graded session (those have a non-NULL `graded_at`). The
    // session_weight column is the geometric-mean composite already
    // computed by `grade_session`.
    let rows = sqlx::query(
        "SELECT observer_id, n_trials, session_weight, golden_pass_rate, even_odd_r \
         FROM sessions \
         WHERE graded_at IS NOT NULL",
    )
    .fetch_all(pool)
    .await?;

    #[derive(Default)]
    struct Accum {
        n_sessions: i64,
        n_trials: i64,
        weight_log_sum: f64,
        weight_log_n: i64,
        golden_num: f64,
        golden_den: i64,
        even_odd_sum: f64,
        even_odd_n: i64,
    }
    let mut acc: HashMap<String, Accum> = HashMap::new();
    for row in rows {
        let obs: String = row.get(0);
        let n_trials: i64 = row.get(1);
        let weight: f32 = row.get(2);
        let golden: Option<f32> = row.get(3);
        let even_odd: Option<f32> = row.get(4);
        let a = acc.entry(obs).or_default();
        a.n_sessions += 1;
        a.n_trials += n_trials;
        // log(0) → -∞ shorts the geometric mean to 0; clamp at 1e-6 so a
        // single F-grade session doesn't zero an observer's lifetime mean.
        let w = (weight as f64).max(1e-6);
        a.weight_log_sum += w.ln();
        a.weight_log_n += 1;
        if let Some(r) = golden {
            a.golden_num += (r as f64) * (n_trials as f64);
            a.golden_den += n_trials;
        }
        if let Some(r) = even_odd {
            a.even_odd_sum += r as f64;
            a.even_odd_n += 1;
        }
    }

    // Idempotent: wipe and re-insert. Cheap on the squintly scale (≤ 10⁴
    // observers expected lifetime) and avoids drift if observer ids ever
    // get deleted upstream.
    sqlx::query("DELETE FROM observer_grades")
        .execute(pool)
        .await?;

    let mut written = 0u64;
    for (observer_id, a) in &acc {
        let weight = if a.weight_log_n > 0 {
            (a.weight_log_sum / a.weight_log_n as f64).exp() as f32
        } else {
            0.0
        };
        let golden = if a.golden_den > 0 {
            Some((a.golden_num / a.golden_den as f64) as f32)
        } else {
            None
        };
        let even_odd = if a.even_odd_n > 0 {
            Some((a.even_odd_sum / a.even_odd_n as f64) as f32)
        } else {
            None
        };
        let grade = grade_letter(weight);

        sqlx::query(
            "INSERT INTO observer_grades (observer_id, computed_at, n_trials, n_sessions, \
             quality_grade, weight, golden_pass_rate, even_odd_r) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(observer_id)
        .bind(now)
        .bind(a.n_trials)
        .bind(a.n_sessions)
        .bind(grade)
        .bind(weight)
        .bind(golden)
        .bind(even_odd)
        .execute(pool)
        .await?;

        // Promotion / demotion. The 50-trial floor stops a single A-grade
        // session from trusting a brand-new observer; the B-grade weight
        // threshold matches `grade_letter`'s tier.
        let trusted = if weight >= 0.70 && a.n_trials >= 50 {
            1
        } else {
            0
        };
        sqlx::query("UPDATE observers SET trusted_pool = ? WHERE id = ?")
            .bind(trusted)
            .bind(observer_id)
            .execute(pool)
            .await?;
        written += 1;
    }
    Ok(written)
}

// TODO(v0.2): pwcmp leave-one-out per-observer log-likelihood (`dist_L > 1.5`).
// TODO(v0.2): Pérez-Ortiz 2019 unified BT + ACR (δ_o, σ_o) per-observer fit.
// TODO(v0.2): CID22 normalised-disagreement aggregation across observers.
// rebuild_observer_grades above ships the nightly aggregation that closes
// amplifier #9's primary blocker; the LOO and per-observer ACR fits feed
// the `pwcmp_*` and `sigma_acr` / `delta_acr` columns once they land.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_fast_pair_with_no_reveal() {
        let f = compute_response_flags(&InlineGradeInput {
            kind: "pair",
            dwell_ms: 400,
            reveal_count: 0,
            choice: "a",
            is_golden: false,
            expected_choice: None,
            image_displayed_w_css: 360.0,
            image_displayed_h_css: 360.0,
            visible_w_css: 360.0,
            visible_h_css: 360.0,
            pan_count: 0,
            intrinsic_w: 1080,
            dpr: 3.0,
        });
        assert!(f.flags.contains(&"rt_too_fast"));
        assert!(f.flags.contains(&"no_reveal"));
    }

    #[test]
    fn flags_golden_mismatch() {
        let f = compute_response_flags(&InlineGradeInput {
            kind: "single",
            dwell_ms: 1500,
            reveal_count: 1,
            choice: "1",
            is_golden: true,
            expected_choice: Some("4"),
            image_displayed_w_css: 360.0,
            image_displayed_h_css: 360.0,
            visible_w_css: 360.0,
            visible_h_css: 360.0,
            pan_count: 0,
            intrinsic_w: 1080,
            dpr: 3.0,
        });
        assert!(f.flags.contains(&"golden_fail"));
    }

    #[test]
    fn flags_viewport_clipped() {
        // Oversized stimulus, only a sliver on screen, and the observer never
        // dragged: they rated a crop they did not choose.
        let base = |visible_w: f64, pan: i64| InlineGradeInput {
            kind: "single",
            dwell_ms: 2000,
            reveal_count: 1,
            choice: "2",
            is_golden: false,
            expected_choice: None,
            image_displayed_w_css: 800.0,
            image_displayed_h_css: 600.0,
            visible_w_css: visible_w,
            visible_h_css: 600.0,
            pan_count: pan,
            intrinsic_w: 2400,
            dpr: 3.0,
        };
        assert!(
            compute_response_flags(&base(300.0, 0))
                .flags
                .contains(&"viewport_clipped")
        );
        // Panning clears it — exploring is the intended behaviour under the
        // 1:1 display rule, not a defect.
        assert!(
            !compute_response_flags(&base(300.0, 3))
                .flags
                .contains(&"viewport_clipped")
        );
        // Fully visible stimulus is never flagged.
        assert!(
            !compute_response_flags(&base(800.0, 0))
                .flags
                .contains(&"viewport_clipped")
        );
    }

    /// Under mandatory 1:1 the OLD rule (displayed_w * dpr < 0.5 * intrinsic_w)
    /// can never fire, because displayed_w * dpr == intrinsic_w by
    /// construction. Guard that the flag still means something.
    #[test]
    fn viewport_clipped_is_not_dead_under_1to1_display() {
        let intrinsic_w: f64 = 2400.0;
        let dpr: f64 = 3.0;
        let displayed_w = intrinsic_w / dpr; // what trial.ts now always sets
        assert!(
            (displayed_w * dpr - intrinsic_w).abs() < 0.001,
            "1:1 display means the old scale-based test is vacuous"
        );
        let f = compute_response_flags(&InlineGradeInput {
            kind: "single",
            dwell_ms: 2000,
            reveal_count: 1,
            choice: "2",
            is_golden: false,
            expected_choice: None,
            image_displayed_w_css: displayed_w,
            image_displayed_h_css: 600.0,
            visible_w_css: 288.0, // a phone-width slice of an 800px-wide stimulus
            visible_h_css: 600.0,
            pan_count: 0,
            intrinsic_w: intrinsic_w as i64,
            dpr,
        });
        assert!(f.flags.contains(&"viewport_clipped"));
    }

    #[test]
    fn composite_weight_geometric_mean_zeroes_on_one_zero() {
        let g = SessionGrade {
            n_trials: 30,
            n_pair_trials: 10,
            golden_pass_rate: Some(0.10), // → 0
            straight_line_ratio: Some(1.0),
            rt_below_floor_count: 0,
            no_reveal_count: 0,
            even_odd_r: Some(0.6),
            ..Default::default()
        };
        let w = composite_weight(&g);
        assert!(w < 0.05, "got {w}");
    }

    #[tokio::test]
    async fn rebuild_observer_grades_aggregates_sessions_and_sets_trust() -> Result<()> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(2)
            .connect("sqlite::memory:")
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;

        // Two observers: alice is strong + has enough trials; bob is weak.
        for obs in ["alice", "bob"] {
            sqlx::query("INSERT INTO observers (id, created_at) VALUES (?, ?)")
                .bind(obs)
                .bind(now_ms())
                .execute(&pool)
                .await?;
        }
        // Helper: insert a graded session with the given weight and trial count.
        let insert_session = |obs: &'static str,
                              id: &'static str,
                              weight: f32,
                              n_trials: i64,
                              golden: Option<f32>,
                              even_odd: Option<f32>| {
            let pool = pool.clone();
            async move {
                sqlx::query(
                    "INSERT INTO sessions (id, observer_id, started_at, device_pixel_ratio, \
                     screen_width_css, screen_height_css, session_weight, session_grade, \
                     golden_pass_rate, even_odd_r, n_trials, graded_at) \
                     VALUES (?, ?, ?, 3.0, 390, 844, ?, ?, ?, ?, ?, ?)",
                )
                .bind(id)
                .bind(obs)
                .bind(now_ms())
                .bind(weight)
                .bind(grade_letter(weight))
                .bind(golden)
                .bind(even_odd)
                .bind(n_trials)
                .bind(now_ms())
                .execute(&pool)
                .await
            }
        };
        insert_session("alice", "s_a1", 0.90, 30, Some(0.95), Some(0.6)).await?;
        insert_session("alice", "s_a2", 0.85, 40, Some(0.90), Some(0.7)).await?;
        insert_session("bob", "s_b1", 0.30, 12, Some(0.35), Some(0.0)).await?;

        let written = rebuild_observer_grades(&pool).await?;
        assert_eq!(written, 2);

        let alice: (i64, i64, String, f32, Option<f32>) = sqlx::query_as(
            "SELECT n_sessions, n_trials, quality_grade, weight, golden_pass_rate \
             FROM observer_grades WHERE observer_id = 'alice'",
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(alice.0, 2);
        assert_eq!(alice.1, 70);
        assert_eq!(alice.2, "A");
        assert!(alice.3 > 0.80, "alice weight: {}", alice.3);
        let alice_golden = alice.4.unwrap();
        // trial-weighted mean: (0.95·30 + 0.90·40) / 70 ≈ 0.921
        assert!(
            (alice_golden - 0.921).abs() < 0.01,
            "alice golden: {alice_golden}"
        );

        let bob: (i64, i64, String, f32) = sqlx::query_as(
            "SELECT n_sessions, n_trials, quality_grade, weight \
             FROM observer_grades WHERE observer_id = 'bob'",
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(bob.0, 1);
        assert_eq!(bob.1, 12);
        assert_eq!(bob.2, "D");

        // Trust pool: alice gets in (B+ AND ≥50 trials), bob does not.
        let alice_trusted: i64 =
            sqlx::query_as::<_, (i64,)>("SELECT trusted_pool FROM observers WHERE id = 'alice'")
                .fetch_one(&pool)
                .await?
                .0;
        let bob_trusted: i64 =
            sqlx::query_as::<_, (i64,)>("SELECT trusted_pool FROM observers WHERE id = 'bob'")
                .fetch_one(&pool)
                .await?
                .0;
        assert_eq!(alice_trusted, 1);
        assert_eq!(bob_trusted, 0);

        // Idempotent: a second rebuild produces the same row count, no
        // duplicates from the UNIQUE PK on observer_id.
        let again = rebuild_observer_grades(&pool).await?;
        assert_eq!(again, 2);
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM observer_grades")
            .fetch_one(&pool)
            .await?;
        assert_eq!(count.0, 2);
        Ok(())
    }
}
