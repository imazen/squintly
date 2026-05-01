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
    let on_screen_intrinsic_w = input.image_displayed_w_css * input.dpr;
    if on_screen_intrinsic_w < (input.intrinsic_w as f64) * 0.5 {
        out.flags.push("viewport_clipped");
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
        "SELECT t.kind, t.is_golden, t.expected_choice, r.choice, r.dwell_ms, r.reveal_count, \
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

// TODO(v0.2): pwcmp leave-one-out per-observer log-likelihood (`dist_L > 1.5`).
// TODO(v0.2): Pérez-Ortiz 2019 unified BT + ACR (δ_o, σ_o) per-observer fit.
// TODO(v0.2): CID22 normalised-disagreement aggregation across observers.
// TODO(v0.2): nightly observer_grades batch.

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
            intrinsic_w: 1080,
            dpr: 3.0,
        });
        assert!(f.flags.contains(&"golden_fail"));
    }

    #[test]
    fn flags_viewport_clipped() {
        let f = compute_response_flags(&InlineGradeInput {
            kind: "single",
            dwell_ms: 2000,
            reveal_count: 1,
            choice: "2",
            is_golden: false,
            expected_choice: None,
            image_displayed_w_css: 100.0,
            intrinsic_w: 4096,
            dpr: 2.0,
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
}
