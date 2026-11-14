//! The hub's own numbers (architecture review R8; docs/design/observability.md), for `GET /api/metrics` in the Prometheus text
//! format. Counting is a few atomics and two clock reads per stored event; nothing is formatted until someone asks. Names and units
//! only: no sentences (the meaning of each is in observability.md).

use std::fmt::Write;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::Duration;

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
        self.buckets[i].fetch_add(1, Relaxed);
        self.sum_us.fetch_add(us, Relaxed);
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
        self.buckets.iter().map(|b| b.load(Relaxed)).sum()
    }

    fn render(&self, name: &str, out: &mut String) {
        let _ = writeln!(out, "# TYPE {name} histogram");
        let mut seen = 0;
        for (i, bound) in self.bounds.iter().enumerate() {
            seen += self.buckets[i].load(Relaxed);
            let _ = writeln!(
                out,
                "{name}_bucket{{le=\"{}\"}} {seen}",
                *bound as f64 / 1e6
            );
        }
        seen += self.buckets[self.bounds.len()].load(Relaxed);
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
}
