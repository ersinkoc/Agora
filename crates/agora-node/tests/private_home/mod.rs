//! Points this test process's home folder at a folder of its own under the target folder, once, before any confined agent starts:
//! the node keeps the agents' git settings files under `~/.agora/git-config` (ADR-0078), and a test must never write into the
//! developer's real node folder. Call [`init`] first in every test that starts a confined agent.
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::OnceLock;

pub fn init() -> &'static PathBuf {
    static HOME: OnceLock<PathBuf> = OnceLock::new();
    HOME.get_or_init(|| {
        let h = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("test-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&h);
        std::fs::create_dir_all(&h).expect("a home folder for the test");
        std::env::set_var("HOME", &h);
        std::env::set_var("USERPROFILE", &h);
        h
    })
}
