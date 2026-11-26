//! `agora manual` as a person or a program runs it (G10): the list, the JSON list, a topic, an unknown topic, a stray word, the
//! Turkish list and a reader that goes away. What the topics say is tested in `src/manualcmd.rs`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::{Command, Output, Stdio};

const TOPICS: [&str; 7] = [
    "scripting",
    "themes",
    "layouts",
    "keys",
    "plugins",
    "mesh",
    "files",
];

fn agora(lang: &str) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_agora"));
    // The manual reads nothing of the machine; a folder of its own keeps it that way if that ever changes.
    c.env(
        "AGORA_HOME",
        std::env::temp_dir().join("agora-manual-test-home"),
    )
    .env("AGORA_LANG", lang)
    .stdin(Stdio::null());
    c
}

fn run(args: &[&str]) -> Output {
    agora("en").args(args).output().unwrap()
}

fn text(b: &[u8]) -> String {
    String::from_utf8(b.to_vec()).unwrap()
}

#[test]
fn the_list_names_every_topic_with_its_summary() {
    let out = run(&["manual"]);
    assert!(out.status.success());
    let s = text(&out.stdout);
    assert!(s.starts_with("The manual has these topics."), "{s}");
    for t in TOPICS {
        assert!(
            s.lines()
                .any(|l| l.trim_start().starts_with(&format!("{t} "))),
            "{t}: {s}"
        );
    }
    assert!(s.contains("Write a theme file"), "{s}");
    assert!(!s.contains('\u{1b}'));
    assert!(out.stderr.is_empty());
}

#[test]
fn the_list_follows_the_persons_language() {
    let out = agora("tr").arg("manual").output().unwrap();
    assert!(out.status.success());
    let s = text(&out.stdout);
    assert!(s.starts_with("Kılavuzun konuları bunlar."), "{s}");
    assert!(s.contains("bir tema dosyası yaz"), "{s}");
}

#[test]
fn json_lists_the_topics_for_programs() {
    let out = run(&["manual", "--json"]);
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let ids: Vec<&str> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, TOPICS);
    assert!(v[0]["summary"]
        .as_str()
        .unwrap()
        .starts_with("Start, prompt"));
}

#[test]
fn a_topic_prints_as_it_is_written() {
    let out = run(&["manual", "themes"]);
    assert!(out.status.success());
    let s = text(&out.stdout);
    let file = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/manual/themes.md"),
    )
    .unwrap()
    .replace("\r\n", "\n");
    assert_eq!(s, file);

    let out = run(&["manual", "mesh", "--json"]);
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["id"], "mesh");
    assert!(v["text"].as_str().unwrap().starts_with("# Mesh"));
}

#[test]
fn an_unknown_topic_or_a_stray_word_is_a_usage_error() {
    let out = run(&["manual", "colours"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    let e = text(&out.stderr);
    assert!(
        e.contains("\"colours\"") && e.contains("scripting, themes"),
        "{e}"
    );

    let out = run(&["manual", "themes", "extra"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
}

#[test]
fn a_reader_that_goes_away_ends_it_quietly() {
    // `agora manual mesh | head -0`: the reader is gone before (or while) it writes.
    let mut child = agora("en")
        .args(["manual", "mesh"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{:?}", out.status);
    assert!(out.stderr.is_empty(), "{}", text(&out.stderr));
}
