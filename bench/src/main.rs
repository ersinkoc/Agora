//! `agora-bench run [--out FILE] [--secs N] [--commit SHA] [scenario...]`
//! `agora-bench compare BASE.json NEW.json [--threshold PCT]` (exit 1 on regression)
//!
//! `AGORA_BENCH_SLOWDOWN_US` deliberately slows the hot path, to prove the gate works.

use std::process::ExitCode;
use std::time::Duration;

use agora_bench::{compare, run_scenario, Report, SCENARIOS};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("run") => run(&args[1..]),
        Some("compare") => cmp(&args[1..]),
        _ => {
            eprintln!("usage: agora-bench run [--out F] [--secs N] [--commit SHA] [scenario...] | compare BASE NEW [--threshold PCT]");
            ExitCode::from(2)
        }
    }
}

fn run(args: &[String]) -> ExitCode {
    let (mut out, mut secs, mut commit, mut names) =
        (None, 2u64, "unknown".to_string(), Vec::new());
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => out = it.next().cloned(),
            "--secs" => secs = it.next().and_then(|s| s.parse().ok()).unwrap_or(secs),
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
        match run_scenario(n, Duration::from_secs(secs), slow) {
            Some(r) => scenarios.push(r),
            None => {
                eprintln!("unknown scenario: {n}");
                return ExitCode::from(2);
            }
        }
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
