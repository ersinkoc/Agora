//! The hub's own numbers (architecture review R8; docs/design/observability.md), for `GET /api/metrics` in the Prometheus text
//! format. Counting is a few atomics and two clock reads per stored event; nothing is formatted until someone asks. Names and units
//! only: no sentences (the meaning of each is in observability.md).

use std::fmt::Write;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::Duration;

/// Monotonic counters stay at their public representation limit instead of wrapping to zero.
pub(crate) fn saturating_add(counter: &AtomicU64, value: u64) {
    let _ = counter.try_update(Relaxed, Relaxed, |current| {
        Some(current.saturating_add(value))
    });
}

/// Bucket bounds in microseconds for the hub's own work: 0.1 ms to 250 ms.
pub const LATENCY_US: [u64; 9] = [100, 250, 500, 1_000, 2_500, 5_000, 10_000, 50_000, 250_000];
/// Bucket bounds in microseconds for a person's answer: 1 s to 1 hour.
pub const ANSWER_US: [u64; 9] = [
    1_000_000,
    5_000_000,
    15_000_000,
    30_000_000,
    60_000_000,
    120_000_000,
    300_000_000,
    900_000_000,
    3_600_000_000,
];

/// A cumulative histogram of durations.
pub struct Histogram {
    bounds: &'static [u64; 9],
    /// Count per bucket (not cumulative: summed when rendered), the last one past every bound.
    buckets: [AtomicU64; 10],
    sum_us: AtomicU64,
}

impl Default for Histogram {
    fn default() -> Self {
        Self::with_bounds(&LATENCY_US)
    }
}

impl Histogram {
    pub fn with_bounds(bounds: &'static [u64; 9]) -> Self {
        Self {
            bounds,
            buckets: Default::default(),
            sum_us: AtomicU64::new(0),
        }
    }

    pub fn observe(&self, d: Duration) {
        let us = u64::try_from(d.as_micros()).unwrap_or(u64::MAX);
        let i = self
            .bounds
            .iter()
            .position(|b| us <= *b)
            .unwrap_or(self.bounds.len());
        saturating_add(&self.buckets[i], 1);
        // Saturating: a time from a machine's clock (an approval's answer far in the future) must not wrap the sum (#574 review).
        let _ = self
            .sum_us
            .try_update(Relaxed, Relaxed, |s| Some(s.saturating_add(us)));
    }

    /// A copy of the counts as they are now (rendered after the hub's lock is let go).
    pub fn snapshot(&self) -> Histogram {
        let h = Histogram::with_bounds(self.bounds);
        for (to, from) in h.buckets.iter().zip(&self.buckets) {
            to.store(from.load(Relaxed), Relaxed);
        }
        h.sum_us.store(self.sum_us.load(Relaxed), Relaxed);
        h
    }

    pub fn count(&self) -> u64 {
        self.buckets
            .iter()
            .map(|b| b.load(Relaxed))
            .fold(0, u64::saturating_add)
    }

    fn render(&self, name: &str, out: &mut String) {
        let _ = writeln!(out, "# TYPE {name} histogram");
        let mut seen = 0u64;
        for (i, bound) in self.bounds.iter().enumerate() {
            seen = seen.saturating_add(self.buckets[i].load(Relaxed));
            let _ = writeln!(
                out,
                "{name}_bucket{{le=\"{}\"}} {seen}",
                *bound as f64 / 1e6
            );
        }
        seen = seen.saturating_add(self.buckets[self.bounds.len()].load(Relaxed));
        let _ = writeln!(out, "{name}_bucket{{le=\"+Inf\"}} {seen}");
        let _ = writeln!(out, "{name}_sum {}", self.sum_us.load(Relaxed) as f64 / 1e6);
        let _ = writeln!(out, "{name}_count {seen}");
    }
}

/// What the hub counts as it works.
#[derive(Default)]
pub struct Metrics {
    /// Events from machines the hub stored (applied or resynced).
    pub events_ingested: AtomicU64,
    /// How long a machine's event waited for the hub's lock.
    pub lock_wait: Histogram,
    /// How long the hub held its lock for one machine event, storing it included: every client and every other machine waits that long.
    pub ingest: Histogram,
}

/// Point-in-time numbers read when the page is asked for.
pub struct Gauges {
    pub nodes_known: usize,
    pub nodes_online: usize,
    pub subscribers: usize,
    pub subscribers_dropped: u64,
    pub approvals_waiting: usize,
    /// How long agents waited for a person's answer, as the hub saw the answers.
    pub approval_wait: Histogram,
    pub relays_dropped: u64,
    pub terminal_bytes: u64,
    pub events_stored: u64,
    pub resyncs: usize,
    /// Commands queued for connected nodes and not yet written to them.
    pub node_commands_queued: usize,
    /// Commands written to connected nodes and not answered yet.
    pub node_commands_unanswered: usize,
}

impl Metrics {
    pub fn render(&self, g: &Gauges) -> String {
        let mut out = String::new();
        let counter = |out: &mut String, name: &str, v: u64| {
            let _ = writeln!(out, "# TYPE {name} counter\n{name} {v}");
        };
        let gauge = |out: &mut String, name: &str, v: u64| {
            let _ = writeln!(out, "# TYPE {name} gauge\n{name} {v}");
        };
        counter(
            &mut out,
            "agora_hub_events_ingested_total",
            self.events_ingested.load(Relaxed),
        );
        self.lock_wait
            .render("agora_hub_lock_wait_seconds", &mut out);
        self.ingest.render("agora_hub_ingest_seconds", &mut out);
        counter(
            &mut out,
            "agora_hub_subscribers_dropped_total",
            g.subscribers_dropped,
        );
        let n = |x: usize| u64::try_from(x).unwrap_or(u64::MAX);
        gauge(&mut out, "agora_hub_nodes_known", n(g.nodes_known));
        gauge(&mut out, "agora_hub_nodes_online", n(g.nodes_online));
        gauge(&mut out, "agora_hub_subscribers", n(g.subscribers));
        gauge(
            &mut out,
            "agora_hub_approvals_waiting",
            n(g.approvals_waiting),
        );
        g.approval_wait
            .render("agora_hub_approval_wait_seconds", &mut out);
        counter(&mut out, "agora_hub_relays_dropped_total", g.relays_dropped);
        counter(&mut out, "agora_hub_terminal_bytes_total", g.terminal_bytes);
        gauge(&mut out, "agora_hub_events_stored", g.events_stored);
        counter(&mut out, "agora_hub_resyncs_total", n(g.resyncs));
        gauge(
            &mut out,
            "agora_hub_node_commands_queued",
            n(g.node_commands_queued),
        );
        gauge(
            &mut out,
            "agora_hub_node_commands_unanswered",
            n(g.node_commands_unanswered),
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_histogram_is_cumulative_and_sums_its_durations() {
        let h = Histogram::default();
        h.observe(Duration::from_micros(50));
        h.observe(Duration::from_micros(700));
        h.observe(Duration::from_secs(1));
        let mut out = String::new();
        h.render("x_seconds", &mut out);
        assert!(out.contains("x_seconds_bucket{le=\"0.0001\"} 1\n"), "{out}");
        assert!(out.contains("x_seconds_bucket{le=\"0.001\"} 2\n"), "{out}");
        assert!(out.contains("x_seconds_bucket{le=\"0.25\"} 2\n"), "{out}");
        assert!(out.contains("x_seconds_bucket{le=\"+Inf\"} 3\n"), "{out}");
        assert!(out.contains("x_seconds_count 3\n"), "{out}");
        assert!(out.contains("x_seconds_sum 1.00075\n"), "{out}");
        assert_eq!(h.count(), 3);
    }

    #[test]
    fn a_far_future_time_does_not_wrap_the_sum() {
        // #574 review: an approval answered with an extreme `at_ms` (a machine's clock) saturates the sum instead of wrapping it.
        let h = Histogram::with_bounds(&ANSWER_US);
        h.observe(Duration::from_millis(u64::MAX));
        h.observe(Duration::from_millis(u64::MAX));
        h.observe(Duration::from_secs(1));
        assert_eq!(h.sum_us.load(Relaxed), u64::MAX);
        assert_eq!(h.count(), 3);
    }

    #[test]
    fn f367_observing_a_saturated_histogram_bucket_does_not_wrap_its_counter() {
        let h = Histogram::default();
        h.buckets[0].store(u64::MAX, Relaxed);
        h.observe(Duration::from_micros(1));
        assert_eq!(h.buckets[0].load(Relaxed), u64::MAX);
    }

    #[test]
    fn an_observation_still_increments_a_regular_histogram_bucket() {
        let h = Histogram::default();
        h.observe(Duration::from_micros(1));
        assert_eq!(h.buckets[0].load(Relaxed), 1);
    }

    #[test]
    fn f368_histogram_count_saturates_when_several_bucket_counts_overflow_the_total() {
        let h = Histogram::default();
        h.buckets[0].store(u64::MAX, Relaxed);
        h.buckets[1].store(1, Relaxed);
        assert_eq!(h.count(), u64::MAX);
    }

    #[test]
    fn one_full_bucket_remains_a_valid_histogram_count() {
        let h = Histogram::default();
        h.buckets[0].store(u64::MAX, Relaxed);
        assert_eq!(h.count(), u64::MAX);
    }

    #[test]
    fn f369_histogram_render_saturates_its_cumulative_bucket_counts() {
        let h = Histogram::default();
        h.buckets[0].store(u64::MAX, Relaxed);
        h.buckets[1].store(1, Relaxed);
        let mut out = String::new();
        h.render("x_seconds", &mut out);
        assert!(out.contains("x_seconds_bucket{le=\"+Inf\"} 18446744073709551615\n"));
        assert!(out.contains("x_seconds_count 18446744073709551615\n"));
    }

    #[test]
    fn histogram_render_keeps_an_exact_unsaturated_cumulative_count() {
        let h = Histogram::default();
        h.buckets[0].store(2, Relaxed);
        h.buckets[1].store(3, Relaxed);
        let mut out = String::new();
        h.render("x_seconds", &mut out);
        assert!(out.contains("x_seconds_bucket{le=\"0.00025\"} 5\n"));
        assert!(out.contains("x_seconds_count 5\n"));
    }

    #[test]
    fn verifies_f367_bucket_saturation_across_first_middle_and_overflow_buckets() {
        for (bucket, duration) in [
            (0, Duration::from_micros(1)),
            (4, Duration::from_micros(2_000)),
            (9, Duration::from_micros(300_000)),
        ] {
            let h = Histogram::default();
            h.buckets[bucket].store(u64::MAX, Relaxed);
            h.observe(duration);
            assert_eq!(h.buckets[bucket].load(Relaxed), u64::MAX, "{bucket}");
        }
    }

    #[test]
    fn verifies_f368_count_saturation_across_early_late_and_many_bucket_totals() {
        let early = Histogram::default();
        early.buckets[0].store(u64::MAX, Relaxed);
        early.buckets[1].store(1, Relaxed);
        assert_eq!(early.count(), u64::MAX);

        let late = Histogram::default();
        late.buckets[8].store(u64::MAX - 1, Relaxed);
        late.buckets[9].store(2, Relaxed);
        assert_eq!(late.count(), u64::MAX);

        let many = Histogram::default();
        for bucket in &many.buckets {
            bucket.store(u64::MAX / 2, Relaxed);
        }
        assert_eq!(many.count(), u64::MAX);
    }

    #[test]
    fn verifies_f369_render_saturation_across_bounded_overflow_and_near_max_totals() {
        for (first, second, overflow) in [(u64::MAX, 1, 0), (0, u64::MAX, 1), (u64::MAX - 2, 1, 2)]
        {
            let h = Histogram::default();
            h.buckets[0].store(first, Relaxed);
            h.buckets[8].store(second, Relaxed);
            h.buckets[9].store(overflow, Relaxed);
            let mut out = String::new();
            h.render("x_seconds", &mut out);
            assert!(
                out.contains("x_seconds_bucket{le=\"+Inf\"} 18446744073709551615\n"),
                "{out}"
            );
            assert!(
                out.contains("x_seconds_count 18446744073709551615\n"),
                "{out}"
            );
        }
    }
}
