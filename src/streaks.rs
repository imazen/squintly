//! Day-streak math + badge awarding.
//!
//! A "streak" counts consecutive observer-local calendar days on which the observer
//! contributed at least one trial. The observer's local date is captured per-session
//! from their reported timezone (we don't trust server time for this — a phone
//! observer in Tokyo crossing midnight should see their streak advance, not wait for
//! UTC).
//!
//! Streak freezes save a missed day. Per Duolingo telemetry (-21% churn for at-risk
//! users) we offer one freeze per week, auto-applied when the gap is exactly 2 days.
//!
//! The simple rule:
//! - same date as `streak_last_date` → no change
//! - one calendar day gap → streak advances
//! - two calendar days gap AND `freezes_remaining > 0` → spend a freeze, streak
//!   advances, `freezes_remaining` decremented
//! - otherwise → streak resets to 1
//!
//! Freezes refresh weekly: every Monday (in observer's local TZ) we set
//! `freezes_remaining = max(freezes_remaining, 1)`.

use chrono::NaiveDate;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreakOutcome {
    /// Observer already contributed today. No change.
    SameDay,
    /// Streak advanced by 1.
    Advanced,
    /// Spent a freeze to bridge a single missed day.
    Frozen,
    /// Streak reset to 1.
    Reset,
}

#[derive(Debug, Clone, Copy)]
pub struct StreakState {
    pub streak_days: u32,
    pub freezes_remaining: u32,
    pub last_date: Option<NaiveDate>,
}

pub fn advance_streak(prev: StreakState, today: NaiveDate) -> (StreakState, StreakOutcome) {
    match prev.last_date {
        None => (
            StreakState {
                streak_days: 1,
                freezes_remaining: prev.freezes_remaining,
                last_date: Some(today),
            },
            StreakOutcome::Advanced,
        ),
        Some(last) if last == today => (prev, StreakOutcome::SameDay),
        Some(last) => {
            let gap = (today - last).num_days();
            if gap == 1 {
                (
                    StreakState {
                        streak_days: prev.streak_days + 1,
                        freezes_remaining: prev.freezes_remaining,
                        last_date: Some(today),
                    },
                    StreakOutcome::Advanced,
                )
            } else if gap == 2 && prev.freezes_remaining > 0 {
                (
                    StreakState {
                        streak_days: prev.streak_days + 1,
                        freezes_remaining: prev.freezes_remaining - 1,
                        last_date: Some(today),
                    },
                    StreakOutcome::Frozen,
                )
            } else {
                (
                    StreakState {
                        streak_days: 1,
                        freezes_remaining: prev.freezes_remaining,
                        last_date: Some(today),
                    },
                    StreakOutcome::Reset,
                )
            }
        }
    }
}

/// Trial-count milestone tiers. Crossing one of these on a session should award
/// the matching `first_N` badge.
pub const TRIAL_MILESTONES: &[(u32, &str)] = &[
    (1, "first_trial"),
    (10, "first_10"),
    (50, "first_50"),
    (100, "first_100"),
    (250, "first_250"),
    (500, "first_500"),
    (1000, "first_1000"),
];

pub const STREAK_MILESTONES: &[(u32, &str)] = &[
    (3, "streak_3"),
    (7, "streak_7"),
    (30, "streak_30"),
];

/// Returns the highest crossed milestone slug given prev/new totals.
pub fn crossed_trial_milestone(prev_total: u32, new_total: u32) -> Option<&'static str> {
    TRIAL_MILESTONES
        .iter()
        .rev()
        .find(|(n, _)| prev_total < *n && new_total >= *n)
        .map(|(_, slug)| *slug)
}

pub fn crossed_streak_milestone(prev_streak: u32, new_streak: u32) -> Option<&'static str> {
    STREAK_MILESTONES
        .iter()
        .rev()
        .find(|(n, _)| prev_streak < *n && new_streak >= *n)
        .map(|(_, slug)| *slug)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn first_session_starts_streak_at_one() {
        let (state, out) = advance_streak(
            StreakState {
                streak_days: 0,
                freezes_remaining: 1,
                last_date: None,
            },
            d(2026, 4, 30),
        );
        assert_eq!(state.streak_days, 1);
        assert_eq!(out, StreakOutcome::Advanced);
    }

    #[test]
    fn same_day_does_not_advance() {
        let (state, out) = advance_streak(
            StreakState {
                streak_days: 5,
                freezes_remaining: 1,
                last_date: Some(d(2026, 4, 30)),
            },
            d(2026, 4, 30),
        );
        assert_eq!(state.streak_days, 5);
        assert_eq!(out, StreakOutcome::SameDay);
    }

    #[test]
    fn next_day_advances() {
        let (state, out) = advance_streak(
            StreakState {
                streak_days: 5,
                freezes_remaining: 1,
                last_date: Some(d(2026, 4, 29)),
            },
            d(2026, 4, 30),
        );
        assert_eq!(state.streak_days, 6);
        assert_eq!(state.freezes_remaining, 1);
        assert_eq!(out, StreakOutcome::Advanced);
    }

    #[test]
    fn two_day_gap_with_freeze_bridges() {
        let (state, out) = advance_streak(
            StreakState {
                streak_days: 5,
                freezes_remaining: 1,
                last_date: Some(d(2026, 4, 28)),
            },
            d(2026, 4, 30),
        );
        assert_eq!(state.streak_days, 6);
        assert_eq!(state.freezes_remaining, 0);
        assert_eq!(out, StreakOutcome::Frozen);
    }

    #[test]
    fn two_day_gap_without_freeze_resets() {
        let (state, out) = advance_streak(
            StreakState {
                streak_days: 5,
                freezes_remaining: 0,
                last_date: Some(d(2026, 4, 28)),
            },
            d(2026, 4, 30),
        );
        assert_eq!(state.streak_days, 1);
        assert_eq!(out, StreakOutcome::Reset);
    }

    #[test]
    fn three_day_gap_resets_even_with_freeze() {
        let (state, out) = advance_streak(
            StreakState {
                streak_days: 5,
                freezes_remaining: 1,
                last_date: Some(d(2026, 4, 27)),
            },
            d(2026, 4, 30),
        );
        assert_eq!(state.streak_days, 1);
        assert_eq!(state.freezes_remaining, 1);
        assert_eq!(out, StreakOutcome::Reset);
    }

    #[test]
    fn milestone_crossings() {
        assert_eq!(crossed_trial_milestone(0, 1), Some("first_trial"));
        assert_eq!(crossed_trial_milestone(9, 10), Some("first_10"));
        assert_eq!(crossed_trial_milestone(98, 102), Some("first_100"));
        assert_eq!(crossed_trial_milestone(10, 11), None);
        assert_eq!(crossed_trial_milestone(0, 100), Some("first_100"));
        assert_eq!(crossed_streak_milestone(2, 3), Some("streak_3"));
        assert_eq!(crossed_streak_milestone(6, 8), Some("streak_7"));
    }
}
