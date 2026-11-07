#![allow(clippy::expect_used)] // expect is allowed in tests

use std::process::Command;

fn run(script: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_agora-fake-agent"))
        .args(["--script", script])
        .output()
        .expect("spawn fake agent")
}

#[test]
fn reproduces_working_blocked_idle() {
    let dir = env!("CARGO_MANIFEST_DIR");
    let out = run(&format!("{dir}/fixtures/working-blocked-idle.toml"));
    assert!(out.status.success());
    let s = String::from_utf8_lossy(&out.stdout);
    let working = s.find("Thinking...").expect("working frame");
    let blocked = s.find("Allow Bash").expect("blocked frame");
    let idle = s.find("Done.").expect("idle frame");
    assert!(working < blocked && blocked < idle);
}

#[test]
fn missing_script_fails() {
    let out = run("/nonexistent.toml");
    assert_eq!(out.status.code(), Some(1));
}
