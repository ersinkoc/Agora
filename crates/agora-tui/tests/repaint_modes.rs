//! ADR-0100 decision 4: the terminal client's emulator and its mode scanner, fed the node's repaint, agree with the node's modes, on a
//! fresh view (late attach: D1, D2, D5) and on one that kept modes from before (a resubscribe after the program changed them: D3).

use agora_node::terminal::repaint_of;
use agora_tui::termio::{
    encode_mouse, focus_bytes, wheel_arrows, Encoding, ModeScanner, Phase, Report, Track,
};
use agora_vt::{AlacrittyTerminal, Modes, MouseEncoding, MouseTracking, Size, Terminal};

const SIZE: Size = Size { cols: 30, rows: 6 };

/// What the client holds for a pane: its emulator and its scanner, both fed the same bytes.
struct View {
    vt: AlacrittyTerminal,
    scanner: ModeScanner,
}

impl View {
    fn new() -> Self {
        Self {
            vt: AlacrittyTerminal::new(SIZE),
            scanner: ModeScanner::default(),
        }
    }
    fn feed(&mut self, bytes: &[u8]) {
        self.scanner.feed(bytes);
        self.vt.feed(bytes);
    }
}

/// The node's terminal after the program wrote `bytes`, and its repaint.
fn node(bytes: &[u8]) -> (AlacrittyTerminal, Vec<u8>) {
    let mut t = AlacrittyTerminal::new(SIZE);
    t.feed(bytes);
    let r = repaint_of(&mut t);
    (t, r)
}

/// The scanner reads the same mouse and focus modes the node's emulator holds.
fn agrees(view: &View, m: &Modes) {
    let p = view.scanner.modes();
    let track = match m.mouse_tracking {
        MouseTracking::Off => Track::Off,
        MouseTracking::Click => Track::Normal,
        MouseTracking::Drag => Track::Button,
        MouseTracking::Motion => Track::Any,
    };
    let encoding = match m.mouse_encoding {
        MouseEncoding::Default => Encoding::Default,
        MouseEncoding::Utf8 => Encoding::Utf8,
        MouseEncoding::Sgr => Encoding::Sgr,
    };
    assert_eq!(
        (p.track, p.encoding, p.focus),
        (track, encoding, m.focus_reporting)
    );
    assert_eq!(view.vt.modes(), *m);
}

#[test]
fn a_late_view_knows_the_mouse_focus_and_alternate_screen_from_the_repaint() {
    let (src, repaint) = node(b"\x1b[?1049h\x1b[HFULL\x1b[?1000h\x1b[?1006h\x1b[?1004h\x1b[?1h");
    let mut view = View::new();
    view.feed(&repaint);
    agrees(&view, &src.modes());
    // D1: a click reaches the program in its encoding.
    let click = Report {
        phase: Phase::Press,
        button: 0,
        mods: 0,
        col: 4,
        row: 2,
    };
    assert_eq!(
        encode_mouse(&view.scanner.modes(), &click).as_deref(),
        Some(&b"\x1b[<0;5;3M"[..])
    );
    // D2: the view is on the program's alternate screen, so the wheel (with no mouse mode) becomes arrow keys there.
    assert!(view.vt.modes().alt_screen);
    assert_eq!(view.vt.line(0), "FULL");
    assert_eq!(wheel_arrows(true, view.vt.modes()), b"\x1bOA\x1bOA\x1bOA");
    // D5: focus reports are sent.
    assert_eq!(
        focus_bytes(&view.scanner.modes(), true),
        Some(&b"\x1b[I"[..])
    );
}

#[test]
fn a_view_that_kept_old_modes_gets_the_programs_current_ones() {
    let mut view = View::new();
    // The view watched while the program had the mouse, focus and the alternate screen on.
    view.feed(b"\x1b[?1049h\x1b[?1002h\x1b[?1006h\x1b[?1004h");
    // Then it lost the stream; meanwhile the program switched all of that off. The resubscribe's repaint says so (D3).
    let (src, repaint) = node(b"back on the main screen");
    view.feed(&repaint);
    agrees(&view, &src.modes());
    assert_eq!(view.scanner.modes().track, Track::Off);
    assert!(!view.vt.modes().alt_screen);
    assert_eq!(view.vt.line(0), "back on the main screen");
    assert_eq!(focus_bytes(&view.scanner.modes(), true), None);
}

#[test]
fn modes_the_node_does_not_track_are_left_alone() {
    // Mouse 9 and the urxvt encoding are known to the scanner only from the live stream; a repaint must not switch them off here.
    // (This holds for the terminal client: xterm.js, in the web window, ends 9 on any of the 1000/1002/1003 resets.)
    let mut view = View::new();
    view.feed(b"\x1b[?9h\x1b[?1015h");
    let (_, repaint) = node(b"");
    view.feed(&repaint);
    assert_eq!(view.scanner.modes().track, Track::X10);
    assert_eq!(view.scanner.modes().encoding, Encoding::Urxvt);
}

#[test]
fn every_carried_mode_agrees_for_each_mouse_mode_and_encoding() {
    for track in ["", "\x1b[?1000h", "\x1b[?1002h", "\x1b[?1003h"] {
        for enc in ["", "\x1b[?1005h", "\x1b[?1006h"] {
            for focus in ["", "\x1b[?1004h"] {
                let (src, repaint) = node(format!("{track}{enc}{focus}").as_bytes());
                for stale in [&b""[..], b"\x1b[?1003h\x1b[?1006h\x1b[?1004h"] {
                    let mut view = View::new();
                    view.feed(stale);
                    view.feed(&repaint);
                    agrees(&view, &src.modes());
                }
            }
        }
    }
}
