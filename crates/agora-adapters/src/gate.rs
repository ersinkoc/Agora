//! ADR-0117 decision 1: the timing gate between a screen reading and the agent's state. A redraw that shows the input box for a moment
//! between two tool calls must not read as idle, and a dialog being redrawn must not read as answered. So, per kind (`[detection]`):
//!
//! - **Idle is confirmed.** While the state is `working`, a reading of `idle` (or `done`) is applied only when every reading for
//!   `idle_confirm` read the same; any other reading drops it.
//! - **Blocked is kept.** While the state is `blocked`, another reading is applied only when no reading was `blocked` for
//!   `blocked_hold`. The person's input to the terminal ends the hold at once ([`Gate::input`]), so an answer shows at once.
//! - Everything else (a reading of `working` or `blocked`, anything from a state at rest) is applied at once, as without the gate.
//!
//! Pure: the clock is passed in, no I/O. The node's facts and an agent's own reports bypass it ([`Gate::clear`]).

use std::time::{Duration, Instant};

use agora_core::AgentState;

/// A kind's `[detection]` timings; zero is "no gate" for that direction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Timing {
    pub idle_confirm: Duration,
    pub blocked_hold: Duration,
}

/// The gate's memory for one agent.
#[derive(Debug, Clone, Default)]
pub struct Gate {
    /// A rest state read while working, and since when every reading read it.
    pending: Option<(AgentState, Instant)>,
    /// While blocked: since when every reading was something else (no blocked rule matched); `None` while the dialog shows.
    hold_since: Option<Instant>,
    /// While blocked: a reading other than blocked is waiting for the hold to end.
    holding: bool,
    /// The person's input since the last reading: the hold ends at the next one.
    released: bool,
}

impl Gate {
    /// Offers one screen reading against the current state; `Some` is the state to apply now.
    pub fn offer(
        &mut self,
        current: AgentState,
        reading: AgentState,
        now: Instant,
        t: &Timing,
    ) -> Option<AgentState> {
        if reading == current {
            self.pending = None;
            self.holding = false;
            // The dialog still shows: the hold starts again from the next reading without it. The person's input is kept for that
            // reading (a check can land before the agent redraws after an answer).
            self.hold_since = None;
            return None;
        }
        let released = std::mem::take(&mut self.released);
        let at_rest = matches!(reading, AgentState::Idle | AgentState::Done);
        if current == AgentState::Working && at_rest && !t.idle_confirm.is_zero() {
            return match self.pending {
                Some((s, since)) if s == reading => {
                    if now.saturating_duration_since(since) >= t.idle_confirm {
                        self.pending = None;
                        Some(reading)
                    } else {
                        None
                    }
                }
                _ => {
                    self.pending = Some((reading, now));
                    None
                }
            };
        }
        // The hold is measured from the first reading without the dialog, never from the last check that saw it: a check after a long
        // back-off must not take one redraw for an answer.
        if current == AgentState::Blocked && !t.blocked_hold.is_zero() && !released {
            let since = *self.hold_since.get_or_insert(now);
            if now.saturating_duration_since(since) < t.blocked_hold {
                self.holding = true;
                return None;
            }
        }
        self.apply();
        Some(reading)
    }

    fn apply(&mut self) {
        self.pending = None;
        self.holding = false;
        self.hold_since = None;
    }

    /// No rule matched this check: a rest state being confirmed is dropped (every check must read it), and nothing waits on this
    /// reading; a hold's clock keeps running.
    pub fn no_opinion(&mut self) {
        self.pending = None;
        self.holding = false;
    }

    /// Whether a change waits (an idle being confirmed, or a hold): the next check is due at the visible floor.
    pub fn waiting(&self) -> bool {
        self.pending.is_some() || self.holding
    }

    /// The person sent input to the terminal: a blocked hold ends at the next reading.
    pub fn input(&mut self) {
        self.released = true;
    }

    /// The state was set some other way (a node fact, the agent's own report): nothing pending stays.
    pub fn clear(&mut self) {
        self.released = false;
        self.apply();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use AgentState::{Blocked, Done, Idle, Working};

    const T: Timing = Timing {
        idle_confirm: Duration::from_millis(300),
        blocked_hold: Duration::from_millis(800),
    };

    fn ms(t0: Instant, n: u64) -> Instant {
        t0 + Duration::from_millis(n)
    }

    #[test]
    fn idle_is_applied_only_after_every_reading_agreed_for_the_confirm_time() {
        let (mut g, t0) = (Gate::default(), Instant::now());
        assert_eq!(g.offer(Working, Idle, t0, &T), None);
        assert!(g.waiting());
        assert_eq!(g.offer(Working, Idle, ms(t0, 100), &T), None);
        // A working reading in between drops it: a tool-call gap that showed the box for a moment stays working.
        assert_eq!(g.offer(Working, Working, ms(t0, 200), &T), None);
        assert!(!g.waiting());
        assert_eq!(g.offer(Working, Idle, ms(t0, 300), &T), None);
        assert_eq!(g.offer(Working, Idle, ms(t0, 500), &T), None);
        assert_eq!(g.offer(Working, Idle, ms(t0, 600), &T), Some(Idle));
        assert!(!g.waiting());
        // A different rest state starts again; done is confirmed like idle.
        assert_eq!(g.offer(Working, Done, ms(t0, 700), &T), None);
        assert_eq!(g.offer(Working, Idle, ms(t0, 800), &T), None);
        assert_eq!(g.offer(Working, Idle, ms(t0, 1100), &T), Some(Idle));
    }

    #[test]
    fn blocked_and_working_are_applied_at_once() {
        let (mut g, t0) = (Gate::default(), Instant::now());
        assert_eq!(g.offer(Working, Blocked, t0, &T), Some(Blocked));
        assert_eq!(g.offer(Idle, Working, t0, &T), Some(Working));
        assert_eq!(g.offer(Idle, Blocked, t0, &T), Some(Blocked));
        // An idle pending while working: blocked wins at once and drops it.
        assert_eq!(g.offer(Working, Idle, t0, &T), None);
        assert_eq!(g.offer(Working, Blocked, ms(t0, 50), &T), Some(Blocked));
        assert!(!g.waiting());
    }

    #[test]
    fn blocked_is_kept_until_no_blocked_reading_for_the_hold_and_input_ends_it() {
        let (mut g, t0) = (Gate::default(), Instant::now());
        assert_eq!(g.offer(Working, Blocked, t0, &T), Some(Blocked));
        // A redraw that misses the dialog for a moment changes nothing.
        assert_eq!(g.offer(Blocked, Working, ms(t0, 100), &T), None);
        assert!(g.waiting());
        assert_eq!(g.offer(Blocked, Blocked, ms(t0, 200), &T), None);
        assert_eq!(
            g.offer(Blocked, Working, ms(t0, 900), &T),
            None,
            "the hold starts at 900"
        );
        assert_eq!(g.offer(Blocked, Working, ms(t0, 1600), &T), None);
        assert_eq!(g.offer(Blocked, Working, ms(t0, 1700), &T), Some(Working));
        // After a long back-off with the dialog showing, one redraw that hides it is not an answer.
        assert_eq!(g.offer(Working, Blocked, ms(t0, 2000), &T), Some(Blocked));
        assert_eq!(g.offer(Blocked, Blocked, ms(t0, 2100), &T), None);
        assert_eq!(
            g.offer(Blocked, Idle, ms(t0, 4100), &T),
            None,
            "the first reading without it"
        );
        assert_eq!(g.offer(Blocked, Blocked, ms(t0, 4200), &T), None);
        // Answered by the person: shown at once, also when a check lands before the agent redraws.
        g.input();
        assert_eq!(g.offer(Blocked, Blocked, ms(t0, 4250), &T), None);
        assert_eq!(g.offer(Blocked, Working, ms(t0, 4300), &T), Some(Working));
    }

    #[test]
    fn a_check_with_no_opinion_drops_an_idle_being_confirmed() {
        let (mut g, t0) = (Gate::default(), Instant::now());
        assert_eq!(g.offer(Working, Idle, t0, &T), None);
        g.no_opinion();
        assert!(!g.waiting());
        assert_eq!(
            g.offer(Working, Idle, ms(t0, 5000), &T),
            None,
            "starts again"
        );
        assert_eq!(g.offer(Working, Idle, ms(t0, 5300), &T), Some(Idle));
    }

    #[test]
    fn no_timings_change_nothing_and_a_node_fact_clears_what_waits() {
        let (mut g, t0) = (Gate::default(), Instant::now());
        let none = Timing::default();
        assert_eq!(g.offer(Working, Idle, t0, &none), Some(Idle));
        assert_eq!(g.offer(Blocked, Working, t0, &none), Some(Working));
        assert_eq!(g.offer(Working, Idle, t0, &T), None);
        g.clear();
        assert!(!g.waiting());
        assert_eq!(
            g.offer(Done, Idle, ms(t0, 20), &T),
            Some(Idle),
            "from rest, at once"
        );
    }
}
