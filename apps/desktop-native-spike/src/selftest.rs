//! Correctness checks of the pane's input path, driven through the window's own event dispatch (`Window::dispatch_event`,
//! `Window::dispatch_keystroke`) so GPUI's hit testing, focus and listeners are part of what is checked. The program's modes are
//! set by feeding its output into the pane's emulator and mode scanner (as the stream does), the bytes the pane sends are tapped
//! and compared with the forms in `docs/ux/terminal-fidelity.md`. One check goes end to end on Unix: a real program (`od`) behind
//! the node's PTY turns the mouse on itself and prints what it received.

use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::app::Root;
use crate::fixture::Stack;
use crate::g::*;
use crate::term::TermPane;
use agora_vt::Terminal;

type This = WeakEntity<Root>;

async fn sleep(cx: &mut AsyncWindowContext, ms: u64) {
    cx.background_executor()
        .timer(Duration::from_millis(ms))
        .await;
}

fn hex(b: &[u8]) -> String {
    b.iter()
        .map(|x| format!("{x:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

struct T<'a> {
    this: &'a This,
    pane: Entity<TermPane>,
    id: usize,
    results: Vec<Value>,
}

impl T<'_> {
    fn inject(&self, cx: &mut AsyncWindowContext, bytes: &[u8]) {
        let b = bytes.to_vec();
        self.pane.read_with(cx, |p, _| {
            let mut st = p.shared.lock();
            st.scanner.feed(&b);
            st.term.feed(&b);
            st.generation += 1;
        });
        self.pane.update(cx, |_, cx| cx.notify());
    }

    fn take(&self, cx: &mut AsyncWindowContext) -> Vec<u8> {
        let m = self
            .this
            .read_with(cx, |r, _| r.metrics.clone())
            .unwrap_or_default();
        let mut g = m.borrow_mut();
        let id = self.id;
        let out: Vec<u8> = g
            .tapped
            .iter()
            .filter(|(p, _)| *p == id)
            .flat_map(|(_, b)| b.clone())
            .collect();
        g.tapped.clear();
        out
    }

    fn check(&mut self, name: &str, expected: &[u8], got: &[u8]) {
        self.results.push(json!({"check": name, "pass": expected == got, "expected": hex(expected), "got": hex(got)}));
    }

    fn check_bool(&mut self, name: &str, ok: bool, detail: Value) {
        self.results
            .push(json!({"check": name, "pass": ok, "detail": detail}));
    }

    fn at(&self, cx: &mut AsyncWindowContext, col: u16, row: u16) -> Point<Pixels> {
        self.pane.read_with(cx, |p, _| p.cell_center(col, row))
    }

    fn ev(&self, cx: &mut AsyncWindowContext, e: PlatformInput) {
        let _ = cx.update(|window, cx| window.dispatch_event(e, cx));
    }

    fn key(&self, cx: &mut AsyncWindowContext, ks: &'static str) {
        if let Ok(k) = Keystroke::parse(ks) {
            let _ = cx.update(|window, cx| window.dispatch_keystroke(k, cx));
        }
    }

    async fn click(
        &self,
        cx: &mut AsyncWindowContext,
        col: u16,
        row: u16,
        mods: Modifiers,
        count: usize,
    ) {
        let p = self.at(cx, col, row);
        self.ev(
            cx,
            PlatformInput::MouseDown(MouseDownEvent {
                button: MouseButton::Left,
                position: p,
                modifiers: mods,
                click_count: count,
                first_mouse: false,
            }),
        );
        self.ev(
            cx,
            PlatformInput::MouseUp(MouseUpEvent {
                button: MouseButton::Left,
                position: p,
                modifiers: mods,
                click_count: count,
            }),
        );
        sleep(cx, 30).await;
    }

    fn wheel(&self, cx: &mut AsyncWindowContext, col: u16, row: u16, lines: f32, mods: Modifiers) {
        let p = self.at(cx, col, row);
        self.ev(
            cx,
            PlatformInput::Scroll(ScrollEvent {
                position: p,
                delta: ScrollDelta::Lines(point(0.0, lines)),
                modifiers: mods,
                ..Default::default()
            }),
        );
    }
}

fn tp_take_all(this: &This, cx: &mut AsyncWindowContext) {
    if let Ok(m) = this.read_with(cx, |r, _| r.metrics.clone()) {
        m.borrow_mut().tapped.clear();
    }
}

pub async fn run(this: &This, cx: &mut AsyncWindowContext, s: &Stack) -> Value {
    let end = Instant::now() + Duration::from_secs(30);
    while this.read_with(cx, |r, _| r.agents.len()).unwrap_or(0) == 0 && Instant::now() < end {
        sleep(cx, 20).await;
    }
    let Some(agent) = s.echo.first() else {
        return json!({"error": "no echo agent"});
    };
    let id = this
        .update(cx, |r, cx| r.open(agent, true, cx))
        .unwrap_or(0);
    let Some(pane) = this
        .read_with(cx, |r, _| r.panes.get(id).cloned())
        .ok()
        .flatten()
    else {
        return json!({"error": "no pane"});
    };
    let m = this
        .read_with(cx, |r, _| r.metrics.clone())
        .unwrap_or_default();
    {
        let mut g = m.borrow_mut();
        g.tap_on = true;
        g.no_clipboard = true;
    }
    let end = Instant::now() + Duration::from_secs(15);
    while !pane.read_with(cx, |p, _| p.shared.lock().got_screen) && Instant::now() < end {
        sleep(cx, 20).await;
    }
    sleep(cx, 500).await;
    let fh = pane.read_with(cx, |p, _| p.focus.clone());
    let _ = cx.update(|window, cx| window.focus(&fh, cx));
    sleep(cx, 100).await;
    let mut t = T {
        this,
        pane: pane.clone(),
        id,
        results: Vec::new(),
    };
    let none = Modifiers::default();
    let shift = Modifiers {
        shift: true,
        ..Default::default()
    };
    let _ = t.take(cx);

    // Keys.
    for (ks, want) in [
        ("a", &b"a"[..]),
        ("enter", b"\r"),
        ("ctrl-c", b"\x03"),
        ("up", b"\x1b[A"),
        ("alt-x", b"\x1bx"),
        ("shift-tab", b"\x1b[Z"),
        ("f5", b"\x1b[15~"),
        ("ctrl-left", b"\x1b[1;5D"),
        ("backspace", b"\x7f"),
        ("escape", b"\x1b"),
    ] {
        t.key(cx, ks);
        sleep(cx, 10).await;
        let got = t.take(cx);
        t.check(&format!("key {ks}"), want, &got);
    }
    t.inject(cx, b"\x1b[?1h");
    sleep(cx, 30).await;
    t.key(cx, "up");
    sleep(cx, 10).await;
    let got = t.take(cx);
    t.check("key up with DECCKM (application cursor)", b"\x1bOA", &got);
    t.inject(cx, b"\x1b[?1l");
    // Non-ASCII text through the input handler (a Turkish-layout character).
    t.key(cx, "ş");
    sleep(cx, 10).await;
    let got = t.take(cx);
    t.check("text ş through the input handler", "ş".as_bytes(), &got);

    // IME: a preedit is drawn and sends nothing; the commit is sent once.
    let p2 = pane.clone();
    let _ = cx.update(|window, cx| {
        p2.update(cx, |p, cx| {
            EntityInputHandler::replace_and_mark_text_in_range(p, None, "にほ", None, window, cx)
        })
    });
    sleep(cx, 300).await;
    let during = t.take(cx);
    let painted = m.borrow().preedit_painted;
    let p2 = pane.clone();
    let _ = cx.update(|window, cx| {
        p2.update(cx, |p, cx| {
            EntityInputHandler::replace_text_in_range(p, None, "日本", window, cx)
        })
    });
    sleep(cx, 30).await;
    let commit = t.take(cx);
    t.check_bool(
        "IME preedit painted at the cursor, nothing sent",
        during.is_empty() && painted,
        json!({"sent_during_preedit": hex(&during), "preedit_painted": painted}),
    );
    t.check("IME commit sent once", "日本".as_bytes(), &commit);

    // Mouse: SGR (1006) with 1000.
    t.inject(cx, b"\x1b[?1000h\x1b[?1006h");
    sleep(cx, 30).await;
    t.click(cx, 4, 2, none, 1).await;
    let got = t.take(cx);
    t.check(
        "mouse 1000+1006 (SGR) click",
        b"\x1b[<0;5;3M\x1b[<0;5;3m",
        &got,
    );
    // Default (X10-style) encoding with 1000: release is button 3.
    t.inject(cx, b"\x1b[?1006l");
    sleep(cx, 30).await;
    t.click(cx, 4, 2, none, 1).await;
    let got = t.take(cx);
    t.check(
        "mouse 1000 default encoding click",
        &[0x1b, b'[', b'M', 32, 37, 35, 0x1b, b'[', b'M', 35, 37, 35],
        &got,
    );
    // urxvt (1015).
    t.inject(cx, b"\x1b[?1015h");
    sleep(cx, 30).await;
    t.click(cx, 4, 2, none, 1).await;
    let got = t.take(cx);
    t.check(
        "mouse 1000+1015 (urxvt) click",
        b"\x1b[32;5;3M\x1b[35;5;3M",
        &got,
    );
    // UTF-8 (1005) at a column past 95, where the coordinate needs two bytes.
    t.inject(cx, b"\x1b[?1015l\x1b[?1005h");
    sleep(cx, 30).await;
    let cols = pane.read_with(cx, |p, _| p.shared.lock().term.size().cols);
    let col = cols.saturating_sub(1).min(120);
    t.click(cx, col, 2, none, 1).await;
    let got = t.take(cx);
    let mut want = Vec::new();
    for code in [32u32, 35] {
        want.extend_from_slice(b"\x1b[M");
        for v in [code, u32::from(col) + 1 + 32, 3 + 32] {
            let mut buf = [0u8; 4];
            want.extend_from_slice(
                char::from_u32(v)
                    .unwrap_or(' ')
                    .encode_utf8(&mut buf)
                    .as_bytes(),
            );
        }
    }
    t.check(
        &format!("mouse 1000+1005 (UTF-8) click at column {}", col + 1),
        &want,
        &got,
    );
    // X10 (9): presses only, no release.
    t.inject(cx, b"\x1b[?1005l\x1b[?1000l\x1b[?9h");
    sleep(cx, 30).await;
    t.click(cx, 4, 2, none, 1).await;
    let got = t.take(cx);
    t.check(
        "mouse 9 (X10) press only",
        &[0x1b, b'[', b'M', 32, 37, 35],
        &got,
    );
    t.inject(cx, b"\x1b[?9l");
    // 1000 sends no drags; 1002 does.
    t.inject(cx, b"\x1b[?1000h\x1b[?1006h");
    sleep(cx, 30).await;
    let (a, b) = (t.at(cx, 4, 2), t.at(cx, 7, 2));
    t.ev(
        cx,
        PlatformInput::MouseDown(MouseDownEvent {
            button: MouseButton::Left,
            position: a,
            modifiers: none,
            click_count: 1,
            first_mouse: false,
        }),
    );
    t.ev(
        cx,
        PlatformInput::MouseMove(MouseMoveEvent {
            position: b,
            pressed_button: Some(MouseButton::Left),
            modifiers: none,
        }),
    );
    t.ev(
        cx,
        PlatformInput::MouseUp(MouseUpEvent {
            button: MouseButton::Left,
            position: b,
            modifiers: none,
            click_count: 1,
        }),
    );
    sleep(cx, 30).await;
    let got = t.take(cx);
    t.check(
        "mouse 1000: drag not reported",
        b"\x1b[<0;5;3M\x1b[<0;8;3m",
        &got,
    );
    t.inject(cx, b"\x1b[?1002h");
    sleep(cx, 30).await;
    t.ev(
        cx,
        PlatformInput::MouseDown(MouseDownEvent {
            button: MouseButton::Left,
            position: a,
            modifiers: none,
            click_count: 1,
            first_mouse: false,
        }),
    );
    t.ev(
        cx,
        PlatformInput::MouseMove(MouseMoveEvent {
            position: b,
            pressed_button: Some(MouseButton::Left),
            modifiers: none,
        }),
    );
    t.ev(
        cx,
        PlatformInput::MouseUp(MouseUpEvent {
            button: MouseButton::Left,
            position: b,
            modifiers: none,
            click_count: 1,
        }),
    );
    sleep(cx, 30).await;
    let got = t.take(cx);
    t.check(
        "mouse 1002: drag reported",
        b"\x1b[<0;5;3M\x1b[<32;8;3M\x1b[<0;8;3m",
        &got,
    );
    // Motion with no button: only under 1003.
    t.ev(
        cx,
        PlatformInput::MouseMove(MouseMoveEvent {
            position: a,
            pressed_button: None,
            modifiers: none,
        }),
    );
    sleep(cx, 20).await;
    let got = t.take(cx);
    t.check("mouse 1002: plain motion not reported", b"", &got);
    t.inject(cx, b"\x1b[?1003h");
    sleep(cx, 30).await;
    t.ev(
        cx,
        PlatformInput::MouseMove(MouseMoveEvent {
            position: b,
            pressed_button: None,
            modifiers: none,
        }),
    );
    sleep(cx, 20).await;
    let got = t.take(cx);
    t.check("mouse 1003: plain motion reported", b"\x1b[<35;8;3M", &got);
    t.inject(cx, b"\x1b[?1003l\x1b[?1002l\x1b[?1000h\x1b[?1006h");
    // Shift keeps the mouse local: nothing sent, a selection is made.
    t.click(cx, 4, 2, shift, 1).await;
    let got = t.take(cx);
    t.check("mouse mode on, Shift+click: nothing sent", b"", &got);
    // Wheel with the mouse on: one report per notch.
    t.wheel(cx, 4, 2, 1.0, none);
    t.wheel(cx, 4, 2, -1.0, none);
    sleep(cx, 20).await;
    let got = t.take(cx);
    t.check(
        "wheel with mouse mode: SGR 64/65 once per notch",
        b"\x1b[<64;5;3M\x1b[<65;5;3M",
        &got,
    );
    // Alternate screen, no mouse mode: three arrows per notch (DECCKM form honoured).
    t.inject(cx, b"\x1b[?1000l\x1b[?1006l\x1b[?1049h");
    sleep(cx, 30).await;
    t.wheel(cx, 4, 2, 1.0, none);
    sleep(cx, 20).await;
    let got = t.take(cx);
    t.check(
        "wheel on alternate screen without mouse mode: 3 arrows",
        b"\x1b[A\x1b[A\x1b[A",
        &got,
    );
    t.inject(cx, b"\x1b[?1h");
    t.wheel(cx, 4, 2, -1.0, none);
    sleep(cx, 20).await;
    let got = t.take(cx);
    t.check(
        "wheel on alternate screen with DECCKM: 3 application arrows",
        b"\x1bOB\x1bOB\x1bOB",
        &got,
    );
    t.inject(cx, b"\x1b[?1l\x1b[?1049l");
    // Main screen: the wheel scrolls history and sends nothing.
    let mut lines = Vec::new();
    for i in 0..200 {
        lines.extend_from_slice(format!("history line {i}\r\n").as_bytes());
    }
    t.inject(cx, &lines);
    sleep(cx, 50).await;
    t.wheel(cx, 4, 2, 2.0, none);
    sleep(cx, 50).await;
    let got = t.take(cx);
    let top = pane.read_with(cx, |p, _| p.visible_line(0));
    t.check_bool(
        "wheel on main screen scrolls history (6 lines for 2 notches), nothing sent",
        got.is_empty() && top.starts_with("history line"),
        json!({"sent": hex(&got), "top_line_after": top}),
    );
    // Typing returns to the live screen.
    t.key(cx, "z");
    sleep(cx, 30).await;
    let _ = t.take(cx);
    let scrolled = pane.read_with(cx, |p, _| p.scrolled());
    t.check_bool(
        "typing returns to the live screen",
        scrolled == 0,
        json!({"scroll": scrolled}),
    );

    // Selection and copy (system clipboard not touched in the self-test).
    t.inject(cx, b"\x1b[2J\x1b[Hhello selection test\r\nsecond line\r\n");
    sleep(cx, 50).await;
    let (a, b) = (t.at(cx, 0, 0), t.at(cx, 14, 0));
    t.ev(
        cx,
        PlatformInput::MouseDown(MouseDownEvent {
            button: MouseButton::Left,
            position: a,
            modifiers: none,
            click_count: 1,
            first_mouse: false,
        }),
    );
    t.ev(
        cx,
        PlatformInput::MouseMove(MouseMoveEvent {
            position: b,
            pressed_button: Some(MouseButton::Left),
            modifiers: none,
        }),
    );
    t.ev(
        cx,
        PlatformInput::MouseUp(MouseUpEvent {
            button: MouseButton::Left,
            position: b,
            modifiers: none,
            click_count: 1,
        }),
    );
    sleep(cx, 30).await;
    t.key(cx, "ctrl-shift-c");
    sleep(cx, 20).await;
    let copied = m.borrow().copied.clone().unwrap_or_default();
    t.check_bool(
        "drag selection + Ctrl+Shift+C copies",
        copied == "hello selection",
        json!({"copied": copied}),
    );
    t.click(cx, 8, 0, none, 2).await;
    let word = pane
        .read_with(cx, |p, _| p.selection_text())
        .unwrap_or_default();
    t.check_bool(
        "double click selects a word",
        word == "selection",
        json!({"selected": word}),
    );
    t.click(cx, 3, 1, none, 3).await;
    let line = pane
        .read_with(cx, |p, _| p.selection_text())
        .unwrap_or_default();
    t.check_bool(
        "triple click selects the line",
        line == "second line",
        json!({"selected": line}),
    );
    let sent = t.take(cx);
    t.check("selection gestures send nothing", b"", &sent);
    // Bracketed paste.
    t.inject(cx, b"\x1b[?2004h");
    pane.update(cx, |p, _| p.paste("x\x1by"));
    let got = t.take(cx);
    t.check(
        "bracketed paste, control characters removed",
        b"\x1b[200~xy\x1b[201~",
        &got,
    );
    t.inject(cx, b"\x1b[?2004l");

    // End to end on Unix: a real program behind the node's PTY turns SGR mouse on itself and prints the bytes it gets.
    if let Some(probe) = s.probe.first() {
        let pid = this
            .update(cx, |r, cx| r.open(probe, true, cx))
            .unwrap_or(0);
        if let Some(pp) = this
            .read_with(cx, |r, _| r.panes.get(pid).cloned())
            .ok()
            .flatten()
        {
            let end = Instant::now() + Duration::from_secs(15);
            while !pp.read_with(cx, |p, _| p.shared.lock().got_screen) && Instant::now() < end {
                sleep(cx, 20).await;
            }
            // The program turns the mouse on only now: a mode switched on before this window watched is not in the node's
            // repaint (docs/ux/terminal-fidelity.md), so it would not be seen.
            let b = b"go\r".to_vec();
            pp.update(cx, |p, _| p.send(b));
            sleep(cx, 1500).await;
            tp_take_all(this, cx);
            let modes = pp.read_with(cx, |p, _| p.shared.lock().scanner.modes());
            let tp = T {
                this,
                pane: pp.clone(),
                id: pid,
                results: Vec::new(),
            };
            tp.click(cx, 2, 1, none, 1).await;
            sleep(cx, 1500).await;
            let screen: Vec<String> =
                pp.read_with(cx, |p, _| p.shared.lock().term.bottom_non_empty_lines(40));
            let sent = tp.take(cx);
            let got: Vec<String> = screen
                .iter()
                .map(|l| l.trim().to_string())
                .filter(|l| l.len() == 2 && u8::from_str_radix(l, 16).is_ok())
                .collect();
            let want: Vec<String> = sent.iter().map(|b| format!("{b:02x}")).collect();
            let ok = !want.is_empty() && got.ends_with(&want);
            t.check_bool(
                "end to end: a real program behind the node PTY (helper probe, raw mode) turned SGR mouse on and received the click",
                ok,
                json!({"modes_seen_from_program": format!("{modes:?}"), "sent": hex(&sent), "program_printed": got.join(" ")}),
            );
        }
    } else {
        t.check_bool(
            "end to end with a real program",
            true,
            json!({"skipped": "no probe agent on this system (Windows)"}),
        );
    }

    let active = cx
        .update(|window, _| window.is_window_active())
        .unwrap_or(false);
    let paints = m.borrow().paints_total;
    let passed = t
        .results
        .iter()
        .filter(|r| r["pass"] == json!(true))
        .count();
    json!({"checks": t.results.len(), "passed": passed, "results": t.results, "wheel_line_deltas_seen": m.borrow().wheel_lines.clone(), "window_active": active, "pane_paints_total": paints})
}
