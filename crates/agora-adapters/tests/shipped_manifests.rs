//! The manifests in `manifests/agents/` against the screens captured from the real agents in `fixtures/<kind>/`
//! (`docs/specs/agent-manifest.md`). Each `<name>.screen` is raw terminal output; `<name>.expect` is `<state> <rule id>` or `none`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use agora_adapters::{Detector, Manifest};
use agora_vt::{AlacrittyTerminal, Size, Terminal};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../manifests/agents")
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
        let dir = root().join("fixtures").join(&kind);
        let mut screens = 0;
        for f in std::fs::read_dir(&dir).unwrap() {
            let f = f.unwrap().path();
            if f.extension().is_none_or(|e| e != "screen") {
                continue;
            }
            // The screens were captured on a 120x40 terminal.
            let mut term = AlacrittyTerminal::new(Size {
                cols: 120,
                rows: 40,
            });
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
            screens += 1;
        }
        assert!(screens >= 3, "{kind} needs captured screens");
        kinds += 1;
    }
    assert!(kinds >= 1);
}
