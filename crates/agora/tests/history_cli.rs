//! ADR-0089 decision 3: `agora history purge` removes every chat transcript the node keeps, and only those.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Command;

const AGORA: &str = env!("CARGO_BIN_EXE_agora");

#[test]
fn purge_removes_the_transcripts_and_nothing_else() {
    let home = std::env::temp_dir().join(format!("agora-history-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let data = home.join("data");
    std::fs::create_dir_all(data.join("transcripts")).unwrap();
    std::fs::write(data.join("transcripts/agent-1.jsonl"), "{}\n").unwrap();
    std::fs::write(data.join("policy"), "").unwrap();
    let run = || {
        Command::new(AGORA)
            .args(["history", "purge"])
            .env("AGORA_HOME", &home)
            .env("AGORA_LANG", "en")
            .output()
            .unwrap()
    };
    let out = run();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("Removed the chat history"));
    assert!(!data.join("transcripts").exists());
    assert!(data.join("policy").exists());
    // Nothing left to remove is not a failure.
    assert!(run().status.success());
    let _ = std::fs::remove_dir_all(&home);
}
