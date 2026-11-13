//! The manifests in `manifests/agents/` against the screens captured from the real agents in `fixtures/<kind>/`
//! (`docs/specs/agent-manifest.md`). Each `<name>.screen` is raw terminal output; `<name>.expect` is `<state> <rule id>` or `none`.
//! A screen captured on another size than 120x40 says so in its name: `<name>@<cols>x<rows>.screen` (and `<name>@<cols>x<rows>.expect`).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use agora_adapters::{Detector, Manifest};
use agora_vt::{AlacrittyTerminal, Size, Terminal};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../manifests/agents")
}

/// The terminal size a screen was captured on, from its file name (`trust@24x20.screen`); 120x40 when the name has none.
/// A size that does not parse fails the test rather than silently falling back.
fn screen_size(f: &std::path::Path) -> Size {
    let stem = f.file_stem().unwrap().to_string_lossy();
    let Some((_, size)) = stem.split_once('@') else {
        return Size {
            cols: 120,
            rows: 40,
        };
    };
    let parsed = size
        .split_once('x')
        .and_then(|(c, r)| Some((c.parse().ok()?, r.parse().ok()?)));
    let (cols, rows) =
        parsed.unwrap_or_else(|| panic!("{}: size must be <cols>x<rows>", f.display()));
    Size { cols, rows }
}

#[test]
fn every_shipped_manifest_loads_and_its_captured_screens_produce_the_expected_state() {
    let mut kinds = 0;
    for entry in std::fs::read_dir(root()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "toml") {
            continue;
        }
        let manifest = Manifest::parse(&std::fs::read_to_string(&path).unwrap())
            .unwrap_or_else(|e| panic!("{}: {e:?}", path.display()));
        let kind = path.file_stem().unwrap().to_string_lossy().into_owned();
        // A kind the node drives without a terminal (an ACP agent, L2) has no screens to check.
        if manifest.driver(std::path::Path::new("agora")).is_some() {
            kinds += 1;
            continue;
        }
        let detector = Detector::new(&manifest);
        let ready = Detector::ready(&manifest);
        let typed = Detector::typed(&manifest);
        assert!(
            !manifest.ready.is_empty(),
            "{kind} needs [[ready]] rules (ADR-0057)"
        );
        let dir = root().join("fixtures").join(&kind);
        let mut screens = 0;
        for f in std::fs::read_dir(&dir).unwrap() {
            let f = f.unwrap().path();
            if f.extension().is_none_or(|e| e != "screen") {
                continue;
            }
            // A screen is replayed on the terminal size it was captured on: `<name>@<cols>x<rows>.screen`, or 120x40 without one.
            let mut term = AlacrittyTerminal::new(screen_size(&f));
            term.feed(&std::fs::read(&f).unwrap());
            let got = detector.detect(&term).map_or_else(
                || "none".to_string(),
                |d| format!("{:?} {}", d.state, d.rule_id),
            );
            let want = std::fs::read_to_string(f.with_extension("expect"));
            let screen = term.bottom_non_empty_lines(12).join("\n");
            let want = want.unwrap_or_else(|_| {
                panic!(
                    "{} has no .expect; it shows:\n{screen}\nand detects: {got}",
                    f.display()
                )
            });
            assert_eq!(got, want.trim(), "{}\n{screen}", f.display());
            // ADR-0057: a helper's first prompt is typed only into a screen the `[[ready]]` rules match. They must match the captured
            // idle and answered screens (an empty input box) and nothing else: not a working screen, not a dialog, and not a menu whose
            // selected row an `idle` rule also matches (`menu.screen`, written to show that difference).
            let name = f.file_stem().unwrap().to_string_lossy().into_owned();
            let empty_box = name == "idle" || name == "answered";
            // `typed.screen` (constructed from the captured idle screen with text in the box, since a real capture of that moment is
            // not kept): `[[typed]]` matches it and nothing else, and `input_box` reads exactly the typed text from it.
            assert_eq!(
                typed.detect(&term).is_some(),
                name == "typed",
                "{}: typed rules\n{screen}",
                f.display()
            );
            if name == "typed" {
                let pattern = manifest.input_box.as_deref().expect("input_box");
                assert_eq!(
                    Detector::input_box(pattern, &term).as_deref(),
                    Some("write the tests"),
                    "{}\n{screen}",
                    f.display()
                );
            }
            assert_eq!(
                ready.detect(&term).is_some(),
                empty_box,
                "{}: ready rules\n{screen}",
                f.display()
            );
            screens += 1;
        }
        assert!(screens >= 3, "{kind} needs captured screens");
        kinds += 1;
    }
    assert!(kinds >= 1);
}

/// The usage files the manifests describe (ADR-0042): lines of the real shape (scrubbed) read through the manifest's own paths.
#[test]
fn the_usage_fixtures_add_up_to_what_the_files_say() {
    let mut checked = 0;
    for entry in std::fs::read_dir(root()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "toml") {
            continue;
        }
        let manifest = Manifest::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let Some(spec) = manifest.usage.and_then(|u| u.file) else {
            continue;
        };
        let dir = root().join("fixtures").join(&manifest.kind);
        let mut reader = agora_adapters::usage::FileReader::new(spec);
        let (mut input, mut output, mut model) = (0, 0, String::new());
        for line in std::fs::read_to_string(dir.join("usage.jsonl"))
            .unwrap()
            .lines()
        {
            if let Some(s) = reader.line(line) {
                input += s.input;
                output += s.output;
                model = s.model;
            }
        }
        let got = format!(
            "input {input} output {output} model {model} cwd {}",
            reader.cwd().unwrap_or("")
        );
        let want = std::fs::read_to_string(dir.join("usage.expect")).unwrap();
        assert_eq!(got, want.trim(), "{}", manifest.kind);
        checked += 1;
    }
    assert!(
        checked >= 3,
        "claude-code, codex and wrongstack read usage files"
    );
}

/// ADR-0059: every shipped `[[launch.option]]` label is a message the windows can show: it is in the English web and terminal-client
/// locale files (the clients that list the options), so no window shows a bare id.
#[test]
fn every_shipped_launch_option_label_is_in_the_english_locales() {
    let locales = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../locales/en");
    let ids = |file: &str| -> Vec<String> {
        std::fs::read_to_string(locales.join(file))
            .unwrap()
            .lines()
            .filter(|l| !l.starts_with(char::is_whitespace))
            .filter_map(|l| l.split_once('=').map(|(id, _)| id.trim().to_string()))
            .collect()
    };
    let (web, tui) = (ids("web.ftl"), ids("tui.ftl"));
    let mut options = 0;
    for entry in std::fs::read_dir(root()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "toml") {
            continue;
        }
        let manifest = Manifest::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        for o in &manifest.launch.option {
            options += 1;
            for (file, have) in [("web.ftl", &web), ("tui.ftl", &tui)] {
                assert!(
                    have.contains(&o.label),
                    "{}: label {} is not in locales/en/{file}",
                    path.display(),
                    o.label
                );
            }
        }
    }
    assert!(options > 0, "no shipped manifest declares a launch option");
}
