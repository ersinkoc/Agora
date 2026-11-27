//! ADR-0117 step 1: the baseline of screen detection, a hot path that scales with terminal agents (ADR-0022). Every captured screen of
//! the shipped terminal manifests (`manifests/agents/fixtures/<kind>/*.screen`) is replayed onto its own terminal, 50 terminals in all
//! (the screens repeated), and one detection tick, a `Detector::detect` on every terminal, is timed. Two figures: the time of a whole
//! tick (what the supervisor spends per check of 50 agents) and the time of one terminal's detection (what the node holds that
//! terminal's lock for today, since detection reads the grid under it), p50 and max of each over the runs.

use std::path::{Path, PathBuf};
use std::time::Instant;

use agora_adapters::{Detector, Manifest};
use agora_vt::{AlacrittyTerminal, Size, Terminal};
use serde::{Deserialize, Serialize};

/// Terminals per tick.
pub const TERMINALS: usize = 50;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DetectionReport {
    pub commit: String,
    /// Shipped screens replayed, and terminals per tick.
    pub screens: usize,
    pub terminals: usize,
    pub ticks: usize,
    /// One tick over every terminal (p50, max).
    pub tick_p50_us: f64,
    pub tick_max_us: f64,
    /// One terminal's detection, the time its lock is held (p50, max).
    pub one_p50_us: f64,
    pub one_max_us: f64,
}

fn size_of(f: &Path) -> Size {
    let stem = f
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    stem.split_once('@')
        .and_then(|(_, s)| s.split_once('x'))
        .and_then(|(c, r)| {
            Some(Size {
                cols: c.parse().ok()?,
                rows: r.parse().ok()?,
            })
        })
        .unwrap_or(Size {
            cols: 120,
            rows: 40,
        })
}

/// Every terminal manifest with its captured screens, from the repository's `manifests/agents`.
fn shipped(root: &Path) -> std::io::Result<Vec<(Detector, Vec<PathBuf>)>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let path = entry?.path();
        if path.extension().is_none_or(|e| e != "toml") {
            continue;
        }
        let Ok(manifest) = Manifest::parse(&std::fs::read_to_string(&path)?) else {
            continue;
        };
        if manifest.driver(Path::new("agora")).is_some() {
            continue; // driven without a terminal
        }
        let kind = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let dir = root.join("fixtures").join(&kind);
        let Ok(files) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut screens: Vec<PathBuf> = files
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "screen"))
            .collect();
        screens.sort();
        if !screens.is_empty() {
            out.push((Detector::new(&manifest), screens));
        }
    }
    Ok(out)
}

fn percentile(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    let i = ((v.len() - 1) as f64 * p).round() as usize;
    v[i.min(v.len() - 1)]
}

pub fn run(commit: &str, ticks: usize) -> std::io::Result<DetectionReport> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../manifests/agents");
    let kinds = shipped(&root)?;
    let all: Vec<(usize, &PathBuf)> = kinds
        .iter()
        .enumerate()
        .flat_map(|(k, (_, screens))| screens.iter().map(move |s| (k, s)))
        .collect();
    if all.is_empty() {
        return Err(std::io::Error::other("no shipped screens"));
    }
    let mut terms: Vec<(usize, AlacrittyTerminal)> = Vec::with_capacity(TERMINALS);
    for i in 0..TERMINALS {
        let (k, f) = all[i % all.len()];
        let mut t = AlacrittyTerminal::new(size_of(f));
        t.feed(&std::fs::read(f)?);
        terms.push((k, t));
    }
    let (mut tick, mut one) = (
        Vec::with_capacity(ticks),
        Vec::with_capacity(ticks * TERMINALS),
    );
    let mut seen = 0usize;
    for _ in 0..ticks.max(1) {
        let start = Instant::now();
        for (k, t) in &terms {
            let at = Instant::now();
            let d = kinds[*k].0.detect(t as &dyn Terminal);
            one.push(at.elapsed().as_secs_f64() * 1e6);
            seen += usize::from(d.is_some());
        }
        tick.push(start.elapsed().as_secs_f64() * 1e6);
    }
    std::hint::black_box(seen);
    Ok(DetectionReport {
        commit: commit.to_string(),
        screens: all.len(),
        terminals: TERMINALS,
        ticks: ticks.max(1),
        tick_p50_us: percentile(&mut tick.clone(), 0.5),
        tick_max_us: percentile(&mut tick, 1.0),
        one_p50_us: percentile(&mut one.clone(), 0.5),
        one_max_us: percentile(&mut one, 1.0),
    })
}
