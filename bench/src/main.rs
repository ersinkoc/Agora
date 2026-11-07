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
        Some("bestof") => bestof(&args[1..]),
        Some("compare") => cmp(&args[1..]),
        _ => {
            eprintln!("usage: agora-bench run [--out F] [--secs N] [--commit SHA] [scenario...] | compare BASE NEW [--threshold PCT]");
            ExitCode::from(2)
        }
    }
}

fn run(args: &[String]) -> ExitCode {
    let (mut out, mut secs, mut repeat, mut commit, mut names) =
        (None, 2u64, 1u32, "unknown".to_string(), Vec::new());
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => out = it.next().cloned(),
            "--secs" => secs = it.next().and_then(|s| s.parse().ok()).unwrap_or(secs),
            "--repeat" => {
                repeat = it
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(repeat)
                    .max(1)
            }
            "--commit" => commit = it.next().cloned().unwrap_or(commit),
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
            out = it.next().cloned();
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
            threshold = it.next().and_then(|s| s.parse().ok()).unwrap_or(threshold);
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

/// `agora-bench footprint [--terminals N] [--idle-secs S] [--commit SHA] [--out FILE] [--check]`
/// Measures a node against the footprint budgets (P2-15). With `--check`, exits 1 if any budget is exceeded.
fn footprint(args: &[String]) -> ExitCode {
    let (mut terminals, mut idle_secs, mut commit, mut out, mut check) =
        (50usize, 5u64, "unknown".to_string(), None, false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--terminals" => {
                terminals = it.next().and_then(|s| s.parse().ok()).unwrap_or(terminals)
            }
            "--idle-secs" => {
                idle_secs = it.next().and_then(|s| s.parse().ok()).unwrap_or(idle_secs)
            }
            "--commit" => commit = it.next().cloned().unwrap_or(commit),
            "--out" => out = it.next().cloned(),
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
