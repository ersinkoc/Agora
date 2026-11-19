//! Plan limits of subscription agents (ADR-0091): what the node sends of the windows a tool reports in its own usage file. The
//! reader (`agora-adapters::usage`) takes three numbers per window and nothing else; this gate bounds them, keeps the newest per
//! `(tool, window)` and sends an observation only when its numbers changed, at most once every 5 seconds per window. Pure: time is
//! given, nothing here reads a file or writes an event.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use agora_adapters::usage::{QuotaRaw, QuotaSample};
use agora_protocol::v1::AgentQuotaObserved;

/// At most one observation per tool and window in this time; a change that comes sooner waits and is sent when it is due.
pub const MIN_GAP: Duration = Duration::from_secs(5);
/// Most `(tool, window)` pairs followed: a hostile file cannot make the table grow by inventing window lengths.
const MAX_WINDOWS: usize = 32;
/// Where the numbers came from (the tool's own file).
const SOURCE: &str = "file";

struct Entry {
    /// What was sent last: `(used_percent, resets_at_ms)`, and when.
    sent: Option<((u32, u64), Instant)>,
    /// The newest observation that differs from what was sent and waits for its time, with the node's clock when it was read.
    waiting: Option<(QuotaSample, u64)>,
}

pub struct QuotaGate {
    node_id: String,
    windows: HashMap<(String, String), Entry>,
}

impl QuotaGate {
    pub fn new(node_id: impl Into<String>) -> Self {
        Self {
            node_id: node_id.into(),
            windows: HashMap::new(),
        }
    }

    fn event(&self, tool: &str, s: &QuotaSample, observed_ms: u64) -> AgentQuotaObserved {
        // Built field by field from the bounded sample: nothing else of what the tool wrote can reach it.
        AgentQuotaObserved {
            node_id: self.node_id.clone(),
            tool: tool.to_string(),
            window: s.window.clone(),
            used_percent: s.used_percent,
            resets_at_ms: s.resets_at_ms,
            source: SOURCE.into(),
            observed_at_ms: observed_ms,
        }
    }

    /// One window as read from a file at `now_ms` (the node's clock). Returns the observation to send now, if any: the numbers are
    /// bounded (an impossible reset time drops it), must differ from the last sent, and the window must not have sent within
    /// [`MIN_GAP`]; otherwise the newest waits for [`Self::due`].
    pub fn offer(
        &mut self,
        now: Instant,
        now_ms: u64,
        tool: &str,
        raw: &QuotaRaw,
    ) -> Option<AgentQuotaObserved> {
        let sample = raw.bound(now_ms)?;
        let key = (tool.to_string(), sample.window.clone());
        if !self.windows.contains_key(&key) && self.windows.len() >= MAX_WINDOWS {
            return None;
        }
        let entry = self.windows.entry(key).or_insert(Entry {
            sent: None,
            waiting: None,
        });
        let numbers = (sample.used_percent, sample.resets_at_ms);
        if entry.sent.map(|(n, _)| n) == Some(numbers) {
            // Back to what the window already shows: nothing to say, and nothing left waiting.
            entry.waiting = None;
            return None;
        }
        let due = entry
            .sent
            .is_none_or(|(_, at)| now.saturating_duration_since(at) >= MIN_GAP);
        if !due {
            entry.waiting = Some((sample, now_ms));
            return None;
        }
        entry.sent = Some((numbers, now));
        entry.waiting = None;
        Some(self.event(tool, &sample, now_ms))
    }

    /// The observations that waited for their time and whose time has come.
    pub fn due(&mut self, now: Instant) -> Vec<AgentQuotaObserved> {
        let mut ready = Vec::new();
        for ((tool, _), e) in &mut self.windows {
            let Some((sample, observed_ms)) = e.waiting.clone() else {
                continue;
            };
            let sent_at = e.sent.map(|(_, at)| at);
            if sent_at.is_none_or(|at| now.saturating_duration_since(at) >= MIN_GAP) {
                e.sent = Some(((sample.used_percent, sample.resets_at_ms), now));
                e.waiting = None;
                ready.push((tool.clone(), sample, observed_ms));
            }
        }
        ready.sort_by(|a, b| (&a.0, &a.1.window).cmp(&(&b.0, &b.1.window)));
        ready
            .iter()
            .map(|(tool, s, at)| self.event(tool, s, *at))
            .collect()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const NOW_MS: u64 = 1_800_000_000_000;

    fn raw(percent: f64, minutes: u64, resets_ms: u64) -> QuotaRaw {
        QuotaRaw {
            percent,
            minutes,
            resets_ms,
        }
    }

    #[test]
    fn an_observation_is_sent_only_when_it_changed() {
        let mut g = QuotaGate::new("n1");
        let t0 = Instant::now();
        let r = raw(40.0, 300, NOW_MS + 3_600_000);
        let e = g.offer(t0, NOW_MS, "codex", &r).unwrap();
        assert_eq!(
            (e.node_id.as_str(), e.tool.as_str(), e.window.as_str()),
            ("n1", "codex", "five_hour")
        );
        assert_eq!((e.used_percent, e.resets_at_ms), (40, NOW_MS + 3_600_000));
        assert_eq!((e.source.as_str(), e.observed_at_ms), ("file", NOW_MS));
        let later = t0 + Duration::from_secs(60);
        assert!(
            g.offer(later, NOW_MS, "codex", &r).is_none(),
            "same numbers"
        );
        let e = g
            .offer(
                later,
                NOW_MS + 60_000,
                "codex",
                &raw(41.0, 300, NOW_MS + 3_600_000),
            )
            .unwrap();
        assert_eq!(e.used_percent, 41);
        // A new reset time is a change too.
        let r2 = raw(41.0, 300, NOW_MS + 4_000_000);
        let much_later = later + Duration::from_secs(60);
        assert!(g.offer(much_later, NOW_MS, "codex", &r2).is_some());
    }

    #[test]
    fn at_most_one_per_window_every_five_seconds_and_the_newest_is_sent_when_due() {
        let mut g = QuotaGate::new("n1");
        let t0 = Instant::now();
        let resets = NOW_MS + 3_600_000;
        assert!(g
            .offer(t0, NOW_MS, "codex", &raw(10.0, 300, resets))
            .is_some());
        for (i, p) in [11.0, 12.0, 13.0].into_iter().enumerate() {
            let at = t0 + Duration::from_millis(500 * (i as u64 + 1));
            assert!(g
                .offer(at, NOW_MS + 500, "codex", &raw(p, 300, resets))
                .is_none());
        }
        // Another window is not held back by this one.
        assert!(g
            .offer(
                t0 + Duration::from_secs(1),
                NOW_MS,
                "codex",
                &raw(5.0, 10_080, resets)
            )
            .is_some());
        assert!(g.due(t0 + Duration::from_secs(4)).is_empty(), "not yet");
        let due = g.due(t0 + Duration::from_secs(5));
        assert_eq!(due.len(), 1);
        assert_eq!(
            (due[0].window.as_str(), due[0].used_percent),
            ("five_hour", 13)
        );
        assert_eq!(
            due[0].observed_at_ms,
            NOW_MS + 500,
            "when it was read, not when it was sent"
        );
        assert!(g.due(t0 + Duration::from_secs(30)).is_empty(), "sent once");
        // A change that returns to what was sent leaves nothing waiting.
        let t = t0 + Duration::from_secs(31);
        assert!(g
            .offer(t, NOW_MS, "codex", &raw(14.0, 300, resets))
            .is_some());
        assert!(g
            .offer(
                t + Duration::from_secs(1),
                NOW_MS,
                "codex",
                &raw(15.0, 300, resets)
            )
            .is_none());
        assert!(g
            .offer(
                t + Duration::from_secs(2),
                NOW_MS,
                "codex",
                &raw(14.0, 300, resets)
            )
            .is_none());
        assert!(g.due(t + Duration::from_secs(60)).is_empty());
    }

    #[test]
    fn the_percentage_is_clamped_and_an_impossible_reset_time_drops_the_observation() {
        let mut g = QuotaGate::new("n1");
        let t = Instant::now();
        let resets = NOW_MS + 1000;
        assert_eq!(
            g.offer(t, NOW_MS, "c", &raw(250.0, 300, resets))
                .unwrap()
                .used_percent,
            100
        );
        assert_eq!(
            g.offer(t, NOW_MS, "c", &raw(-40.0, 10_080, resets))
                .unwrap()
                .used_percent,
            0
        );
        let day = 24 * 60 * 60 * 1000;
        for bad in [NOW_MS - day - 1, NOW_MS + 31 * day + 1, 0, u64::MAX] {
            assert!(
                g.offer(t, NOW_MS, "c", &raw(10.0, 90, bad)).is_none(),
                "{bad}"
            );
        }
        assert!(g
            .offer(t, NOW_MS, "c", &raw(f64::NAN, 90, resets))
            .is_none());
        assert!(g
            .offer(t, NOW_MS, "c", &raw(f64::INFINITY, 91, resets))
            .is_none());
        // The edges are inside.
        assert!(g
            .offer(t, NOW_MS, "c", &raw(10.0, 92, NOW_MS - day))
            .is_some());
        assert!(g
            .offer(t, NOW_MS, "c", &raw(10.0, 93, NOW_MS + 31 * day))
            .is_some());
    }

    #[test]
    fn the_table_of_windows_is_bounded() {
        let mut g = QuotaGate::new("n1");
        let t = Instant::now();
        let mut sent = 0;
        for minutes in 1..=100 {
            if g.offer(t, NOW_MS, "c", &raw(1.0, minutes, NOW_MS + 1000))
                .is_some()
            {
                sent += 1;
            }
        }
        assert_eq!(sent, MAX_WINDOWS);
    }
}
