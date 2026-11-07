//! Benchmark harness (ADR-0022): named scenarios, JSON results, regression comparison.
//!
//! The first scenario is a stand-in for "fake agent through a PTY into the VT backend": bytes go through
//! an in-process channel into a [`Sink`]. It is replaced by the real PTY and VT path once `agora-platform`
//! and `agora-vt` exist; the result format and comparison logic stay the same.

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Result of one scenario run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScenarioResult {
    pub name: String,
    /// Higher is better.
    pub throughput_mb_s: f64,
    /// Lower is better.
    pub p95_latency_us: f64,
}

/// All results of one run, tagged with the commit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub commit: String,
    pub scenarios: Vec<ScenarioResult>,
}

/// Consumes terminal bytes; the VT backend will implement this.
pub trait Sink: Send + 'static {
    fn consume(&mut self, bytes: &[u8]);
}

/// Names of built-in scenarios.
pub const SCENARIOS: &[&str] = &["channel-throughput"];

/// Run a scenario by name. `slowdown` adds a delay per chunk (used to prove regression detection).
pub fn run_scenario(name: &str, duration: Duration, slowdown: Duration) -> Option<ScenarioResult> {
    match name {
        "channel-throughput" => Some(channel_throughput(duration, slowdown)),
        _ => None,
    }
}

struct Checksum(u64);

impl Sink for Checksum {
    fn consume(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 = self.0.wrapping_mul(31).wrapping_add(u64::from(*b));
        }
    }
}

const CHUNK: usize = 16 * 1024;

fn channel_throughput(duration: Duration, slowdown: Duration) -> ScenarioResult {
    let (tx, rx) = mpsc::sync_channel::<(Instant, Vec<u8>)>(64);
    let consumer = thread::spawn(move || {
        let mut sink = Checksum(0);
        let mut latencies = Vec::new();
        let mut bytes = 0usize;
        while let Ok((sent, chunk)) = rx.recv() {
            sink.consume(&chunk);
            if !slowdown.is_zero() {
                thread::sleep(slowdown);
            }
            latencies.push(sent.elapsed());
            bytes += chunk.len();
        }
        (bytes, latencies, sink.0)
    });
    let start = Instant::now();
    while start.elapsed() < duration {
        if tx.send((Instant::now(), vec![b'x'; CHUNK])).is_err() {
            break;
        }
    }
    drop(tx);
    let (bytes, mut latencies, _) = consumer.join().unwrap_or((0, Vec::new(), 0));
    let secs = start.elapsed().as_secs_f64();
    ScenarioResult {
        name: "channel-throughput".into(),
        throughput_mb_s: bytes as f64 / 1e6 / secs,
        p95_latency_us: p95_us(&mut latencies),
    }
}

/// 95th percentile in microseconds; 0 for no samples.
pub fn p95_us(samples: &mut [Duration]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    samples.sort();
    let idx = ((samples.len() as f64) * 0.95).ceil() as usize;
    samples[idx.saturating_sub(1).min(samples.len() - 1)].as_secs_f64() * 1e6
}

/// A regression found by [`compare`].
#[derive(Debug, Clone, PartialEq)]
pub struct Regression {
    pub scenario: String,
    pub metric: &'static str,
    pub change_pct: f64,
}

/// Compare `new` against `base`; a metric worse by more than `threshold_pct` is a regression.
/// Scenarios missing from either report are ignored.
pub fn compare(base: &Report, new: &Report, threshold_pct: f64) -> Vec<Regression> {
    let mut out = Vec::new();
    for n in &new.scenarios {
        let Some(b) = base.scenarios.iter().find(|b| b.name == n.name) else {
            continue;
        };
        if b.throughput_mb_s > 0.0 {
            let drop = (b.throughput_mb_s - n.throughput_mb_s) / b.throughput_mb_s * 100.0;
            if drop > threshold_pct {
                out.push(Regression {
                    scenario: n.name.clone(),
                    metric: "throughput_mb_s",
                    change_pct: -drop,
                });
            }
        }
        if b.p95_latency_us > 0.0 {
            let rise = (n.p95_latency_us - b.p95_latency_us) / b.p95_latency_us * 100.0;
            if rise > threshold_pct {
                out.push(Regression {
                    scenario: n.name.clone(),
                    metric: "p95_latency_us",
                    change_pct: rise,
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(tp: f64, lat: f64) -> Report {
        Report {
            commit: "x".into(),
            scenarios: vec![ScenarioResult {
                name: "s".into(),
                throughput_mb_s: tp,
                p95_latency_us: lat,
            }],
        }
    }

    #[test]
    fn within_threshold_passes() {
        assert!(compare(&report(100.0, 10.0), &report(95.0, 10.9), 10.0).is_empty());
    }

    #[test]
    fn slower_or_laggier_fails() {
        let r = compare(&report(100.0, 10.0), &report(80.0, 12.0), 10.0);
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn p95_of_samples() {
        let mut s: Vec<_> = (1..=100).map(Duration::from_micros).collect();
        assert_eq!(p95_us(&mut s), 95.0);
        assert_eq!(p95_us(&mut []), 0.0);
    }

    #[test]
    fn unknown_scenario_is_none() {
        assert!(run_scenario("nope", Duration::from_millis(1), Duration::ZERO).is_none());
    }
}
