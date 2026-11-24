//! `agora-bench run [--out FILE] [--secs N] [--repeat N] [--commit SHA] [scenario...]`
//! (`--repeat` runs each scenario N times and keeps the best of each metric, which filters scheduler noise
//! on shared CI runners; a real regression shows in every repetition.)
//! `agora-bench bestof A.json B.json... [--out FILE]` (merge reports, keeping the best of each metric per scenario;
//! CI alternates base and head runs and merges each side, so slow drift on a shared runner hits both equally)
//! `agora-bench compare BASE.json NEW.json [--threshold PCT]` (exit 1 on regression)
//!
//! `AGORA_BENCH_SLOWDOWN_US` deliberately slows the hot path, to prove the gate works.

use std::process::ExitCode;
use std::time::Duration;

use agora_bench::{compare, run_scenario, Report, SCENARIOS};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        // Acts as the fake agent inside a benchmark PTY; ends when the terminal closes.
        Some(agora_bench::FLOOD_CHILD_ARG) => {
            let _ = agora_fake_agent::flood(&mut std::io::stdout().lock(), None);
            ExitCode::SUCCESS
        }
        Some("run") => run(&args[1..]),
        Some("footprint") => footprint(&args[1..]),
        Some("history") => history(&args[1..]),
        Some("transcript") => transcript(&args[1..]),
        Some("bestof") => bestof(&args[1..]),
        Some("compare") => cmp(&args[1..]),
        _ => {
            eprintln!("usage: agora-bench run [--out F] [--secs N] [--commit SHA] [scenario...] | compare BASE NEW [--threshold PCT]");
            ExitCode::from(2)
        }
    }
}

fn next_value<'a>(it: &mut std::slice::Iter<'a, String>, flag: &str) -> Result<&'a str, ExitCode> {
    it.next().map(String::as_str).ok_or_else(|| {
        eprintln!("agora-bench: missing value for {flag}");
        ExitCode::from(2)
    })
}

fn parse_value<T: std::str::FromStr>(
    it: &mut std::slice::Iter<'_, String>,
    flag: &str,
) -> Result<T, ExitCode> {
    let raw = next_value(it, flag)?;
    raw.parse().map_err(|_| {
        eprintln!("agora-bench: invalid value for {flag}: {raw}");
        ExitCode::from(2)
    })
}

fn run(args: &[String]) -> ExitCode {
    let (mut out, mut secs, mut repeat, mut commit, mut names) =
        (None, 2u64, 1u32, "unknown".to_string(), Vec::new());
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => match next_value(&mut it, "--out") {
                Ok(v) => out = Some(v.to_string()),
                Err(code) => return code,
            },
            "--secs" => match parse_value(&mut it, "--secs") {
                Ok(v) => secs = v,
                Err(code) => return code,
            },
            "--repeat" => match parse_value::<u32>(&mut it, "--repeat") {
                Ok(v) => repeat = v.max(1),
                Err(code) => return code,
            },
            "--commit" => match next_value(&mut it, "--commit") {
                Ok(v) => commit = v.to_string(),
                Err(code) => return code,
            },
            n => names.push(n.to_string()),
        }
    }
    if names.is_empty() {
        names = SCENARIOS.iter().map(|s| s.to_string()).collect();
    }
    let slow = std::env::var("AGORA_BENCH_SLOWDOWN_US")
        .ok()
        .and_then(|v| v.parse().ok())
        .map_or(Duration::ZERO, Duration::from_micros);
    let mut scenarios = Vec::new();
    for n in &names {
        let mut best: Option<agora_bench::ScenarioResult> = None;
        for _ in 0..repeat {
            let Some(r) = run_scenario(n, Duration::from_secs(secs), slow) else {
                eprintln!("unknown scenario, or the PTY could not start: {n}");
                return ExitCode::from(2);
            };
            best = Some(match best {
                None => r,
                Some(b) => agora_bench::best_of(b, r),
            });
        }
        scenarios.extend(best);
    }
    let report = Report { commit, scenarios };
    let json = match serde_json::to_string_pretty(&report) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("agora-bench: {e}");
            return ExitCode::from(1);
        }
    };
    match out {
        Some(path) => {
            if let Err(e) = std::fs::write(&path, json) {
                eprintln!("agora-bench: {path}: {e}");
                return ExitCode::from(1);
            }
        }
        None => println!("{json}"),
    }
    ExitCode::SUCCESS
}

fn bestof(args: &[String]) -> ExitCode {
    let mut files = Vec::new();
    let mut out = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--out" {
            match next_value(&mut it, "--out") {
                Ok(v) => out = Some(v.to_string()),
                Err(code) => return code,
            }
        } else {
            files.push(a.as_str());
        }
    }
    let mut merged: Option<Report> = None;
    for f in files {
        let r = match load(f) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("agora-bench: {e}");
                return ExitCode::from(2);
            }
        };
        merged = Some(match merged {
            None => r,
            Some(m) => agora_bench::merge_reports(m, r),
        });
    }
    let Some(report) = merged else {
        eprintln!("usage: agora-bench bestof A.json B.json... [--out FILE]");
        return ExitCode::from(2);
    };
    match serde_json::to_string_pretty(&report) {
        Ok(j) => match out {
            Some(p) => match std::fs::write(&p, j) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("agora-bench: {p}: {e}");
                    ExitCode::from(1)
                }
            },
            None => {
                println!("{j}");
                ExitCode::SUCCESS
            }
        },
        Err(e) => {
            eprintln!("agora-bench: {e}");
            ExitCode::from(1)
        }
    }
}

fn load(path: &str) -> Result<Report, String> {
    let s = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    serde_json::from_str(&s).map_err(|e| format!("{path}: {e}"))
}

fn cmp(args: &[String]) -> ExitCode {
    let mut files = Vec::new();
    let mut threshold = 10.0;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--threshold" {
            let v: f64 = match parse_value(&mut it, "--threshold") {
                Ok(v) => v,
                Err(code) => return code,
            };
            if !v.is_finite() || v < 0.0 {
                eprintln!("agora-bench: invalid value for --threshold: {v}");
                return ExitCode::from(2);
            }
            threshold = v;
        } else {
            files.push(a.as_str());
        }
    }
    let [base, new] = files[..] else {
        eprintln!("usage: agora-bench compare BASE.json NEW.json [--threshold PCT]");
        return ExitCode::from(2);
    };
    match (load(base), load(new)) {
        (Ok(b), Ok(n)) => {
            let regs = compare(&b, &n, threshold);
            for r in &regs {
                eprintln!(
                    "regression: {} {} {:+.1}%",
                    r.scenario, r.metric, r.change_pct
                );
            }
            if regs.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("agora-bench: {e}");
            ExitCode::from(2)
        }
    }
}

/// `agora-bench history [--runs N] [--commit SHA] [--out FILE] [--check]`
/// Measures the history replay a window may ask for (ADR-0100 decision 2): the lock hold for a full 2000-line, 200-column history,
/// the bytes per attach, and the terminal snapshot after switching with a full history (budget 150 ms). With `--check`, exits 1 if
/// the switch is over that budget.
/// `agora-bench transcript [--runs N] [--commit SHA] [--out FILE]` (ADR-0089, ADR-0104 C1).
fn transcript(args: &[String]) -> ExitCode {
    let (mut runs, mut commit, mut out) = (20usize, "unknown".to_string(), None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--runs" => match parse_value::<usize>(&mut it, "--runs") {
                Ok(v) => runs = v.max(1),
                Err(code) => return code,
            },
            "--commit" => match next_value(&mut it, "--commit") {
                Ok(v) => commit = v.to_string(),
                Err(code) => return code,
            },
            "--out" => match next_value(&mut it, "--out") {
                Ok(v) => out = Some(v.to_string()),
                Err(code) => return code,
            },
            _ => {
                eprintln!("usage: agora-bench transcript [--runs N] [--commit SHA] [--out FILE]");
                return ExitCode::from(2);
            }
        }
    }
    let r = match agora_bench::transcript::run(&commit, runs) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("agora-bench: transcript: {e}");
            return ExitCode::from(1);
        }
    };
    println!(
        "push one entry: memory only {:.0} ns, saving {:.0} ns | a 16 KiB one evicting: memory only {:.0} ns, saving {:.0} ns | take a batch of {} under the lock: p50 {:.1} us, max {:.1} us | write it: p50 {:.2} ms, max {:.2} ms | seed from a {} byte file: p50 {:.2} ms, max {:.2} ms",
        r.push_memory_ns, r.push_saving_ns, r.push_large_memory_ns, r.push_large_saving_ns, r.batch_entries, r.take_p50_us, r.take_max_us, r.write_p50_ms, r.write_max_ms,
        r.seed_file_bytes, r.seed_p50_ms, r.seed_max_ms
    );
    println!(
        "with a 52-byte secret: take {:.1} us, write {:.2} ms | with an 8 KiB secret: take {:.1} us, write {:.2} ms (p50, one stream, newest pieces held)",
        r.take_52_p50_us, r.write_52_p50_ms, r.take_8k_p50_us, r.write_8k_p50_ms
    );
    println!(
        "a 16 KiB output dense with edges, an 8 KiB value held: write {:.2} ms (p50)",
        r.write_dense_8k_p50_ms
    );
    if let Some(path) = out {
        match serde_json::to_string_pretty(&r) {
            Ok(json) if std::fs::write(&path, &json).is_ok() => {}
            _ => {
                eprintln!("agora-bench: could not write {path}");
                return ExitCode::from(1);
            }
        }
    }
    ExitCode::SUCCESS
}

fn history(args: &[String]) -> ExitCode {
    let (mut runs, mut commit, mut out, mut check) = (20usize, "unknown".to_string(), None, false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--runs" => match parse_value::<usize>(&mut it, "--runs") {
                Ok(v) => runs = v.max(1),
                Err(code) => return code,
            },
            "--commit" => match next_value(&mut it, "--commit") {
                Ok(v) => commit = v.to_string(),
                Err(code) => return code,
            },
            "--out" => match next_value(&mut it, "--out") {
                Ok(v) => out = Some(v.to_string()),
                Err(code) => return code,
            },
            "--check" => check = true,
            _ => {
                eprintln!(
                    "usage: agora-bench history [--runs N] [--commit SHA] [--out FILE] [--check]"
                );
                return ExitCode::from(2);
            }
        }
    }
    let report = match agora_bench::history::run(&commit, runs) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("agora-bench: history: {e}");
            return ExitCode::from(1);
        }
    };
    for c in &report.capture {
        println!(
            "{:<28} kept {:>4} replayed {:>4} lines | lock p50 {:>6.2} ms max {:>6.2} ms | format p50 {:>6.2} ms | {:>7} bytes in {:>2} frames",
            c.content, c.kept_lines, c.replayed_lines, c.lock_p50_ms, c.lock_max_ms, c.format_p50_ms, c.bytes, c.frames
        );
    }
    println!(
        "attach with a full history: {} bytes of history in {} frames, {} bytes of repaint",
        report.attach_history_bytes, report.attach_history_frames, report.attach_repaint_bytes
    );
    let over =
        report.switch_history_p95_ms > 150.0 || report.switch_history_new_connection_p95_ms > 150.0;
    println!(
        "snapshot after switching, full history: p50 {:.2} ms p95 {:.2} ms (budget 150 ms) {}; without history: p50 {:.2} ms p95 {:.2} ms",
        report.switch_history_p50_ms,
        report.switch_history_p95_ms,
        if over { "OVER BUDGET" } else { "ok" },
        report.switch_plain_p50_ms,
        report.switch_plain_p95_ms
    );
    println!(
        "  on a new connection per subscription (as the uplink routes one): full history p50 {:.2} ms p95 {:.2} ms, without history p50 {:.2} ms p95 {:.2} ms",
        report.switch_history_new_connection_p50_ms,
        report.switch_history_new_connection_p95_ms,
        report.switch_plain_new_connection_p50_ms,
        report.switch_plain_new_connection_p95_ms
    );
    if let Some(path) = out {
        match serde_json::to_string_pretty(&report) {
            Ok(j) => {
                if let Err(e) = std::fs::write(&path, j) {
                    eprintln!("agora-bench: {path}: {e}");
                    return ExitCode::from(1);
                }
            }
            Err(e) => {
                eprintln!("agora-bench: {e}");
                return ExitCode::from(1);
            }
        }
    }
    if check && over {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// `agora-bench footprint [--terminals N] [--idle-secs S] [--commit SHA] [--out FILE] [--check]`
/// Measures a node against the footprint budgets (P2-15). With `--check`, exits 1 if any budget is exceeded.
fn footprint(args: &[String]) -> ExitCode {
    let (mut terminals, mut idle_secs, mut commit, mut out, mut check) =
        (50usize, 5u64, "unknown".to_string(), None, false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--terminals" => match parse_value(&mut it, "--terminals") {
                Ok(v) => terminals = v,
                Err(code) => return code,
            },
            "--idle-secs" => match parse_value(&mut it, "--idle-secs") {
                Ok(v) => idle_secs = v,
                Err(code) => return code,
            },
            "--commit" => match next_value(&mut it, "--commit") {
                Ok(v) => commit = v.to_string(),
                Err(code) => return code,
            },
            "--out" => match next_value(&mut it, "--out") {
                Ok(v) => out = Some(v.to_string()),
                Err(code) => return code,
            },
            "--check" => check = true,
            _ => {
                eprintln!("usage: agora-bench footprint [--terminals N] [--idle-secs S] [--commit SHA] [--out FILE] [--check]");
                return ExitCode::from(2);
            }
        }
    }
    let report =
        match agora_bench::footprint::run(terminals, Duration::from_secs(idle_secs), &commit) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("agora-bench: footprint: {e}");
                return ExitCode::from(1);
            }
        };
    let mut failed = false;
    for b in agora_bench::footprint::budgets(&report) {
        failed |= !b.ok();
        println!(
            "{:<28} {:>9.2} {:<2} (budget {:>6.1} {:<2}) {}",
            b.name,
            b.value,
            b.unit,
            b.limit,
            b.unit,
            if b.ok() { "ok" } else { "OVER BUDGET" }
        );
    }
    if let Some(path) = out {
        match serde_json::to_string_pretty(&report) {
            Ok(j) => {
                if let Err(e) = std::fs::write(&path, j) {
                    eprintln!("agora-bench: {path}: {e}");
                    return ExitCode::from(1);
                }
            }
            Err(e) => {
                eprintln!("agora-bench: {e}");
                return ExitCode::from(1);
            }
        }
    }
    if check && failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::unwrap_used)]
    fn compare_rejects_invalid_and_missing_threshold_values() {
        let dir = std::env::temp_dir().join(format!("agora-bench-f31-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("r.json");
        std::fs::write(&path, r#"{"commit":"abc","scenarios":[]}"#).unwrap();
        let p = path.to_string_lossy().into_owned();

        assert_eq!(
            cmp(&[p.clone(), p.clone(), "--threshold".into(), "10".into()]),
            ExitCode::SUCCESS
        );
        assert_eq!(
            cmp(&[p.clone(), p.clone(), "--threshold".into(), "invalid".into()]),
            ExitCode::from(2)
        );
        assert_eq!(
            cmp(&[p.clone(), p.clone(), "--threshold".into(), "NaN".into()]),
            ExitCode::from(2)
        );
        assert_eq!(
            cmp(&[p.clone(), p.clone(), "--threshold".into()]),
            ExitCode::from(2)
        );
        assert_eq!(bestof(&[p, "--out".into()]), ExitCode::from(2));
        // `history` (added on main after the audit) reads its flags the same way: a bad or missing value is refused before any run.
        assert_eq!(
            history(&["--runs".into(), "many".into()]),
            ExitCode::from(2)
        );
        assert_eq!(history(&["--out".into()]), ExitCode::from(2));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
