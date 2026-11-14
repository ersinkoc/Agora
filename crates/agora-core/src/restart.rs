//! When a supervisor starts the role it runs (the hub or the node) again after it ended (ADR-0062 item 8).
//!
//! The rule, measured first as the restart loop of the Windows node (`docs/adr/0062-agora-up-brings-a-machine-back.md`):
//! - an exit with code 0 is a deliberate stop (`agora down`, a clean shutdown): never restarted;
//! - an end soon after the start is a refusal (the port is taken, the configuration is wrong): starting again would fail the same
//!   way, so it is not retried;
//! - any other end is a crash: restarted after a wait, at most a few times in a window, then the supervisor gives up.
//!
//! Plain data and a pure function: the caller measures the times (milliseconds on a clock of its own) and acts on the answer.

/// The numbers of the rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RestartRule {
    /// How long to wait before starting again after a crash.
    pub wait_ms: u64,
    /// An end within this long of the start is a refusal.
    pub refusal_ms: u64,
    /// At most this many restarts in `window_ms`; the next crash makes the supervisor give up.
    pub max_restarts: usize,
    pub window_ms: u64,
}

impl Default for RestartRule {
    /// 30 s between restarts, an end within 10 s is a refusal, at most 5 restarts in 10 minutes.
    fn default() -> Self {
        RestartRule {
            wait_ms: 30_000,
            refusal_ms: 10_000,
            max_restarts: 5,
            window_ms: 600_000,
        }
    }
}

/// How the supervised process ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    /// Exit code 0.
    Clean,
    /// Any other exit code, or killed by a signal.
    Failed,
}

/// What the supervisor does next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    /// A deliberate stop: the supervisor ends too.
    Stop,
    /// It ended too soon after its start to be a crash: not retried, the supervisor ends.
    Refused,
    /// A crash after `max_restarts` restarts within the window: the supervisor ends.
    GiveUp,
    /// A crash: start it again after `wait_ms`; this is restart number `count` within the window.
    Restart { wait_ms: u64, count: usize },
}

/// The restarts made so far, as times on the caller's clock.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Restarts {
    at: Vec<u64>,
}

impl Restarts {
    /// The restarts within the rule's window before `now`.
    pub fn recent(&self, rule: &RestartRule, now: u64) -> usize {
        self.at
            .iter()
            .filter(|t| now.saturating_sub(**t) < rule.window_ms)
            .count()
    }

    /// Decides what follows an end at `now` of a process that had run for `up_ms`, and records a restart when it is one.
    pub fn after(&mut self, rule: &RestartRule, ending: Ending, up_ms: u64, now: u64) -> Next {
        if ending == Ending::Clean {
            return Next::Stop;
        }
        if up_ms < rule.refusal_ms {
            return Next::Refused;
        }
        self.at.retain(|t| now.saturating_sub(*t) < rule.window_ms);
        if self.at.len() >= rule.max_restarts {
            return Next::GiveUp;
        }
        self.at.push(now);
        Next::Restart {
            wait_ms: rule.wait_ms,
            count: self.at.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60_000;

    #[test]
    fn a_clean_exit_is_never_restarted_however_long_it_ran() {
        let rule = RestartRule::default();
        let mut r = Restarts::default();
        assert_eq!(r.after(&rule, Ending::Clean, 0, 0), Next::Stop);
        assert_eq!(
            r.after(&rule, Ending::Clean, 99 * MIN, 99 * MIN),
            Next::Stop
        );
        assert_eq!(r.recent(&rule, 99 * MIN), 0);
    }

    #[test]
    fn a_failure_soon_after_the_start_is_a_refusal_and_not_retried() {
        let rule = RestartRule::default();
        let mut r = Restarts::default();
        assert_eq!(r.after(&rule, Ending::Failed, 9_999, 9_999), Next::Refused);
        assert_eq!(r.recent(&rule, 9_999), 0);
        // At the threshold it is a crash.
        assert_eq!(
            r.after(&rule, Ending::Failed, 10_000, 10_000),
            Next::Restart {
                wait_ms: 30_000,
                count: 1
            }
        );
    }

    #[test]
    fn crashes_restart_five_times_in_ten_minutes_then_give_up() {
        let rule = RestartRule::default();
        let mut r = Restarts::default();
        let mut now = 0;
        for count in 1..=5 {
            now += 31_000 + rule.wait_ms;
            assert_eq!(
                r.after(&rule, Ending::Failed, 31_000, now),
                Next::Restart {
                    wait_ms: 30_000,
                    count
                }
            );
        }
        now += 31_000 + rule.wait_ms;
        assert_eq!(r.after(&rule, Ending::Failed, 31_000, now), Next::GiveUp);
        // Giving up records nothing more.
        assert_eq!(r.recent(&rule, now), 5);
    }

    #[test]
    fn restarts_older_than_the_window_no_longer_count() {
        let rule = RestartRule::default();
        let mut r = Restarts::default();
        for i in 0..5 {
            r.after(&rule, Ending::Failed, MIN, i * MIN);
        }
        assert_eq!(r.recent(&rule, 4 * MIN), 5);
        // Ten minutes after the first restart only four are recent: one more is allowed.
        assert_eq!(r.recent(&rule, 10 * MIN), 4);
        assert_eq!(
            r.after(&rule, Ending::Failed, MIN, 10 * MIN),
            Next::Restart {
                wait_ms: 30_000,
                count: 5
            }
        );
        assert_eq!(
            r.after(&rule, Ending::Failed, MIN, 10 * MIN + 1),
            Next::GiveUp
        );
    }

    #[test]
    fn the_rule_s_numbers_are_the_agreed_ones() {
        assert_eq!(
            RestartRule::default(),
            RestartRule {
                wait_ms: 30_000,
                refusal_ms: 10_000,
                max_restarts: 5,
                window_ms: 10 * MIN
            }
        );
    }
}
