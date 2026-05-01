//! Transformed up–down adaptive staircase (Levitt 1971), one per
//! (session, source, codec, threshold-target).
//!
//! - `3down1up` converges on P=0.794 (used for `q_notice`)
//! - `2down1up` converges on P=0.707 (used for `q_dislike`)
//! - `1down1up` converges on P=0.500 (used for `q_hate`)
//!
//! Direction convention: "down" means *increase distortion* (lower q), since lower q
//! makes the stimulus easier to detect/dislike. We track reversals on the sign of the
//! step direction; step size halves on each reversal until reaching `min_step`, then
//! stays fixed. Convergence = N consecutive reversals at min_step.

use serde::{Deserialize, Serialize};

#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Target {
    Notice,
    Dislike,
    Hate,
}

impl Target {
    pub fn rule(self) -> Rule {
        match self {
            Self::Notice => Rule::ThreeDownOneUp,
            Self::Dislike => Rule::TwoDownOneUp,
            Self::Hate => Rule::OneDownOneUp,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Notice => "notice",
            Self::Dislike => "dislike",
            Self::Hate => "hate",
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Rule {
    OneDownOneUp,
    TwoDownOneUp,
    ThreeDownOneUp,
}

impl Rule {
    fn down_count(self) -> u8 {
        match self {
            Self::OneDownOneUp => 1,
            Self::TwoDownOneUp => 2,
            Self::ThreeDownOneUp => 3,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OneDownOneUp => "1down1up",
            Self::TwoDownOneUp => "2down1up",
            Self::ThreeDownOneUp => "3down1up",
        }
    }
}

/// 4-tier ACR rating.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Rating {
    Imperceptible = 1,
    Notice = 2,
    Dislike = 3,
    Hate = 4,
}

impl Rating {
    pub fn from_choice(s: &str) -> Option<Self> {
        match s {
            "1" => Some(Self::Imperceptible),
            "2" => Some(Self::Notice),
            "3" => Some(Self::Dislike),
            "4" => Some(Self::Hate),
            _ => None,
        }
    }

    /// Returns true if the response counts as a "hit" for this threshold target —
    /// i.e., the observer reached the rating that defines the threshold or worse.
    pub fn meets(self, target: Target) -> bool {
        match target {
            Target::Notice => self as u8 >= Rating::Notice as u8,
            Target::Dislike => self as u8 >= Rating::Dislike as u8,
            Target::Hate => self as u8 >= Rating::Hate as u8,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Staircase {
    pub target: Target,
    pub rule: Rule,
    /// Quality grid (sorted ascending), as exposed by coefficient for this codec.
    pub grid: Vec<f32>,
    /// Index into `grid` for the next stimulus.
    pub idx: usize,
    /// Step in grid indices; halves on reversal.
    pub step: usize,
    /// Pending consecutive "down" hits required before the next decrease.
    pending_down: u8,
    /// Direction of the last move (+1 = q increased, -1 = q decreased, 0 = none).
    last_dir: i8,
    pub reversals: u32,
    /// Quality values at each reversal (for averaging the converged threshold).
    pub reversal_qs: Vec<f32>,
    pub min_step: usize,
    pub target_reversals: u32,
    pub converged: bool,
}

impl Staircase {
    pub fn new(target: Target, grid: Vec<f32>) -> Self {
        let rule = target.rule();
        // Start at the "easiest" (highest q) end so most observers see the imperceptible
        // case first; reduces frustration and gives the staircase a clean ramp-down.
        let idx = grid.len().saturating_sub(1);
        let step = (grid.len() / 8).max(2);
        Self {
            target,
            rule,
            grid,
            idx,
            step,
            pending_down: rule.down_count(),
            last_dir: 0,
            reversals: 0,
            reversal_qs: Vec::new(),
            min_step: 1,
            target_reversals: 8,
            converged: false,
        }
    }

    pub fn current_q(&self) -> f32 {
        self.grid[self.idx]
    }

    /// Update the staircase given a 4-tier rating; returns the next quality to test
    /// (or `None` if converged).
    pub fn step(&mut self, rating: Rating) -> Option<f32> {
        if self.converged {
            return None;
        }
        let met = rating.meets(self.target);
        let mut new_dir = 0i8;

        if met {
            // observer noticed/disliked/hated — go EASIER (increase q) but only after
            // 1 trial (the "1up" half of the rule).
            self.pending_down = self.rule.down_count();
            let next = (self.idx + self.step).min(self.grid.len() - 1);
            if next != self.idx {
                new_dir = 1;
                self.idx = next;
            }
        } else {
            // observer rated "imperceptible" (or below threshold) — count down.
            self.pending_down = self.pending_down.saturating_sub(1);
            if self.pending_down == 0 {
                self.pending_down = self.rule.down_count();
                let next = self.idx.saturating_sub(self.step);
                if next != self.idx {
                    new_dir = -1;
                    self.idx = next;
                }
            }
        }

        // Reversal detection
        if new_dir != 0 && self.last_dir != 0 && new_dir != self.last_dir {
            self.reversals += 1;
            self.reversal_qs.push(self.current_q());
            // Halve step size, floor at min_step
            self.step = (self.step / 2).max(self.min_step);
            if self.reversals >= self.target_reversals && self.step == self.min_step {
                self.converged = true;
                return None;
            }
        }
        if new_dir != 0 {
            self.last_dir = new_dir;
        }

        Some(self.current_q())
    }

    /// Mean of the last `n` reversal qualities (standard threshold estimator).
    pub fn estimate(&self) -> Option<f32> {
        if self.reversal_qs.len() < 2 {
            return None;
        }
        let n = self.reversal_qs.len();
        // Drop the first reversal (often biased by the starting point).
        let take_from = if n >= 6 { n - 6 } else { 1 };
        let slice = &self.reversal_qs[take_from..];
        let mean = slice.iter().sum::<f32>() / slice.len() as f32;
        Some(mean)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staircase_converges_in_synthetic_run() {
        // Synthetic observer: notices when q < 50.
        let grid: Vec<f32> = (0..=100).step_by(5).map(|q| q as f32).collect();
        let mut sc = Staircase::new(Target::Notice, grid);
        sc.target_reversals = 6;
        for _ in 0..200 {
            let q = sc.current_q();
            let rating = if q < 50.0 {
                Rating::Notice
            } else {
                Rating::Imperceptible
            };
            if sc.step(rating).is_none() {
                break;
            }
        }
        assert!(sc.converged, "should converge");
        let est = sc.estimate().expect("estimate after convergence");
        // 3-down-1-up biases above 50% noticing — i.e., higher q at threshold.
        assert!((est - 50.0).abs() <= 10.0, "got {est}");
    }
}
