//! Benchmark harness (ADR-0022): named scenarios, JSON results, regression comparison.
//!
//! `channel-throughput` measures bare channel overhead (a floor for the other scenarios). `pty-vt-throughput`
//! and `terminals-20` run the real path: the fake agent floods a PTY and every chunk is fed to the VT backend,
//! for one terminal and for twenty concurrent ones (`design/performance.md`).

pub mod footprint;

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
pub const SCENARIOS: &[&str] = &["channel-throughput", "pty-vt-throughput", "terminals-20"];

/// Argument that makes the bench binary act as the flooding fake agent (it spawns itself in the PTY).
pub const FLOOD_CHILD_ARG: &str = "flood-child";

/// Run a scenario by name. `slowdown` adds a delay per chunk (used to prove regression detection).
pub fn run_scenario(name: &str, duration: Duration, slowdown: Duration) -> Option<ScenarioResult> {
    match name {
        "channel-throughput" => Some(channel_throughput(duration, slowdown)),
        "pty-vt-throughput" => pty_vt("pty-vt-throughput", 1, duration, slowdown),
        "terminals-20" => pty_vt("terminals-20", 20, duration, slowdown),
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

/// `n` terminals, each a flooding fake agent in a PTY feeding an `AlacrittyTerminal`. Throughput is the total
/// over all terminals; latency is the p95 time to parse one chunk. `None` if the PTY cannot be started.
fn pty_vt(name: &str, n: usize, duration: Duration, slowdown: Duration) -> Option<ScenarioResult> {
    let exe = std::env::current_exe().ok()?.to_string_lossy().into_owned();
    pty_vt_with(
        name,
        n,
        &[exe, FLOOD_CHILD_ARG.to_string()],
        duration,
        slowdown,
    )
}

fn pty_vt_with(
    name: &str,
    n: usize,
    argv: &[String],
    duration: Duration,
    slowdown: Duration,
) -> Option<ScenarioResult> {
    use agora_platform::{spawn, PtyConfig};
    use agora_vt::{AlacrittyTerminal, Size, Terminal};

    let size = Size {
        cols: 120,
        rows: 40,
    };
    let mut ptys = Vec::new();
    for _ in 0..n {
        let mut cfg = PtyConfig::new(argv.to_vec());
        cfg.cols = size.cols;
        cfg.rows = size.rows;
        ptys.push(spawn(&cfg).ok()?);
    }
    let start = Instant::now();
    let deadline = start + duration;
    let results: Vec<(usize, Vec<Duration>)> = thread::scope(|s| {
        let handles: Vec<_> = ptys
            .iter_mut()
            .map(|pty| {
                s.spawn(move || {
                    let mut term = AlacrittyTerminal::new(size);
                    let (mut bytes, mut lat) = (0usize, Vec::new());
                    while Instant::now() < deadline {
                        let Ok(chunk) = pty.output().recv_timeout(Duration::from_millis(100))
                        else {
                            continue;
                        };
                        let t = Instant::now();
                        term.feed(&chunk);
                        if !slowdown.is_zero() {
                            thread::sleep(slowdown);
                        }
                        lat.push(t.elapsed());
                        bytes += chunk.len();
                    }
                    let _ = pty.kill();
                    (bytes, lat)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or((0, Vec::new())))
            .collect()
    });
    let secs = start.elapsed().as_secs_f64();
    let bytes: usize = results.iter().map(|r| r.0).sum();
    let mut latencies: Vec<Duration> = results.into_iter().flat_map(|r| r.1).collect();
    Some(ScenarioResult {
        name: name.into(),
        throughput_mb_s: bytes as f64 / 1e6 / secs,
        p95_latency_us: p95_us(&mut latencies),
    })
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

/// Combine two runs of the same scenario, keeping the best of each metric (highest throughput, lowest latency).
pub fn best_of(a: ScenarioResult, b: ScenarioResult) -> ScenarioResult {
    ScenarioResult {
        name: a.name,
        throughput_mb_s: a.throughput_mb_s.max(b.throughput_mb_s),
        p95_latency_us: a.p95_latency_us.min(b.p95_latency_us),
    }
}

/// Merge two reports of the same scenarios, keeping the best of each metric (see [`best_of`]). Scenarios present in
/// only one report are kept as they are.
pub fn merge_reports(mut a: Report, b: Report) -> Report {
    for nb in b.scenarios {
        match a.scenarios.iter().position(|x| x.name == nb.name) {
            Some(i) => {
                let old = a.scenarios[i].clone();
                a.scenarios[i] = best_of(old, nb);
            }
            None => a.scenarios.push(nb),
        }
    }
    a
}

/// Scenarios whose latency is reported but not gated. `terminals-20` runs twenty busy threads on a CI runner
/// with a few cores, so its p95 measures the OS scheduler, not our code (a 25% swing on identical code was
/// observed). Its throughput, which is stable, still gates.
pub const LATENCY_NOT_GATED: &[&str] = &["terminals-20"];

/// Scenarios whose latency is gated at a multiple of the threshold. `pty-vt-throughput`'s p95 is the time to
/// parse one chunk, tens of microseconds, so shared-runner noise reached +26% on identical code. A real
/// slowdown of the parser is orders of magnitude larger (the deliberate-slowdown check moves it by >1000%) and
/// still fails, and the stable throughput metric keeps the exact threshold.
///
/// `channel-throughput` is a bare in-process channel (the gate's stable base); its p95 is a few microseconds and
/// showed +13% and +15% on two unrelated PRs (docs and a test fix), so it gets the same factor. Its throughput
/// still gates at the exact threshold.
pub const LATENCY_THRESHOLD_FACTOR: &[(&str, f64)] =
    &[("pty-vt-throughput", 3.0), ("channel-throughput", 3.0)];

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
        if b.p95_latency_us > 0.0 && !LATENCY_NOT_GATED.contains(&n.name.as_str()) {
            let rise = (n.p95_latency_us - b.p95_latency_us) / b.p95_latency_us * 100.0;
            let factor = LATENCY_THRESHOLD_FACTOR
                .iter()
                .find(|(name, _)| *name == n.name)
                .map_or(1.0, |(_, f)| *f);
            if rise > threshold_pct * factor {
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
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn merging_reports_keeps_the_best_of_each_metric() {
        let merged = merge_reports(report(100.0, 50.0), report(90.0, 40.0));
        assert_eq!(merged.scenarios.len(), 1);
        assert!((merged.scenarios[0].throughput_mb_s - 100.0).abs() < 1e-9);
        assert!((merged.scenarios[0].p95_latency_us - 40.0).abs() < 1e-9);
    }

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
    fn oversubscribed_scenarios_gate_throughput_but_not_latency() {
        let mut base = report(100.0, 10.0);
        let mut new = report(100.0, 20.0);
        base.scenarios[0].name = "terminals-20".into();
        new.scenarios[0].name = "terminals-20".into();
        assert!(compare(&base, &new, 10.0).is_empty());
        new.scenarios[0].throughput_mb_s = 50.0;
        let r = compare(&base, &new, 10.0);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].metric, "throughput_mb_s");
    }

    #[test]
    fn micro_latency_scenarios_tolerate_noise_but_not_real_slowdowns() {
        let named = |tp, lat| {
            let mut r = report(tp, lat);
            r.scenarios[0].name = "pty-vt-throughput".into();
            r
        };
        // +26% latency (seen on identical code) passes; +100% fails; throughput keeps the exact threshold.
        assert!(compare(&named(100.0, 50.0), &named(100.0, 63.0), 10.0).is_empty());
        let r = compare(&named(100.0, 50.0), &named(100.0, 100.0), 10.0);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].metric, "p95_latency_us");
        assert_eq!(
            compare(&named(100.0, 50.0), &named(85.0, 50.0), 10.0).len(),
            1
        );
    }

    #[test]
    fn best_of_keeps_the_best_metric_from_each_run() {
        let r = |tp, lat| ScenarioResult {
            name: "s".into(),
            throughput_mb_s: tp,
            p95_latency_us: lat,
        };
        assert_eq!(best_of(r(100.0, 50.0), r(80.0, 20.0)), r(100.0, 20.0));
    }

    #[test]
    fn p95_of_samples() {
        let mut s: Vec<_> = (1..=100).map(Duration::from_micros).collect();
        assert_eq!(p95_us(&mut s), 95.0);
        assert_eq!(p95_us(&mut []), 0.0);
    }

    #[cfg(unix)]
    #[test]
    fn pty_pipeline_moves_bytes_through_the_vt_backend() {
        let argv: Vec<String> = ["sh", "-c", "yes 'some output'"].map(String::from).to_vec();
        let r =
            pty_vt_with("t", 2, &argv, Duration::from_millis(300), Duration::ZERO).expect("spawn");
        assert!(r.throughput_mb_s > 0.0);
        assert!(r.p95_latency_us > 0.0);
    }

    #[test]
    fn unknown_scenario_is_none() {
        assert!(run_scenario("nope", Duration::from_millis(1), Duration::ZERO).is_none());
    }
}
