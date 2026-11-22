//! Instrumentation for the ADR-0099 fixtures, all on the main thread.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::g::Window;

pub fn epoch_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
}

#[derive(Clone, Copy, Debug)]
pub struct EchoSample {
    /// Key handled -> first output byte after it reached the emulator (network thread).
    pub to_recv_ms: f64,
    /// Key handled -> the frame that first contains it painted (before present).
    pub to_paint_ms: f64,
    /// Key handled -> the next frame callback after that frame, i.e. after it was handed to the compositor (upper bound of "on screen").
    pub to_present_ms: f64,
}

#[derive(Default)]
pub struct Metrics {
    pub recording: bool,
    /// (pane, time) of every paint while recording.
    pub paints: Vec<(usize, Instant)>,
    /// Paint cost of each pane paint while recording (µs).
    pub paint_us: Vec<u32>,
    /// Bytes each pane sent, while `tap_on` (self-test).
    pub tap_on: bool,
    pub tapped: Vec<(usize, Vec<u8>)>,
    pub echo_pane: Option<usize>,
    pending_echo: Option<(usize, Instant, u64)>,
    pub echo: Vec<EchoSample>,
    pub heartbeat_late_ms: Vec<f64>,
    pub first_frame_ms: Option<u128>,
    pub first_useful_ms: Option<u128>,
    pub copied: Option<String>,
    pub preedit_seen: Option<String>,
    pub preedit_painted: bool,
    pub wheel_lines: Vec<f32>,
    /// Pane -> when its terminal was asked for (switch to a terminal).
    pub open_started: HashMap<usize, Instant>,
    pub open_to_screen_ms: Vec<f64>,
    /// Pane -> when it was brought to the front (switch between open terminals).
    pub switch_started: HashMap<usize, Instant>,
    pub switch_ms: Vec<f64>,
    pub no_clipboard: bool,
    pub window_active_samples: Vec<bool>,
    /// Every pane paint since start.
    pub paints_total: u64,
    /// One entry per window frame that painted any pane while recording (a frame ends at the next frame callback).
    pub frames: Vec<Instant>,
    frame_open: bool,
}

impl Metrics {
    pub fn tap(&mut self, pane: usize, bytes: &[u8]) {
        if self.tap_on {
            self.tapped.push((pane, bytes.to_vec()));
        }
    }

    /// Bytes were sent to `pane` whose emulator was at `generation`.
    pub fn sent(&mut self, pane: usize, generation: u64, at: Instant) {
        if self.echo_pane == Some(pane) && self.pending_echo.is_none() {
            self.pending_echo = Some((pane, at, generation));
        }
    }

    pub fn clear_echo(&mut self) {
        self.pending_echo = None;
    }

    pub fn note_wheel(&mut self, lines: f32) {
        self.wheel_lines.push(lines);
    }
}

/// What a pane's paint tells the metrics.
pub struct Painted {
    pub pane: usize,
    pub generation: u64,
    pub got_screen: bool,
    pub first_after_send: Option<Instant>,
    pub with_preedit: bool,
    pub started: Instant,
}

pub fn painted(m: &Rc<RefCell<Metrics>>, p: Painted, window: &mut Window) {
    let now = Instant::now();
    let mut g = m.borrow_mut();
    g.paints_total += 1;
    if p.with_preedit {
        g.preedit_painted = true;
    }
    if g.recording {
        g.paints.push((p.pane, now));
        if !g.frame_open {
            g.frame_open = true;
            g.frames.push(now);
            let mc = m.clone();
            window.on_next_frame(move |_, _| mc.borrow_mut().frame_open = false);
        }
        g.paint_us.push(
            now.duration_since(p.started)
                .as_micros()
                .min(u128::from(u32::MAX)) as u32,
        );
    }
    if p.got_screen {
        if let Some(t0) = g.open_started.remove(&p.pane) {
            g.open_to_screen_ms.push(ms(now - t0));
        }
    }
    if let Some(t0) = g.switch_started.remove(&p.pane) {
        let mc = m.clone();
        window.on_next_frame(move |_, _| mc.borrow_mut().switch_ms.push(ms(t0.elapsed())));
    }
    if let Some((pane, t0, gen)) = g.pending_echo {
        if pane == p.pane && p.generation > gen {
            g.pending_echo = None;
            let to_recv = p
                .first_after_send
                .map_or(f64::NAN, |r| ms(r.saturating_duration_since(t0)));
            let to_paint = ms(now - t0);
            let mc = m.clone();
            window.on_next_frame(move |_, _| {
                mc.borrow_mut().echo.push(EchoSample {
                    to_recv_ms: to_recv,
                    to_paint_ms: to_paint,
                    to_present_ms: ms(t0.elapsed()),
                })
            });
        }
    }
}

pub fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

pub fn percentile(v: &[f64], p: f64) -> f64 {
    let mut s: Vec<f64> = v.iter().copied().filter(|x| x.is_finite()).collect();
    if s.is_empty() {
        return f64::NAN;
    }
    s.sort_by(|a, b| a.total_cmp(b));
    let idx = ((p / 100.0) * (s.len() - 1) as f64).round() as usize;
    s[idx.min(s.len() - 1)]
}

/// Frames per second and gaps for one pane's paints (`pane`), or for a list of frame times (`pane` `None`).
pub fn frame_stats(
    paints: &[(usize, Instant)],
    pane: Option<usize>,
    span: Duration,
) -> serde_json::Value {
    let mut ts: Vec<Instant> = paints
        .iter()
        .filter(|(p, _)| pane.is_none_or(|x| x == *p))
        .map(|(_, t)| *t)
        .collect();
    ts.sort();
    let gaps: Vec<f64> = ts.windows(2).map(|w| ms(w[1] - w[0])).collect();
    serde_json::json!({
        "frames": ts.len(),
        "fps": ts.len() as f64 / span.as_secs_f64(),
        "gap_p50_ms": percentile(&gaps, 50.0),
        "gap_p95_ms": percentile(&gaps, 95.0),
        "gap_max_ms": gaps.iter().copied().fold(0.0, f64::max),
        "gaps_over_50ms": gaps.iter().filter(|g| **g > 50.0).count(),
    })
}
