//! #902, #938 review: how a held adapter ended is read under the node's supervisor lock, so it must never ask the holder (a list with
//! a 10 s reply timeout, a release, the link's lock, shared by every held agent): it is the status the output's reader already heard
//! from the holder with the output, or unknown. A synthetic `HeldControl` stands in for the holder and counts every question; no real
//! agent.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agora_adapters::host::{Adapter, HeldIo};
use agora_platform::{ExitStatus, HeldControl, HeldSignal};

/// A holder that would stall every status question for 400 ms; it counts them.
#[derive(Default)]
struct Holder {
    asked: AtomicUsize,
}

impl HeldControl for Holder {
    fn signal(&self, _: HeldSignal) -> std::io::Result<()> {
        Ok(())
    }
    fn exit_status(&self) -> std::io::Result<Option<ExitStatus>> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(400));
        Ok(Some(ExitStatus {
            code: Some(99),
            signal: None,
        }))
    }
}

fn held(holder: &Arc<Holder>, pid: u32, heard: Option<ExitStatus>) -> Adapter {
    Adapter::attach_held(
        HeldIo {
            pid,
            input: Box::new(std::io::sink()),
            output: Box::new(std::io::empty()),
            control: Arc::clone(holder) as Arc<dyn HeldControl>,
            stop: Arc::new(AtomicBool::new(false)),
            dropped: Arc::new(AtomicU64::new(0)),
            ready: Arc::new(AtomicBool::new(true)),
            exit: Arc::new(Mutex::new(heard)),
        },
        vec![],
    )
    .unwrap()
}

#[test]
fn held_adapters_that_ended_give_the_status_heard_with_their_output_without_asking_the_holder() {
    let holder = Arc::new(Holder::default());
    let code = |c| ExitStatus {
        code: Some(c),
        signal: None,
    };
    let killed = ExitStatus {
        code: None,
        signal: Some(9),
    };
    let heard = [
        Some(code(3)),
        Some(killed),
        None,
        Some(code(1)),
        None,
        Some(code(0)),
        Some(code(7)),
        None,
    ];
    let mut adapters: Vec<Adapter> = heard
        .iter()
        .enumerate()
        .map(|(i, h)| held(&holder, 10 + i as u32, *h))
        .collect();
    // As one supervisor tick would ask them all, one after the other, with its 50 ms budget each.
    let began = Instant::now();
    let got: Vec<Option<ExitStatus>> = adapters
        .iter_mut()
        .map(|a| a.exit_status(Duration::from_millis(50)))
        .collect();
    let took = began.elapsed();
    assert_eq!(got, heard, "the cached status, or unknown");
    assert_eq!(
        holder.asked.load(Ordering::SeqCst),
        0,
        "the holder was asked"
    );
    assert!(
        took < Duration::from_millis(50),
        "{took:?} for eight adapters"
    );
    // Dropping them ends them as before (that may ask the holder: off the supervisor's status read).
    drop(adapters);
}
