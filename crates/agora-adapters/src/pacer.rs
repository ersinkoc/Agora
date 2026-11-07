//! Adaptive detection interval: how long to wait before the next screen check.
//!
//! Visible terminals are checked often so state changes feel instant; hidden ones less often, but never
//! never: detection must keep working for terminals nobody is looking at (a blocked agent in a hidden pane
//! still needs to raise a notification). A terminal whose state has been stable backs off up to a ceiling,
//! and any change snaps it back.

use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pacer {
    pub visible_min: Duration,
    pub hidden_min: Duration,
    pub max: Duration,
    stable_ticks: u32,
}

impl Default for Pacer {
    fn default() -> Self {
        Self {
            visible_min: Duration::from_millis(100),
            hidden_min: Duration::from_millis(500),
            max: Duration::from_secs(2),
            stable_ticks: 0,
        }
    }
}

impl Pacer {
    /// Record the outcome of a check: `changed` is whether the detected state differed from the last one.
    pub fn record(&mut self, changed: bool) {
        self.stable_ticks = if changed {
            0
        } else {
            self.stable_ticks.saturating_add(1)
        };
    }

    /// Time to wait before the next check.
    pub fn interval(&self, visible: bool) -> Duration {
        let floor = if visible {
            self.visible_min
        } else {
            self.hidden_min
        };
        // Double every 4 stable ticks, so a quiet terminal reaches the ceiling within a few seconds.
        let shift = (self.stable_ticks / 4).min(8);
        floor.saturating_mul(1 << shift).min(self.max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_terminals_are_still_checked_just_less_often() {
        let p = Pacer::default();
        assert!(p.interval(false) > p.interval(true));
        assert!(p.interval(false) <= p.max);
    }

    #[test]
    fn stable_terminals_back_off_to_the_ceiling_and_a_change_resets() {
        let mut p = Pacer::default();
        let start = p.interval(true);
        for _ in 0..40 {
            p.record(false);
        }
        assert_eq!(p.interval(true), p.max);
        assert_eq!(p.interval(false), p.max);
        p.record(true);
        assert_eq!(p.interval(true), start);
    }

    #[test]
    fn interval_never_exceeds_the_ceiling() {
        let mut p = Pacer::default();
        for _ in 0..100_000 {
            p.record(false);
        }
        assert_eq!(p.interval(false), p.max);
    }
}
