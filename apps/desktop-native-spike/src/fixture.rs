//! The ADR-0099 measurement fixtures (F1-F5, start, switching) and the correctness self-test, run inside the app so every system
//! runs them unchanged. Each run writes one JSON result; the driver (`run`) adds what only the outside can see (memory, CPU, start
//! from process spawn) and collects the files.

use std::cell::RefCell;
use std::io::Write as _;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::app::Root;
use crate::g::*;
use crate::metrics::{epoch_ms, frame_stats, ms, percentile, Metrics};
use crate::term::TermPane;
use agora_vt::Terminal as _;

#[derive(Clone, Default, serde::Deserialize)]
pub struct Stack {
    pub echo: Vec<String>,
    pub flood: Vec<String>,
    pub trickle: Vec<String>,
    #[serde(default)]
    pub probe: Vec<String>,
    #[serde(default)]
    pub flood_secs: u64,
}

pub struct Opts {
    pub fixture: String,
    pub result: PathBuf,
    pub stack: Stack,
    pub hold_secs: u64,
    pub main_ms: u128,
}

pub fn phase(name: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "SPIKE-PHASE {name} {}", epoch_ms());
    let _ = out.flush();
}

async fn sleep(cx: &mut AsyncWindowContext, ms: u64) {
    cx.background_executor()
        .timer(Duration::from_millis(ms))
        .await;
}

type This = WeakEntity<Root>;

fn metrics(this: &This, cx: &mut AsyncWindowContext) -> Rc<RefCell<Metrics>> {
    this.read_with(cx, |r, _| r.metrics.clone())
        .unwrap_or_default()
}

fn pane(this: &This, cx: &mut AsyncWindowContext, id: usize) -> Option<Entity<TermPane>> {
    this.read_with(cx, |r, _| r.panes.get(id).cloned())
        .ok()
        .flatten()
}

async fn wait_agents(this: &This, cx: &mut AsyncWindowContext, n: usize) -> bool {
    let end = Instant::now() + Duration::from_secs(30);
    while Instant::now() < end {
        if this.read_with(cx, |r, _| r.agents.len()).unwrap_or(0) >= n {
            return true;
        }
        sleep(cx, 20).await;
    }
    false
}

fn open(this: &This, cx: &mut AsyncWindowContext, agent: &str, front: bool) -> usize {
    this.update(cx, |r, cx| r.open(agent, front, cx))
        .unwrap_or(usize::MAX)
}

async fn wait_screens(this: &This, cx: &mut AsyncWindowContext, ids: &[usize], secs: u64) -> usize {
    let end = Instant::now() + Duration::from_secs(secs);
    loop {
        let ready = ids
            .iter()
            .filter(|id| {
                pane(this, cx, **id)
                    .is_some_and(|p| p.read_with(cx, |p, _| p.shared.lock().got_screen))
            })
            .count();
        if ready == ids.len() || Instant::now() > end {
            return ready;
        }
        sleep(cx, 20).await;
    }
}

fn send(this: &This, cx: &mut AsyncWindowContext, id: usize, bytes: &[u8]) {
    if let Some(p) = pane(this, cx, id) {
        let b = bytes.to_vec();
        p.update(cx, |p, _| p.send(b));
    }
}

fn focus(this: &This, cx: &mut AsyncWindowContext, id: usize) {
    if let Some(p) = pane(this, cx, id) {
        let fh = p.read_with(cx, |p, _| p.focus.clone());
        let _ = cx.update(|window, cx| window.focus(&fh, cx));
    }
}

fn bytes_of(this: &This, cx: &mut AsyncWindowContext, id: usize) -> u64 {
    pane(this, cx, id).map_or(0, |p| p.read_with(cx, |p, _| p.shared.lock().bytes))
}

fn parse_ms(this: &This, cx: &mut AsyncWindowContext, id: usize) -> f64 {
    pane(this, cx, id).map_or(0.0, |p| {
        p.read_with(cx, |p, _| {
            p.shared.parse_ns.load(std::sync::atomic::Ordering::Relaxed) as f64 / 1e6
        })
    })
}

fn tail_of(this: &This, cx: &mut AsyncWindowContext, id: usize) -> Vec<String> {
    pane(this, cx, id).map_or(Vec::new(), |p| {
        p.read_with(cx, |p, _| p.shared.lock().term.bottom_non_empty_lines(2))
    })
}

fn behind_of(this: &This, cx: &mut AsyncWindowContext, id: usize) -> u64 {
    pane(this, cx, id).map_or(0, |p| p.read_with(cx, |p, _| p.shared.lock().behind))
}

fn gpu(cx: &mut AsyncWindowContext) -> Value {
    cx.update(|window, _| {
        window.gpu_specs().map_or(json!(null), |g| {
            json!({"device": g.device_name, "driver": g.driver_name, "driver_info": g.driver_info, "software": g.is_software_emulated})
        })
    })
    .unwrap_or(json!(null))
}

fn active(cx: &mut AsyncWindowContext) -> bool {
    cx.update(|window, _| window.is_window_active())
        .unwrap_or(false)
}

/// Records `secs` of frames, stalls and paint cost.
async fn record(this: &This, cx: &mut AsyncWindowContext, secs: u64, panes: &[usize]) -> Value {
    let m = metrics(this, cx);
    let b0: Vec<u64> = panes.iter().map(|p| bytes_of(this, cx, *p)).collect();
    let p0: Vec<f64> = panes.iter().map(|p| parse_ms(this, cx, *p)).collect();
    let k0: Vec<u64> = panes.iter().map(|p| behind_of(this, cx, *p)).collect();
    {
        let mut g = m.borrow_mut();
        g.paints.clear();
        g.frames.clear();
        g.paint_us.clear();
        g.heartbeat_late_ms.clear();
        g.window_active_samples.clear();
        g.recording = true;
    }
    let t0 = Instant::now();
    let mut active_n = 0;
    let mut samples = 0;
    while t0.elapsed() < Duration::from_secs(secs) {
        sleep(cx, 250).await;
        samples += 1;
        if active(cx) {
            active_n += 1;
        }
    }
    let span = t0.elapsed();
    m.borrow_mut().recording = false;
    let b1: Vec<u64> = panes.iter().map(|p| bytes_of(this, cx, *p)).collect();
    let p1: Vec<f64> = panes.iter().map(|p| parse_ms(this, cx, *p)).collect();
    let k1: Vec<u64> = panes.iter().map(|p| behind_of(this, cx, *p)).collect();
    let g = m.borrow();
    let late = &g.heartbeat_late_ms;
    let paint: Vec<f64> = g.paint_us.iter().map(|u| f64::from(*u) / 1000.0).collect();
    let per_pane: Vec<Value> = panes
        .iter()
        .enumerate()
        .map(|(i, p)| {
            json!({
                "pane": p,
                "frames": frame_stats(&g.paints, Some(*p), span),
                "received_mb_per_s": (b1[i] - b0[i]) as f64 / 1e6 / span.as_secs_f64(),
                "parse_ms_per_s_network_thread": (p1[i] - p0[i]) / span.as_secs_f64(),
                "node_said_behind_and_rewatched": k1[i] - k0[i],
                "screen_tail_at_end": tail_of(this, cx, *p),
            })
        })
        .collect();
    json!({
        "seconds": span.as_secs_f64(),
        "window_frames": frame_stats(&g.frames.iter().map(|t| (0, *t)).collect::<Vec<_>>(), None, span),
        "per_pane": per_pane,
        "main_thread_heartbeat": {
            "interval_ms": 10,
            "samples": late.len(),
            "late_p50_ms": percentile(late, 50.0),
            "late_p95_ms": percentile(late, 95.0),
            "late_max_ms": late.iter().copied().fold(0.0, f64::max),
            "stalls_over_50ms": late.iter().filter(|l| **l > 50.0).count(),
        },
        "pane_paint_ms": {"p50": percentile(&paint, 50.0), "p95": percentile(&paint, 95.0), "max": paint.iter().copied().fold(0.0, f64::max), "count": paint.len()},
        "window_active_fraction": if samples == 0 { 0.0 } else { f64::from(active_n) / f64::from(samples) },
    })
}

pub fn start(window: &mut Window, cx: &mut Context<Root>, m: Rc<RefCell<Metrics>>, opts: Opts) {
    // Main-thread heartbeat: a 10 ms timer on the foreground executor; lateness is time the main thread was busy (a stall).
    let hb = m.clone();
    cx.spawn(async move |_, cx| loop {
        // Only while recording: an idle window must not be woken 100 times a second by its own instrumentation.
        if !hb.borrow().recording {
            cx.background_executor()
                .timer(Duration::from_millis(250))
                .await;
            continue;
        }
        let t = Instant::now();
        cx.background_executor()
            .timer(Duration::from_millis(10))
            .await;
        let late = ms(t.elapsed()) - 10.0;
        let mut g = hb.borrow_mut();
        if g.recording {
            g.heartbeat_late_ms.push(late.max(0.0));
        }
    })
    .detach();
    cx.spawn_in(window, async move |this, cx| {
        let mut out = match opts.fixture.as_str() {
            "start" => fx_start(&this, cx).await,
            "f1" => fx_f1(&this, cx, &opts.stack).await,
            "f2" => fx_f2(&this, cx, &opts.stack).await,
            "f3" => fx_f3(&this, cx, &opts.stack).await,
            "switch" => fx_switch(&this, cx, &opts.stack).await,
            "f4" => fx_f4(&this, cx, &opts.stack).await,
            "selftest" => crate::selftest::run(&this, cx, &opts.stack).await,
            "hold" | "a11y" => fx_hold(&this, cx, &opts.stack, opts.hold_secs).await,
            other => json!({"error": format!("unknown fixture {other}")}),
        };
        let m = metrics(&this, cx);
        let g = m.borrow();
        let errors = this
            .read_with(cx, |r, _| r.open_errors.clone())
            .unwrap_or_default();
        if let Value::Object(o) = &mut out {
            o.insert("fixture".into(), json!(opts.fixture));
            o.insert("main_entered_epoch_ms".into(), json!(opts.main_ms as u64));
            o.insert(
                "first_frame_epoch_ms".into(),
                json!(g.first_frame_ms.map(|v| v as u64)),
            );
            o.insert(
                "first_useful_paint_epoch_ms".into(),
                json!(g.first_useful_ms.map(|v| v as u64)),
            );
            o.insert("open_errors".into(), json!(errors));
        }
        drop(g);
        let gpu = gpu(cx);
        if let Value::Object(o) = &mut out {
            o.insert("gpu".into(), gpu);
        }
        let _ = std::fs::write(
            &opts.result,
            serde_json::to_vec_pretty(&out).unwrap_or_default(),
        );
        phase("done");
        let _ = cx.update(|_, cx| cx.quit());
    })
    .detach();
}

async fn fx_start(this: &This, cx: &mut AsyncWindowContext) -> Value {
    wait_agents(this, cx, 1).await;
    let m = metrics(this, cx);
    let end = Instant::now() + Duration::from_secs(10);
    while m.borrow().first_useful_ms.is_none() && Instant::now() < end {
        sleep(cx, 5).await;
    }
    sleep(cx, 200).await;
    json!({})
}

async fn fx_f1(this: &This, cx: &mut AsyncWindowContext, s: &Stack) -> Value {
    wait_agents(this, cx, 1).await;
    let Some(agent) = s.echo.first() else {
        return json!({"error": "no echo agent"});
    };
    let id = open(this, cx, agent, true);
    wait_screens(this, cx, &[id], 15).await;
    sleep(cx, 800).await;
    focus(this, cx, id);
    sleep(cx, 200).await;
    let m = metrics(this, cx);
    let keys: [&'static str; 6] = ["a", "s", "d", "f", "j", "k"];
    let mut timeouts = 0;
    let total = 300;
    for i in 0..total {
        if i % 50 == 49 {
            m.borrow_mut().echo_pane = None;
            let _ = cx.update(|window, cx| {
                window.dispatch_keystroke(
                    Keystroke::parse("enter").unwrap_or_else(|_| Keystroke::default()),
                    cx,
                )
            });
            sleep(cx, 120).await;
        }
        let before = m.borrow().echo.len();
        {
            let mut g = m.borrow_mut();
            g.clear_echo();
            g.echo_pane = Some(id);
        }
        let ks = Keystroke::parse(keys[i % keys.len()]).unwrap_or_else(|_| Keystroke::default());
        let _ = cx.update(|window, cx| window.dispatch_keystroke(ks, cx));
        let end = Instant::now() + Duration::from_millis(500);
        while m.borrow().echo.len() == before && Instant::now() < end {
            sleep(cx, 1).await;
        }
        if m.borrow().echo.len() == before {
            timeouts += 1;
        }
        // Irregular spacing (30-75 ms), like typing.
        sleep(cx, 30 + ((i * 37) % 46) as u64).await;
    }
    m.borrow_mut().echo_pane = None;
    let g = m.borrow();
    let recv: Vec<f64> = g.echo.iter().map(|e| e.to_recv_ms).collect();
    let paint: Vec<f64> = g.echo.iter().map(|e| e.to_paint_ms).collect();
    let present: Vec<f64> = g.echo.iter().map(|e| e.to_present_ms).collect();
    let stat = |v: &[f64]| json!({"p50": percentile(v, 50.0), "p95": percentile(v, 95.0), "max": v.iter().copied().fold(0.0, f64::max)});
    drop(g);
    json!({
        "samples": present.len(),
        "timeouts": timeouts,
        "key_to_output_received_ms": stat(&recv),
        "key_to_frame_painted_ms": stat(&paint),
        "key_to_frame_presented_ms": stat(&present),
        "window_active": active(cx),
        "method": "keystrokes dispatched into the window's input path (Window::dispatch_keystroke -> key handler -> input handler -> \
                   network thread -> hub -> node PTY echo -> back); 'presented' is the next frame callback after the frame that first \
                   contains the echo was drawn and handed to the platform (an upper bound of the frame being on screen, excluding \
                   OS input-event latency and display scan-out)",
    })
}

async fn trigger(this: &This, cx: &mut AsyncWindowContext, ids: &[usize]) {
    for id in ids {
        send(this, cx, *id, b"g\r");
    }
}

async fn fx_f2(this: &This, cx: &mut AsyncWindowContext, s: &Stack) -> Value {
    wait_agents(this, cx, 21).await;
    if s.flood.is_empty() || s.trickle.len() < 20 {
        return json!({"error": "stack needs 1 flood and 20 trickle agents"});
    }
    let flood = open(this, cx, &s.flood[0], true);
    let mut ids = vec![flood];
    for a in &s.trickle[..20] {
        ids.push(open(this, cx, a, false));
    }
    let ready = wait_screens(this, cx, &ids, 20).await;
    sleep(cx, 500).await;
    trigger(this, cx, &ids).await;
    sleep(cx, 2000).await;
    phase("f2-record");
    let rec = record(this, cx, 10, &ids).await;
    let bg_total: f64 = rec["per_pane"].as_array().map_or(0.0, |a| {
        a.iter()
            .skip(1)
            .map(|p| p["received_mb_per_s"].as_f64().unwrap_or(0.0))
            .sum()
    });
    sleep(cx, s.flood_secs.saturating_sub(12).max(1) * 1000).await;
    json!({"terminals_ready": ready, "visible_pane": flood, "background_total_mb_per_s": bg_total, "record": rec})
}

async fn fx_f3(this: &This, cx: &mut AsyncWindowContext, s: &Stack) -> Value {
    wait_agents(this, cx, 4).await;
    if s.flood.len() < 4 {
        return json!({"error": "stack needs 4 flood agents"});
    }
    let ids: Vec<usize> = s.flood[..4]
        .iter()
        .map(|a| open(this, cx, a, true))
        .collect();
    let ready = wait_screens(this, cx, &ids, 20).await;
    sleep(cx, 500).await;
    trigger(this, cx, &ids).await;
    sleep(cx, 2000).await;
    phase("f3-record");
    let rec = record(this, cx, 10, &ids).await;
    sleep(cx, s.flood_secs.saturating_sub(12).max(1) * 1000).await;
    json!({"terminals_ready": ready, "record": rec})
}

async fn fx_switch(this: &This, cx: &mut AsyncWindowContext, s: &Stack) -> Value {
    wait_agents(this, cx, 6).await;
    // Switch to a terminal not open yet: open request -> the node's screen painted.
    let mut ids = Vec::new();
    for a in s.echo.iter().chain(s.trickle.iter()).take(6) {
        ids.push(open(this, cx, a, true));
        wait_screens(this, cx, &[*ids.last().unwrap_or(&0)], 10).await;
        sleep(cx, 300).await;
    }
    // Switch between terminals already open (a background one to the front): request -> presented frame.
    for k in 0..12 {
        let hidden: Vec<usize> = this
            .read_with(cx, |r, _| {
                ids.iter()
                    .copied()
                    .filter(|i| !r.visible.contains(i))
                    .collect()
            })
            .unwrap_or_default();
        if let Some(h) = hidden.get(k % hidden.len().max(1)) {
            let h = *h;
            let _ = this.update(cx, |r, cx| r.bring_front(h, cx));
        }
        sleep(cx, 250).await;
    }
    // Switching the selected agent in the list: request -> presented frame (next frame after the one that drew it).
    let agent_switch = Rc::new(RefCell::new(Vec::<f64>::new()));
    for k in 0..12 {
        let t0 = Instant::now();
        let a2 = agent_switch.clone();
        let _ = this.update_in(cx, |r, window, cx| {
            r.selected = (k * 3) % r.agents.len().max(1);
            cx.notify();
            window.on_next_frame(move |w, _| {
                w.on_next_frame(move |_, _| a2.borrow_mut().push(ms(t0.elapsed())))
            });
        });
        sleep(cx, 200).await;
    }
    let m = metrics(this, cx);
    let g = m.borrow();
    let st = |v: &[f64]| json!({"samples": v.len(), "p50": percentile(v, 50.0), "p95": percentile(v, 95.0), "max": v.iter().copied().fold(0.0, f64::max)});
    let a = agent_switch.borrow();
    json!({
        "open_terminal_to_snapshot_painted_ms": st(&g.open_to_screen_ms),
        "switch_open_terminal_to_front_presented_ms": st(&g.switch_ms),
        "switch_selected_agent_presented_ms": st(&a),
    })
}

async fn fx_f4(this: &This, cx: &mut AsyncWindowContext, s: &Stack) -> Value {
    wait_agents(this, cx, 20).await;
    let agents: Vec<String> = s
        .echo
        .iter()
        .take(1)
        .chain(s.trickle.iter().take(5))
        .cloned()
        .collect();
    if agents.len() < 6 {
        return json!({"error": "stack needs 1 echo and 5 trickle agents"});
    }
    let mut ids = Vec::new();
    for (i, a) in agents.iter().enumerate() {
        ids.push(open(this, cx, a, i < 4));
    }
    let ready = wait_screens(this, cx, &ids, 15).await;
    let listed = this.read_with(cx, |r, _| r.agents.len()).unwrap_or(0);
    sleep(cx, 3000).await;
    phase("f4-begin");
    let m = metrics(this, cx);
    let p0 = m.borrow().paints_total;
    let t0 = Instant::now();
    sleep(cx, 11_000).await;
    phase("f4-end");
    let frames = m.borrow().paints_total - p0;
    json!({"agents_listed": listed, "visible": 4, "background": 2, "terminals_ready": ready, "idle_seconds": t0.elapsed().as_secs_f64(), "pane_paints_while_idle": frames, "window_active": active(cx)})
}

async fn fx_hold(this: &This, cx: &mut AsyncWindowContext, s: &Stack, secs: u64) -> Value {
    wait_agents(this, cx, 1).await;
    if let Some(a) = s.echo.first() {
        open(this, cx, a, true);
    }
    let before = this.read_with(cx, |r, _| r.panes.len()).unwrap_or(0);
    phase("hold");
    sleep(cx, secs * 1000).await;
    let after = this.read_with(cx, |r, _| r.panes.len()).unwrap_or(0);
    json!({"held_seconds": secs, "terminals_open_before": before, "terminals_open_after": after})
}
