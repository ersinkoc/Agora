#![allow(clippy::unwrap_used, clippy::expect_used)]

use agora_adapters::{Detector, Manifest, ManifestError, Pacer};
use agora_core::AgentState;
use agora_vt::{AlacrittyTerminal, Size, Terminal};

const MANIFEST: &str = include_str!("fixtures/example-agent.toml");

/// A terminal as it looks mid-session: earlier output has scrolled, so the agent's current screen sits at
/// the bottom of the grid (`bottom_lines(n)` is the literal bottom of the screen, not the last text).
fn term_with(bytes: &[u8]) -> AlacrittyTerminal {
    let mut t = AlacrittyTerminal::new(Size { cols: 80, rows: 24 });
    t.feed(&b"\r\n".repeat(30));
    t.feed(bytes);
    t
}

fn detector() -> Detector {
    Detector::new(&Manifest::parse(MANIFEST).expect("valid manifest"))
}

fn detect_screen(name: &str) -> Option<(AgentState, String)> {
    let path = format!(
        "{}/tests/fixtures/screens/{name}.screen",
        env!("CARGO_MANIFEST_DIR")
    );
    let bytes = std::fs::read(&path).expect("fixture");
    detector()
        .detect(&term_with(&bytes))
        .map(|d| (d.state, d.rule_id))
}

#[test]
fn fixture_screens_produce_the_expected_states() {
    let cases = [
        ("blocked", Some((AgentState::Blocked, "permission_prompt"))),
        ("working-title", Some((AgentState::Working, "spinner"))),
        (
            "working-hint",
            Some((AgentState::Working, "interrupt_hint")),
        ),
        ("done", Some((AgentState::Done, "finished"))),
        ("idle", Some((AgentState::Idle, "prompt"))),
        ("unknown", None),
    ];
    for (name, want) in cases {
        let want = want.map(|(s, id)| (s, id.to_string()));
        assert_eq!(detect_screen(name), want, "screen {name}");
    }
}

#[test]
fn higher_priority_wins_when_several_rules_match() {
    // Both the permission prompt (900) and the interrupt hint (700) are on screen.
    let t = term_with(b"* Working (esc to interrupt)\r\nAllow this? [y/n] ");
    let d = detector().detect(&t).unwrap();
    assert_eq!(
        (d.state, d.rule_id.as_str()),
        (AgentState::Blocked, "permission_prompt")
    );
}

#[test]
fn not_conditions_exclude_a_rule() {
    // "esc to interrupt" with a [y/n] prompt must not report the lower-priority working rule alone.
    let manifest = Manifest::parse(&MANIFEST.replace("priority = 900", "priority = 50")).unwrap();
    let t = term_with(b"esc to interrupt\r\nAllow? [y/n] ");
    let d = Detector::new(&manifest).detect(&t).unwrap();
    assert_ne!(d.rule_id, "interrupt_hint");
}

#[test]
fn scrolled_back_output_is_never_read() {
    // The permission prompt scrolls off the 24-row screen into scrollback; it must not be detected.
    let mut t = AlacrittyTerminal::new(Size { cols: 80, rows: 24 });
    t.feed(b"Allow this command? [y/n]\r\n");
    for i in 0..100 {
        t.feed(format!("output line {i}\r\n").as_bytes());
    }
    assert_eq!(detector().detect(&t), None);
}

#[test]
fn detection_does_not_depend_on_anyone_watching() {
    // The detector takes only the terminal; there is no visibility input. A hidden terminal is still
    // detected, and the pacer merely checks it less often.
    let t = term_with(
        &std::fs::read(format!(
            "{}/tests/fixtures/screens/blocked.screen",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap(),
    );
    assert_eq!(detector().detect(&t).unwrap().state, AgentState::Blocked);
    let p = Pacer::default();
    assert!(p.interval(false) > p.interval(true));
}

fn bad(replacement: (&str, &str)) -> ManifestError {
    Manifest::parse(&MANIFEST.replacen(replacement.0, replacement.1, 1)).unwrap_err()
}

fn field(e: &ManifestError) -> &str {
    match e {
        ManifestError::Invalid { field } => field,
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[test]
fn manifest_validation_rejects_bad_input() {
    assert!(matches!(
        bad(("min_engine = 1", "min_engine = 99")),
        ManifestError::EngineTooOld { required: 99, .. }
    ));
    assert_eq!(
        field(&bad((
            "kind = \"example-agent\"",
            "kind = \"Example Agent\""
        ))),
        "kind"
    );
    assert_eq!(
        field(&bad(("bottom_lines(8)", "bottom_lines(0)"))),
        "rules[0].region"
    );
    assert_eq!(
        field(&bad(("bottom_lines(8)", "viewport"))),
        "rules[0].region"
    );
    assert_eq!(
        field(&bad(("id = \"spinner\"", "id = \"permission_prompt\""))),
        "rules[1].id"
    );
    assert_eq!(
        field(&bad(("regex = '^[⠋", "regex = '(["))),
        "rules[1].regex"
    );
    assert!(matches!(
        bad(("state = \"blocked\"", "state = \"exploding\"")),
        ManifestError::Parse(_)
    ));
    assert!(matches!(
        bad(("priority = 900", "prioritee = 900")),
        ManifestError::Parse(_)
    ));
}

#[test]
fn a_rule_with_only_exclusions_is_rejected() {
    let src = r#"
kind = "k"
display_name = "K"
version = "1"
min_engine = 1
[[rules]]
id = "r"
state = "idle"
priority = 1
region = "title"
not = [{ contains = "x" }]
"#;
    assert_eq!(
        field(&Manifest::parse(src).unwrap_err()),
        "rules[0].conditions"
    );
}

#[test]
fn unknown_sections_are_ignored_so_newer_manifests_load() {
    let src =
        format!("{MANIFEST}\n[usage]\nsource = \"otel\"\n[accounts]\nconfig_dir_env = \"X\"\n");
    assert!(Manifest::parse(&src).is_ok());
}

#[test]
fn pathological_regexes_cannot_hang_the_node() {
    // Linear-time engine: this classic backtracking bomb finishes instantly.
    let src = MANIFEST.replace("contains = \"esc to interrupt\"", "regex = '(a+)+$'");
    let m = Manifest::parse(&src).unwrap();
    let t = term_with(format!("{}\r\n", "a".repeat(5000)).as_bytes());
    let start = std::time::Instant::now();
    let _ = Detector::new(&m).detect(&t);
    assert!(start.elapsed() < std::time::Duration::from_secs(2));
}
