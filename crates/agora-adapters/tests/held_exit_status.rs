//! #902, #938 review: how a held adapter ended is asked under the node's supervisor lock, so the answer must come within the time the
//! caller gives, however slow the holder link is (a list with a 10 s reply timeout, a release, the link's lock). A synthetic
//! `HeldControl` stands in for the holder; no real agent.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use agora_adapters::host::{Adapter, HeldIo};
use agora_platform::{ExitStatus, HeldControl, HeldSignal};

/// A holder that answers after `delay`, with exit code 3 once `running` answers have been given.
struct Holder {
    delay: Duration,
    running: usize,
    asked: AtomicUsize,
}

impl HeldControl for Holder {
    fn signal(&self, _: HeldSignal) -> std::io::Result<()> {
        Ok(())
    }
    fn exit_status(&self) -> std::io::Result<Option<ExitStatus>> {
        std::thread::sleep(self.delay);
        let n = self.asked.fetch_add(1, Ordering::SeqCst);
        Ok((n >= self.running).then_some(ExitStatus {
            code: Some(3),
            signal: None,
        }))
    }
}

fn held(delay: Duration, running: usize) -> Adapter {
    Adapter::attach_held(
        HeldIo {
            pid: 1,
            input: Box::new(std::io::sink()),
            output: Box::new(std::io::empty()),
            control: Arc::new(Holder {
                delay,
                running,
                asked: AtomicUsize::new(0),
            }),
            stop: Arc::new(AtomicBool::new(false)),
            dropped: Arc::new(AtomicU64::new(0)),
            ready: Arc::new(AtomicBool::new(true)),
        },
        vec![],
    )
    .unwrap()
}

#[test]
fn a_slow_holder_does_not_hold_the_caller_past_its_budget() {
    for budget in [Duration::from_millis(10), Duration::from_millis(50)] {
        let mut adapter = held(Duration::from_millis(400), 0);
        let began = Instant::now();
        let status = adapter.exit_status(budget);
        let took = began.elapsed();
        assert_eq!(status, None, "the answer came after the budget: unknown");
        // The budget plus scheduling slack on a loaded machine, far below the holder's 400 ms.
        assert!(
            took < budget + Duration::from_millis(100),
            "{took:?} for {budget:?}"
        );
    }
}

#[test]
fn a_holder_that_answers_in_time_gives_the_status() {
    let mut adapter = held(Duration::from_millis(1), 0);
    assert_eq!(
        adapter.exit_status(Duration::from_millis(500)),
        Some(ExitStatus {
            code: Some(3),
            signal: None
        })
    );
    // Still running at first: asked again within the budget until it has ended.
    let mut adapter = held(Duration::from_millis(1), 3);
    assert_eq!(
        adapter
            .exit_status(Duration::from_millis(500))
            .and_then(|s| s.code),
        Some(3)
    );
}
