//! The mode scanner and the terminal emulator the screen is drawn by agree on long mode lists (F1140, Codex review of #790): a list of
//! 32 numbers (the most the emulator's parser keeps) is applied by both, one of 33 by neither. Bracketed paste (2004) is the
//! emulator's flag; mouse tracking (1000) the scanner's; both are switched in the same sequence, so they must move together.

use agora_tui::termio::{ModeScanner, Track};
use agora_vt::{AlacrittyTerminal, Size, Terminal};

const ON: &[u8] = b"\x1b[?1000h\x1b[?2004h";

/// `n` numbers, 1000 and 2004 last.
fn list(n: usize, fin: char) -> Vec<u8> {
    let mut v: Vec<String> = (0..n - 2).map(|i| (100 + i).to_string()).collect();
    v.extend(["1000", "2004"].map(String::from));
    format!("\x1b[?{}{fin}", v.join(";")).into_bytes()
}

/// Feeds `before` then `seq` to both: (emulator bracketed paste on, scanner tracking).
fn both(before: &[u8], seq: &[u8]) -> (bool, Track) {
    let mut vt = AlacrittyTerminal::new(Size { cols: 20, rows: 4 });
    let mut s = ModeScanner::default();
    for part in [before, seq] {
        vt.feed(part);
        s.feed(part);
    }
    (vt.modes().bracketed_paste, s.modes().track)
}

#[test]
fn a_32_number_list_clears_a_tracked_mode_as_the_emulator_does() {
    assert_eq!(both(ON, b""), (true, Track::Normal));
    assert_eq!(both(ON, &list(32, 'l')), (false, Track::Off));
    assert_eq!(both(b"", &list(32, 'h')), (true, Track::Normal));
}

#[test]
fn a_33_number_list_changes_nothing_as_in_the_emulator() {
    assert_eq!(both(ON, &list(33, 'l')), (true, Track::Normal));
    assert_eq!(both(b"", &list(33, 'h')), (false, Track::Off));
}
