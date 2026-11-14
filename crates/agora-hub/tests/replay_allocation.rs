//! #519 review (Codex): a resume over the byte budget must not read a page of large events into memory before it falls back to
//! a snapshot. The allocation meter and the fixture are the reviewer's proof (docs/reviews/0519-replay-allocation-proof.rs at 8b23337).
#![allow(clippy::unwrap_used, clippy::expect_used)]
use agora_hub::{
    hub::{Hub, MAX_REPLAY_BYTES},
    session::NodeSession,
    store::SqliteStore,
};
use agora_protocol::{
    handshake::Support,
    v1::{self as v1, event::Body},
};
use prost::Message;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

// Count live Rust allocations. The sole test resets the high-water mark immediately before subscribe;
// fixture construction, SQLite storage and ingestion are outside that measured interval.
struct Meter;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
fn added(n: usize) {
    let live = LIVE.fetch_add(n, Relaxed) + n;
    PEAK.fetch_max(live, Relaxed);
}
unsafe impl GlobalAlloc for Meter {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            added(l.size());
        }
        p
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(l) };
        if !p.is_null() {
            added(l.size());
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size(), Relaxed);
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, l, n) };
        if !q.is_null() {
            if n >= l.size() {
                added(n - l.size());
            } else {
                LIVE.fetch_sub(l.size() - n, Relaxed);
            }
        }
        q
    }
}
#[global_allocator]
static ALLOCATOR: Meter = Meter;

#[test]
fn the_byte_budget_is_enforced_before_a_large_page_is_read() {
    let hub = Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap();
    let support = Support {
        generations: vec![1],
        capabilities: vec![],
        required: vec![],
    };
    let mut session = NodeSession::new(hub.store(), "n", &support);
    session
        .hello(
            &v1::Hello {
                role: v1::Role::Node as i32,
                protocol_generations: vec![1],
                ..Default::default()
            },
            0,
        )
        .unwrap();
    session.resume(&v1::Resume::default()).unwrap();
    let paths: Vec<String> = (0..256)
        .map(|i| format!("src/{i:04}/{}.rs", "x".repeat(240)))
        .collect();
    let mut total = 0;
    for seq in 1..=1000 {
        let payload = v1::Event {
            producer_id: "n".into(),
            seq,
            at_ms: seq,
            body: Some(Body::FilesChanged(v1::FilesChanged {
                worktree_id: "w".into(),
                paths: paths.clone(),
            })),
        }
        .encode_to_vec();
        assert!(payload.len() < agora_protocol::frame::DEFAULT_MAX_PAYLOAD);
        total += payload.len();
        hub.ingest(&mut session, &payload).unwrap();
    }
    drop(paths);
    let start = Instant::now();
    let baseline = LIVE.load(Relaxed);
    PEAK.store(baseline, Relaxed);
    let subscription = hub.subscribe(Some(0)).unwrap();
    let extra = PEAK.load(Relaxed).saturating_sub(baseline);
    let elapsed = start.elapsed();
    assert!(subscription.backlog.is_empty());
    assert!(subscription.snapshot.is_some());
    eprintln!("stored_payload_bytes={total}; replay_budget={MAX_REPLAY_BYTES}; extra_peak_rust_heap_bytes={extra}; subscribe_elapsed={elapsed:?}");
    // Allow 2x the encoded budget for decoded objects and one record of overhead. A complete 1000-row page is much larger.
    assert!(
        extra <= 2 * MAX_REPLAY_BYTES,
        "8 MiB replay fallback allocated {extra} additional bytes before returning the snapshot"
    );
}
