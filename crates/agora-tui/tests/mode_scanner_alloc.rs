//! The mode scanner sits on the client's output path (every byte of every pane): once built, reading output never allocates
//! (Codex review of #790). Its own binary, so the counting allocator sees only this test; counted per thread.
use agora_tui::termio::{ModeScanner, Track};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct Counting;

thread_local! {
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
}

// SAFETY: forwards every call to the system allocator unchanged; only counts allocations on the calling thread.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = ALLOCS.try_with(|n| n.set(n.get() + 1));
        // SAFETY: same contract as the caller's.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: same contract as the caller's.
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let _ = ALLOCS.try_with(|n| n.set(n.get() + 1));
        // SAFETY: same contract as the caller's.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn allocs() -> usize {
    ALLOCS.with(Cell::get)
}

#[test]
fn reading_output_never_allocates() {
    // Typical output: cursor moves, colours, a mode list it acts on, one it drops, and text.
    let mut chunk = Vec::new();
    for i in 0..1000 {
        chunk.extend_from_slice(
            format!(
                "\x1b[{};{}H\x1b[38;5;{}mtext\x1b[0m",
                i % 50,
                i % 80,
                i % 256
            )
            .as_bytes(),
        );
    }
    chunk.extend_from_slice(b"\x1b[?1000;1006;1004h");
    chunk.extend_from_slice(format!("\x1b[?{}l", "7;".repeat(40)).as_bytes());
    let mut s = ModeScanner::default();
    let before = allocs();
    for _ in 0..10 {
        s.feed(&chunk);
    }
    let after = allocs();
    assert_eq!(after - before, 0, "allocations while reading output");
    assert_eq!(s.modes().track, Track::Normal);
}
