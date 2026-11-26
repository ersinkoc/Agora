//! `agora agent explain KIND --file SCREEN [--size COLSxROWS] [--manifests DIR]`: which screen rule decides a captured screen and why
//! every other rule did not, for people writing or fixing a manifest (`docs/specs/agent-manifest.md`). Offline: it reads the kind the
//! way the node does (registry, built-in, then the manifests folder), replays the file on a terminal of the given size and asks the
//! same rule engine detection uses (`Detector::explain`, which decides exactly as `Detector::detect`). No node, no socket, no protocol
//! message.

use std::io::{Read, Write};
use std::path::Path;
use std::process::ExitCode;

use agora_adapters::{Detector, Explanation, RuleOutcome};
use agora_core::AgentState;
use agora_i18n::key;
use agora_vt::{AlacrittyTerminal, Size, Terminal};

use crate::{home, Opts, Say};

/// A captured screen is a few hundred KiB at most (the shipped fixtures are under 100 KiB); a larger file is not a screen.
pub(crate) const MAX_SCREEN_BYTES: u64 = 16 << 20;
/// The size a screen is replayed on when neither `--size` nor its file name (`name@COLSxROWS.screen`) says: the fixtures' own.
const DEFAULT_SIZE: Size = Size {
    cols: 120,
    rows: 40,
};
/// Region text shown per region, and per line: enough to see the rule's evidence, bounded for a long `bottom_lines(200)`.
const MAX_SHOWN_LINES: usize = 40;
const MAX_SHOWN_CHARS: usize = 200;

pub(crate) fn explain(say: &Say, o: &Opts) -> ExitCode {
    let ([kind], Some(file)) = (o.positional.as_slice(), o.file.as_deref()) else {
        say.err(key!("cli-usage"), &[]);
        return ExitCode::from(2);
    };
    let size = match o.size.as_deref() {
        Some(s) => match parse_size(s) {
            Some(size) => size,
            None => {
                say.err(key!("cli-explain-bad-size"), &[]);
                return ExitCode::from(2);
            }
        },
        None => match size_from_name(file) {
            Ok(size) => size.unwrap_or(DEFAULT_SIZE),
            Err(()) => {
                say.err(key!("cli-explain-bad-size"), &[]);
                return ExitCode::from(2);
            }
        },
    };
    let dir = o
        .manifests
        .clone()
        .unwrap_or_else(|| home().join("manifests"));
    // A folder file that cannot be read is skipped by the node too, so the kind falls back to the built-in one (or is unknown): say
    // so first, or the report would explain a manifest other than the one being fixed.
    let (loaded, problems) = agora_node::manifests::load(&dir);
    for (path, reason) in &problems {
        say.err(
            key!("cli-explain-manifest-problem"),
            &[
                (
                    "path",
                    diagnostic_inline(&path.display().to_string()).into(),
                ),
                ("reason", diagnostic(reason).into()),
            ],
        );
    }
    let Some(found) = loaded.iter().find(|l| l.manifest.kind == *kind) else {
        say.err(
            key!("cli-explain-unknown-kind"),
            &[("kind", diagnostic_inline(kind).into())],
        );
        return ExitCode::from(1);
    };
    if found.manifest.rules.is_empty() {
        say.out(
            key!("cli-explain-no-rules"),
            &[("kind", kind.as_str().into())],
        );
        return ExitCode::SUCCESS;
    }
    let bytes = match read_capped(file) {
        Ok(b) => b,
        Err(ReadError::TooLarge) => {
            say.err(
                key!("cli-explain-file-too-large"),
                &[
                    (
                        "path",
                        diagnostic_inline(&file.display().to_string()).into(),
                    ),
                    ("max", (MAX_SCREEN_BYTES as i64).into()),
                ],
            );
            return ExitCode::from(1);
        }
        Err(ReadError::Io(e)) => {
            say.err(
                key!("cli-explain-bad-file"),
                &[
                    (
                        "path",
                        diagnostic_inline(&file.display().to_string()).into(),
                    ),
                    ("reason", diagnostic(&e.to_string()).into()),
                ],
            );
            return ExitCode::from(1);
        }
    };
    let mut term = AlacrittyTerminal::new(size);
    term.feed(&bytes);
    let why = Detector::new(&found.manifest).explain(&term);
    // A reader that went away (`| head`) ends the report quietly instead of a panic.
    let mut stdout = std::io::stdout().lock();
    for line in render(say, kind, found.source.name(), size, &why) {
        if writeln!(stdout, "{line}").is_err() {
            break;
        }
    }
    ExitCode::SUCCESS
}

/// The report, one printed line per entry: what was read, every rule in the order detection reads them, then the decision.
pub(crate) fn render(
    say: &Say,
    kind: &str,
    source: &str,
    size: Size,
    why: &Explanation,
) -> Vec<String> {
    let mut out = vec![say.text(
        key!("cli-explain-header"),
        &[
            ("kind", kind.into()),
            ("source", source.into()),
            ("cols", i64::from(size.cols).into()),
            ("rows", i64::from(size.rows).into()),
        ],
    )];
    for (region, text) in &why.regions {
        out.push(say.text(
            key!("cli-explain-region"),
            &[("region", region.name().into())],
        ));
        let lines: Vec<&str> = text.lines().collect();
        if lines.iter().all(|l| l.trim().is_empty()) {
            out.push(format!(
                "    {}",
                say.text(key!("cli-explain-region-empty"), &[])
            ));
            continue;
        }
        let skip = lines.len().saturating_sub(MAX_SHOWN_LINES);
        if skip > 0 {
            out.push(format!(
                "    {}",
                say.text(
                    key!("cli-explain-region-cut"),
                    &[("count", (skip as i64).into())]
                )
            ));
        }
        for l in &lines[skip..] {
            out.push(format!("    | {}", shown(l)));
        }
    }
    for c in &why.checks {
        let (id, index) = match c.outcome {
            RuleOutcome::Matched => (key!("cli-explain-rule-matched"), None),
            RuleOutcome::OwnMissing => (key!("cli-explain-rule-own-missing"), None),
            RuleOutcome::AllMissing(i) => (key!("cli-explain-rule-all-missing"), Some(i)),
            RuleOutcome::AnyMissing => (key!("cli-explain-rule-any-missing"), None),
            RuleOutcome::NotPresent(i) => (key!("cli-explain-rule-not-present"), Some(i)),
        };
        let mut args = vec![
            ("id", diagnostic_inline(&c.id).into()),
            ("state", state_name(c.state).into()),
            ("priority", i64::from(c.priority).into()),
            ("region", c.region.name().into()),
        ];
        if let Some(i) = index {
            args.push(("index", (i as i64).into()));
        }
        out.push(format!("  {}", say.text(id, &args)));
    }
    out.push(match &why.decided {
        Some(d) => say.text(
            key!("cli-explain-decided"),
            &[
                ("state", state_name(d.state).into()),
                ("id", diagnostic_inline(&d.rule_id).into()),
            ],
        ),
        None => say.text(key!("cli-explain-undecided"), &[]),
    });
    out.push(say.text(key!("cli-explain-note"), &[]));
    out
}

/// The state as the manifest writes it (`state = "working"`): data, like the rule ids around it.
fn state_name(s: AgentState) -> &'static str {
    match s {
        AgentState::Working => "working",
        AgentState::Blocked => "blocked",
        AgentState::Idle => "idle",
        AgentState::Done => "done",
        _ => "unknown",
    }
}

/// A line of screen text made safe to print on the person's own terminal: control characters (a captured title can hold any) are
/// shown as `?`, and the line is cut at [`MAX_SHOWN_CHARS`] characters.
fn shown(line: &str) -> String {
    let mut s: String = line
        .chars()
        .take(MAX_SHOWN_CHARS)
        .map(|c| {
            if c.is_control() || agora_core::text::is_hidden(c) {
                '?'
            } else {
                c
            }
        })
        .collect();
    if line.chars().nth(MAX_SHOWN_CHARS).is_some() {
        s.push('…');
    }
    s
}

/// `COLSxROWS`, each 2 to 1000.
pub(crate) fn parse_size(s: &str) -> Option<Size> {
    let (c, r) = s.split_once(['x', 'X'])?;
    let ok = |v: &str| {
        (!v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
            .then(|| v.parse::<u16>().ok())
            .flatten()
            .filter(|n| (2..=1000).contains(n))
    };
    Some(Size {
        cols: ok(c)?,
        rows: ok(r)?,
    })
}

/// The size a fixture file name carries: `trust@24x20.screen` (`docs/specs/agent-manifest.md`).
pub(crate) fn size_from_name(path: &Path) -> Result<Option<Size>, ()> {
    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return Ok(None);
    };
    let Some((_, annotated)) = stem.rsplit_once('@') else {
        return Ok(None);
    };
    parse_size(annotated).map(Some).ok_or(())
}

/// Text used in an error parameter: retain what it said while making terminal controls and hidden formatting visible.
fn diagnostic(text: &str) -> String {
    agora_core::text::visible(text)
}

/// A value embedded in one diagnostic or report line: unlike a multiline reason, a line break or tab must not create a second
/// apparent diagnostic or move the following text into another column.
fn diagnostic_inline(text: &str) -> String {
    diagnostic(text)
        .replace('\n', "\\u{a}")
        .replace('\t', "\\u{9}")
}

enum ReadError {
    TooLarge,
    Io(std::io::Error),
}

fn read_capped(path: &Path) -> Result<Vec<u8>, ReadError> {
    let f = std::fs::File::open(path).map_err(ReadError::Io)?;
    let mut bytes = Vec::new();
    f.take(MAX_SCREEN_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(ReadError::Io)?;
    if bytes.len() as u64 > MAX_SCREEN_BYTES {
        return Err(ReadError::TooLarge);
    }
    Ok(bytes)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use agora_i18n::catalog::Catalog;

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../manifests/agents/fixtures")
            .join(name)
    }

    fn report(kind: &str, screen: &str) -> Vec<String> {
        let (loaded, _) = agora_node::manifests::load(Path::new("/nonexistent-agora-manifests"));
        let found = loaded
            .iter()
            .find(|l| l.manifest.kind == kind)
            .expect("kind");
        let path = fixture(screen);
        let size = size_from_name(&path).unwrap().unwrap_or(DEFAULT_SIZE);
        let mut term = AlacrittyTerminal::new(size);
        term.feed(&std::fs::read(&path).expect("fixture"));
        let why = Detector::new(&found.manifest).explain(&term);
        render(
            &Say(Catalog::english()),
            kind,
            found.source.name(),
            size,
            &why,
        )
    }

    #[test]
    fn a_captured_dialog_is_explained_with_its_deciding_rule_and_every_other_one() {
        let out = report("claude-code", "claude-code/permission.screen");
        let text = out.join("\n");
        assert!(
            out[0].contains("claude-code")
                && out[0].contains("built in")
                && out[0].contains("120x40"),
            "{text}"
        );
        assert!(text.contains("permission_prompt"), "{text}");
        assert!(text.contains("prompt_box"), "{text}");
        assert!(text.contains("Do you want to proceed?"), "{text}");
        let decided = out.iter().rev().nth(1).expect("decision line");
        assert!(
            decided.contains("blocked") && decided.contains("permission_prompt"),
            "{text}"
        );
    }

    #[test]
    fn the_size_comes_from_the_flag_or_the_file_name() {
        assert_eq!(parse_size("24x20"), Some(Size { cols: 24, rows: 20 }));
        assert_eq!(parse_size("1x20"), None);
        assert_eq!(parse_size("120"), None);
        assert_eq!(
            size_from_name(Path::new("fixtures/codex/trust@24x20.screen")),
            Ok(Some(Size { cols: 24, rows: 20 }))
        );
        assert_eq!(
            size_from_name(Path::new("fixtures/codex/trust.screen")),
            Ok(None)
        );
        let out = report("codex", "codex/trust@24x20.screen");
        assert!(out[0].contains("24x20"), "{}", out.join("\n"));
    }

    #[test]
    fn screen_text_is_printed_without_control_characters_and_cut() {
        assert_eq!(shown("a\u{1b}[31mb\u{7}"), "a?[31mb?");
        let long = "x".repeat(MAX_SHOWN_CHARS + 5);
        assert_eq!(shown(&long).chars().count(), MAX_SHOWN_CHARS + 1);
    }

    #[test]
    fn every_report_line_has_its_text_in_both_locales() {
        for cat in [Catalog::english(), Catalog::for_locale("tr")] {
            let say = Say(cat);
            for id in [
                key!("cli-explain-header"),
                key!("cli-explain-region"),
                key!("cli-explain-region-empty"),
                key!("cli-explain-region-cut"),
                key!("cli-explain-rule-matched"),
                key!("cli-explain-rule-own-missing"),
                key!("cli-explain-rule-all-missing"),
                key!("cli-explain-rule-any-missing"),
                key!("cli-explain-rule-not-present"),
                key!("cli-explain-decided"),
                key!("cli-explain-undecided"),
                key!("cli-explain-note"),
                key!("cli-explain-no-rules"),
                key!("cli-explain-unknown-kind"),
                key!("cli-explain-bad-file"),
                key!("cli-explain-file-too-large"),
                key!("cli-explain-bad-size"),
                key!("cli-explain-manifest-problem"),
            ] {
                assert_ne!(say.text(id, &[]), id, "{id} has no text");
            }
        }
    }

    #[test]
    fn where_the_kind_comes_from_is_said_in_the_person_s_language() {
        let say = Say(Catalog::for_locale("tr"));
        let size = DEFAULT_SIZE;
        let why = Explanation {
            checks: Vec::new(),
            regions: Vec::new(),
            decided: None,
        };
        for (source, want) in [
            ("built-in", "yerleşik"),
            ("folder", "manifest klasöründen"),
            ("registry", "ACP kayıt defterinden"),
        ] {
            let header = &render(&say, "codex", source, size, &why)[0];
            assert!(
                header.contains(want) && !header.contains(source),
                "{header}"
            );
        }
    }
}
