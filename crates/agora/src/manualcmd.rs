//! `agora manual [TOPIC] [--json]` (G10, roadmap M6.5): how to drive and customize Agora, written for coding agents, so an agent can
//! script Agora for its person or write them a theme, a layout, a keybindings file or a plugin.
//!
//! The topics are Markdown files in `docs/manual/`, built into the binary. They are documentation, like the rest of `docs/`, and are
//! English only; Fluent (`locales/<lang>/cli.ftl`) holds what is printed around them: the heading, each topic's one-line summary and
//! the error for an unknown topic, in every language. A Fluent message cannot hold a JSON example as it is (`{` and `}` start
//! placeables, and a line may not start with `[`, `*` or `.`), and the examples must stay exactly what the product reads, since the
//! tests below feed them to the product's own readers.
//!
//! Plain text on stdout, never colour: the reader is a program as often as a person.

use std::fmt::Write as _;
use std::process::ExitCode;

use agora_i18n::{key, Arg};

use crate::{out_line, Opts, Say};

/// One topic of the manual.
pub(crate) struct Topic {
    /// What `agora manual TOPIC` takes, and the file's name in `docs/manual/`.
    pub(crate) id: &'static str,
    /// The Fluent key of its one-line summary.
    pub(crate) summary: &'static str,
    /// The topic's text.
    pub(crate) text: &'static str,
}

/// Every topic, in the order the list shows them.
pub(crate) static TOPICS: [Topic; 7] = [
    Topic {
        id: "scripting",
        summary: key!("cli-manual-topic-scripting"),
        text: include_str!("../../../docs/manual/scripting.md"),
    },
    Topic {
        id: "themes",
        summary: key!("cli-manual-topic-themes"),
        text: include_str!("../../../docs/manual/themes.md"),
    },
    Topic {
        id: "layouts",
        summary: key!("cli-manual-topic-layouts"),
        text: include_str!("../../../docs/manual/layouts.md"),
    },
    Topic {
        id: "keys",
        summary: key!("cli-manual-topic-keys"),
        text: include_str!("../../../docs/manual/keys.md"),
    },
    Topic {
        id: "plugins",
        summary: key!("cli-manual-topic-plugins"),
        text: include_str!("../../../docs/manual/plugins.md"),
    },
    Topic {
        id: "mesh",
        summary: key!("cli-manual-topic-mesh"),
        text: include_str!("../../../docs/manual/mesh.md"),
    },
    Topic {
        id: "files",
        summary: key!("cli-manual-topic-files"),
        text: include_str!("../../../docs/manual/files.md"),
    },
];

fn find(id: &str) -> Option<&'static Topic> {
    TOPICS.iter().find(|t| t.id == id)
}

/// Prints lines until the reader goes away (`agora manual themes | head`).
fn print(text: &str) {
    for line in text.lines() {
        if !out_line(line) {
            break;
        }
    }
}

pub(crate) fn manual(say: &Say, o: &Opts, topic: Option<&str>) -> ExitCode {
    if !o.positional.is_empty() {
        say.err(key!("cli-bad-arguments"), &[]);
        say.err(key!("cli-usage"), &[]);
        return ExitCode::from(2);
    }
    let Some(id) = topic else {
        if o.json {
            let list: Vec<serde_json::Value> = TOPICS
                .iter()
                .map(|t| serde_json::json!({"id": t.id, "summary": say.text(t.summary, &[])}))
                .collect();
            print(&serde_json::Value::Array(list).to_string());
        } else {
            let width = TOPICS.iter().map(|t| t.id.len()).max().unwrap_or(0);
            let mut text = say.text(key!("cli-manual-heading"), &[]);
            for t in &TOPICS {
                let summary = say.text(t.summary, &[]);
                // Writing to a String cannot fail.
                let _ = write!(text, "\n  {:width$}  {summary}", t.id);
            }
            print(&text);
        }
        return ExitCode::SUCCESS;
    };
    let Some(t) = find(id) else {
        let ids: Vec<&str> = TOPICS.iter().map(|t| t.id).collect();
        say.err(
            key!("cli-manual-unknown"),
            &[
                ("topic", Arg::Str(id.to_string())),
                ("topics", Arg::Str(ids.join(", "))),
            ],
        );
        return ExitCode::from(2);
    };
    if o.json {
        print(
            &serde_json::json!({"id": t.id, "summary": say.text(t.summary, &[]), "text": t.text})
                .to_string(),
        );
    } else {
        print(t.text);
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    //! The manual must say what the product does: every command line in it is read by the real command line reader, every example
    //! by the reader the product uses for that file, and the names, limits and lists it states are compared with the constants they
    //! come from. The web window's formats (keybindings, plugins) and its side of themes and layouts are checked by
    //! `apps/web/src/manual.test.ts`.

    use super::*;
    use std::collections::BTreeSet;

    use agora_i18n::Catalog;
    use serde_json::Value;

    fn topic(id: &str) -> &'static str {
        find(id).map(|t| t.text).unwrap_or_default()
    }

    /// The fenced blocks of a topic: (info string, body).
    fn blocks(text: &str) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut open: Option<(String, Vec<&str>)> = None;
        for line in text.lines() {
            match (&mut open, line.strip_prefix("```")) {
                (None, Some(info)) => open = Some((info.trim().to_string(), Vec::new())),
                (Some(_), Some("")) => {
                    if let Some((info, lines)) = open.take() {
                        out.push((info, lines.join("\n") + "\n"));
                    }
                }
                (Some((_, lines)), _) => lines.push(line),
                (None, None) => {}
            }
        }
        assert!(open.is_none(), "a fenced block is not closed");
        out
    }

    /// The text of a topic outside its fenced blocks.
    fn prose(text: &str) -> String {
        let mut out = String::new();
        let mut inside = false;
        for line in text.lines() {
            if line.starts_with("```") {
                inside = !inside;
                continue;
            }
            if !inside {
                out.push_str(line);
                out.push('\n');
            }
        }
        out
    }

    /// Every inline code span of a topic's prose (a span may run over a line break).
    fn spans(text: &str) -> Vec<String> {
        prose(text)
            .split('`')
            .skip(1)
            .step_by(2)
            .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect()
    }

    /// A command line as a shell splits it (quotes and leading `NAME=value` assignments), without a trailing `# comment`.
    fn words(line: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut word = String::new();
        let mut quote: Option<char> = None;
        let mut any = false;
        for c in line.chars() {
            match (quote, c) {
                (Some(q), c) if c == q => quote = None,
                (Some(_), c) => word.push(c),
                (None, '"' | '\'') => {
                    quote = Some(c);
                    any = true;
                }
                (None, '#') if !any && word.is_empty() => break,
                (None, c) if c.is_whitespace() => {
                    if any || !word.is_empty() {
                        out.push(std::mem::take(&mut word));
                        any = false;
                    }
                }
                (None, c) => {
                    word.push(c);
                    any = true;
                }
            }
        }
        if any || !word.is_empty() {
            out.push(word);
        }
        while out.first().is_some_and(|w| {
            w.split_once('=').is_some_and(|(k, _)| {
                !k.is_empty() && k.chars().all(|c| c.is_ascii_uppercase() || c == '_')
            })
        }) {
            out.remove(0);
        }
        out
    }

    /// Every `agora …` command line of a topic: in its shell blocks, and as inline code.
    fn command_lines(text: &str) -> Vec<String> {
        let from_blocks = blocks(text)
            .into_iter()
            .filter(|(info, _)| info == "sh")
            .flat_map(|(_, body)| {
                body.lines()
                    .map(str::trim)
                    .filter(|l| l.starts_with("agora "))
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            });
        from_blocks
            .chain(
                spans(text)
                    .into_iter()
                    .filter(|s| s == "agora" || s.starts_with("agora ")),
            )
            .collect()
    }

    fn english_usage() -> Vec<String> {
        Catalog::english()
            .render("cli-usage", &[])
            .unwrap_or_default()
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| l.starts_with("agora"))
            .collect()
    }

    #[test]
    fn the_topics_are_the_files_of_docs_manual() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/manual");
        let on_disk: BTreeSet<String> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter_map(|n| n.strip_suffix(".md").map(str::to_string))
            .collect();
        let listed: BTreeSet<String> = TOPICS.iter().map(|t| t.id.to_string()).collect();
        assert_eq!(on_disk, listed);
        for t in &TOPICS {
            assert_eq!(
                t.summary,
                format!("cli-manual-topic-{}", t.id),
                "{}: the summary key names the topic",
                t.id
            );
            assert!(t.text.starts_with("# "), "{}: starts with its title", t.id);
            // The text is printed on a terminal: no links or images, which would point nowhere there.
            assert!(!t.text.contains("]("), "{}: a Markdown link", t.id);
            // Every topic ends with a minimal example.
            assert!(t.text.contains("\n## Example\n"), "{}: no example", t.id);
        }
    }

    #[test]
    fn every_summary_and_message_is_in_english_and_turkish() {
        let en = Catalog::english();
        let tr = agora_i18n::lint::ftl_ids(include_str!("../../../locales/tr/cli.ftl"));
        let ids = TOPICS
            .iter()
            .map(|t| t.summary)
            .chain([key!("cli-manual-heading"), key!("cli-manual-unknown")]);
        for id in ids {
            assert!(en.render(id, &[]).is_some(), "{id} is not in English");
            assert!(tr.contains(id), "{id} is not in Turkish");
        }
        for usage in [
            include_str!("../../../locales/en/cli.ftl"),
            include_str!("../../../locales/tr/cli.ftl"),
        ] {
            assert!(
                usage.contains("      agora manual ["),
                "the usage page lists `agora manual`"
            );
        }
    }

    #[test]
    fn every_command_line_is_one_the_command_line_reader_takes() {
        let usage = english_usage();
        let mut seen = 0;
        for t in &TOPICS {
            for line in command_lines(t.text) {
                let args = words(&line);
                assert_eq!(
                    args.first().map(String::as_str),
                    Some("agora"),
                    "{}: {line}",
                    t.id
                );
                let args = &args[1..];
                // `main` answers these before it reads a command.
                if matches!(args, [h] if h == "--help" || h == "--version") {
                    seen += 1;
                    continue;
                }
                assert!(
                    !args.iter().any(|a| a.starts_with('[') || a.contains('|')),
                    "{}: write a real command line, not a usage pattern: {line}",
                    t.id
                );
                let Some((cmd, opts)) = crate::split(args) else {
                    panic!("{}: the reader refuses the flags of: {line}", t.id);
                };
                assert!(
                    crate::route(&cmd, &opts).is_some(),
                    "{}: no such command: {line}",
                    t.id
                );
                // Every flag is one this command takes: on its line of the usage page.
                // A route that takes its second word as a value (`agora manual TOPIC`, `agora join ADDRESS`) is listed by its first.
                let on_page = |name: &str| -> Vec<&String> {
                    usage
                        .iter()
                        .filter(|u| *u == name || u.starts_with(&format!("{name} ")))
                        .collect()
                };
                let mut name = std::iter::once("agora")
                    .chain(cmd.iter().copied())
                    .collect::<Vec<_>>()
                    .join(" ");
                let mut lines = on_page(&name);
                if lines.is_empty() && cmd.len() == 2 {
                    name = format!("agora {}", cmd[0]);
                    lines = on_page(&name);
                }
                assert!(!lines.is_empty(), "{}: not on the usage page: {name}", t.id);
                for flag in args.iter().filter(|a| a.starts_with("--")) {
                    assert!(
                        lines.iter().any(|u| u
                            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                            .any(|w| w == flag)),
                        "{}: `{name}` does not take {flag}: {line}",
                        t.id
                    );
                }
                seen += 1;
            }
        }
        // The extraction finds what is there: a change that hides every command line fails here, not silently.
        assert!(seen >= 40, "only {seen} command lines found");
        assert!(command_lines(topic("scripting")).len() >= 15);
    }

    #[test]
    fn the_extractors_find_what_they_should() {
        assert_eq!(
            words(r#"AGORA_LANG=en agora agent prompt agent-1 "fix it now" --wait  # comment"#),
            [
                "agora",
                "agent",
                "prompt",
                "agent-1",
                "fix it now",
                "--wait"
            ]
        );
        assert!(crate::split(&words("agora agent wait a --timeout SECS")[1..]).is_none());
        assert!(crate::route(&["agent", "fly"], &Opts::default()).is_none());
        let b = blocks("x\n```json a\n{}\n```\ny\n");
        assert_eq!(b, [("json a".to_string(), "{}\n".to_string())]);
        assert_eq!(spans("a `b c` d `e\nf` g"), ["b c", "e f"]);
    }

    #[test]
    fn the_scripting_topic_states_the_exit_codes_and_timings() {
        use crate::orchestrate::{
            EXIT_BLOCKED, EXIT_EXITED, EXIT_REFUSED, EXIT_TIMEOUT, POLL, SETTLE,
        };
        let s = topic("scripting");
        for (code, word) in [
            (EXIT_BLOCKED, "blocked"),
            (EXIT_EXITED, "exited"),
            (EXIT_TIMEOUT, "`--timeout`"),
            (EXIT_REFUSED, "the prompt was not sent"),
        ] {
            assert!(
                s.contains(&format!("| {code} | {word}")),
                "exit code {code}"
            );
        }
        assert!(s.contains(&format!("every {} ms", POLL.as_millis())));
        assert!(s.contains(&format!("after {} s", SETTLE.as_secs())));
        // The words of `agora events` and `agora agent list` are the ones the code prints.
        let src = format!(
            "{}{}",
            include_str!("orchestrate.rs"),
            include_str!("main.rs")
        );
        for w in [
            "present",
            "added",
            "state",
            "exited",
            "running",
            "restarted",
            "interrupted",
            "removed",
            "event",
            "agent_id",
            "kind",
            "restarts",
            "time_ms",
            "previous",
            "stopped",
            "still-running",
            "resume",
            "start-again",
            "installed",
            "missing",
        ] {
            assert!(s.contains(&format!("`{w}`")), "{w} is not in the topic");
            assert!(src.contains(&format!("\"{w}\"")), "{w} is not printed");
        }
        assert!(crate::socket_path(&Opts::default()).ends_with("node.sock"));
        assert!(s.contains("`node.sock`"));
    }

    fn theme_examples() -> Vec<String> {
        blocks(topic("themes"))
            .into_iter()
            .filter(|(info, body)| info.starts_with("json") && body.contains("\"agora-theme\""))
            .map(|(_, body)| body)
            .collect()
    }

    #[test]
    fn the_theme_examples_are_accepted_by_the_terminal_client_without_a_warning() {
        use agora_tui::theme::file::{read_text, Read};
        let examples = theme_examples();
        assert_eq!(examples.len(), 2);
        assert!(blocks(topic("themes"))
            .iter()
            .any(|(i, _)| i == "json agora-theme"));
        for e in examples {
            match read_text(&e) {
                Read::Ok { warnings, .. } => assert!(warnings.is_empty(), "{warnings:?}\n{e}"),
                Read::Refused(p) => panic!("refused: {p:?}\n{e}"),
            }
        }
    }

    #[test]
    fn the_themes_topic_names_every_key_and_limit() {
        use agora_tui::theme::{
            built_ins, file::MAX_FILE_BYTES, file::MAX_ISSUES, floor_of, folder, ChromeKind,
            CHROME, COLOUR_TOKENS, DERIVED, FONT_TOKENS, MAX_NAME, MIN_BASE_TEXT, REGIONS,
            REGION_PARTS, SHAPE_TOKENS, SYNTAX_KEYS, TERMINAL_KEYS,
        };
        let s = topic("themes");
        let built = built_ins();
        let names = COLOUR_TOKENS
            .iter()
            .chain(&REGIONS)
            .chain(&REGION_PARTS)
            .chain(&TERMINAL_KEYS)
            .chain(&SYNTAX_KEYS)
            .chain(&FONT_TOKENS)
            .copied()
            .chain(SHAPE_TOKENS.iter().map(|(n, ..)| *n))
            .chain(built.iter().map(|b| b.id));
        for n in names {
            assert!(s.contains(&format!("`{n}`")), "{n} is not in the topic");
        }
        for (n, min, max) in SHAPE_TOKENS {
            let min = if n == "text-16" { MIN_BASE_TEXT } else { min };
            assert!(
                s.contains(&format!("`{n}` (")) && s.contains(&format!("{min}–{max} px")),
                "{n}'s range"
            );
        }
        assert!(s.contains(&format!("(`{}`, `{}`, ...)", DERIVED[0].0, DERIVED[5].0)));
        for (key, kind) in CHROME {
            let row = s
                .lines()
                .find(|l| l.starts_with(&format!("| `{key}` |")))
                .unwrap_or_else(|| panic!("no row for {key}"));
            match kind {
                ChromeKind::List { values, default } => {
                    let d = values[usize::from(default)];
                    assert!(row.contains(&format!("`{d}` (default)")), "{key}'s default");
                    for v in values {
                        assert!(row.contains(&format!("`{v}`")), "{key}: {v}");
                    }
                }
                ChromeKind::Px { min, max, default } => {
                    assert!(
                        row.contains(&format!("{min}–{max} px (default {default})")),
                        "{key}"
                    );
                }
            }
        }
        for n in [
            "danger",
            "on-danger",
            "diff-removed",
            "approval-card",
            "approval",
            "focus-ring",
            "focus",
            "needs-you",
            "attention",
            "plugin-marker",
        ] {
            assert!(
                floor_of(n).is_some() && s.contains(&format!("`{n}`")),
                "{n}"
            );
        }
        assert!(s.contains(&format!("larger than {} KiB", MAX_FILE_BYTES / 1024)));
        assert!(s.contains(&format!("At most {MAX_ISSUES} problems")));
        assert!(s.contains(&format!("1 to {MAX_NAME} characters")));
        assert!(s.contains(&format!("`{}/NAME.json`", folder::FOLDER)));
        assert!(s.contains(&format!("at most {}", folder::MAX_FILES)));
        assert!(s.contains(&format!("`{}`", folder::CHOICE_FILE)));
        // The lines `tui-theme` may hold are the ones the client reads back.
        for line in [
            "built-in slate",
            "file dark NAME.json",
            "file light NAME.json",
        ] {
            assert_ne!(
                folder::Choice::parse(line),
                folder::Choice::Terminal,
                "{line}"
            );
        }
    }

    #[test]
    fn the_keys_topic_matches_the_terminal_clients_commands_and_reader() {
        use agora_tui::keys::commands::{commands, KEPT_KEYS};
        use agora_tui::keys::file::{read, FILE, MAX_FILE_BYTES};
        use agora_tui::keys::strokes::{parse_binding, stroke_text};
        let s = topic("keys");
        // The terminal client's table names exactly its commands (`a-1` … `a-9` names nine).
        let mut named = BTreeSet::new();
        for row in s
            .lines()
            .skip_while(|l| !l.starts_with("| Terminal client command |"))
            .skip(2)
            .take_while(|l| l.starts_with("| `"))
        {
            let cell = row.split('|').nth(1).unwrap_or_default();
            let codes: Vec<&str> = cell.split('`').skip(1).step_by(2).collect();
            match (codes.as_slice(), cell.contains('…')) {
                ([first, last], true) => {
                    let (stem, a) = first.rsplit_once('-').unwrap();
                    let b = last.rsplit_once('-').unwrap().1;
                    for i in a.parse::<u32>().unwrap()..=b.parse::<u32>().unwrap() {
                        named.insert(format!("{stem}-{i}"));
                    }
                }
                _ => named.extend(codes.iter().map(|c| c.to_string())),
            }
        }
        let real: BTreeSet<String> = commands().into_iter().map(|c| c.id).collect();
        assert_eq!(named, real);
        for (key, _) in KEPT_KEYS {
            assert!(s.contains(&format!("`{key}`")), "{key}");
        }
        assert!(s.contains(&format!("`{FILE}`")) && topic("files").contains(&format!("`{FILE}`")));
        assert!(s.contains(&format!("at most {} KiB", MAX_FILE_BYTES / 1024)));
        assert!(topic("files").contains(&format!("over {} KiB", MAX_FILE_BYTES / 1024)));
        // Every file in the topic is one the terminal client reads, and the example takes no key it keeps.
        let files: Vec<String> = blocks(s)
            .into_iter()
            .filter(|(info, _)| info.starts_with("json"))
            .map(|(_, b)| b)
            .collect();
        assert_eq!(files.len(), 2);
        for f in &files {
            assert!(read(f).is_ok(), "{:?}\n{f}", read(f));
        }
        let example = blocks(s)
            .into_iter()
            .find(|(info, _)| info == "json agora-keys")
            .map(|(_, b)| b)
            .unwrap();
        for entry in read(&example).unwrap().entries {
            for key in &entry.keys {
                for stroke in parse_binding(key).unwrap() {
                    let t = stroke_text(&stroke);
                    assert!(!KEPT_KEYS.iter().any(|(k, _)| *k == t), "{t}");
                }
            }
        }
    }

    #[test]
    fn the_layout_example_is_read_by_the_terminal_client_as_it_is_written() {
        use agora_tui::layout::tree::{deserialise_layout, layout_value};
        let examples: Vec<String> = blocks(topic("layouts"))
            .into_iter()
            .filter(|(info, _)| info == "json agora-layout")
            .map(|(_, b)| b)
            .collect();
        assert_eq!(examples.len(), 1);
        for e in examples {
            let layout = deserialise_layout(&e).unwrap_or_else(|err| panic!("{err:?}"));
            let written: Value = serde_json::from_str(&e).unwrap();
            assert_eq!(layout_value(&layout), written, "normalised on reading");
        }
    }

    #[test]
    fn the_layouts_topic_names_the_panels_and_limits() {
        use agora_tui::layout::store::{CURRENT_FILE, FOLDER, MAX_FILE_BYTES, MAX_NAME, MAX_SAVED};
        use agora_tui::layout::tree::{MAX_DEPTH, MAX_PANES, MIN_RATIO};
        use agora_tui::layout::view::{draws, Draws};
        let s = topic("layouts");
        let table: Vec<&str> = s
            .lines()
            .skip_while(|l| !l.starts_with("| Panel |"))
            .skip(2)
            .take_while(|l| l.starts_with("| `"))
            .filter_map(|l| l.split('`').nth(1))
            .collect();
        assert_eq!(table.len(), 15);
        for p in &table {
            assert_ne!(draws(p), Draws::Unknown, "{p}");
        }
        let drawn = s
            .split("The terminal client draws")
            .nth(1)
            .and_then(|r| r.split(", and shows").next())
            .unwrap();
        for p in drawn.split('`').skip(1).step_by(2) {
            assert!(
                !matches!(draws(p), Draws::Elsewhere(_) | Draws::Unknown),
                "{p}"
            );
        }
        for p in table
            .iter()
            .filter(|p| !matches!(draws(p), Draws::Elsewhere(_)))
        {
            assert!(drawn.contains(&format!("`{p}`")), "{p} is drawn");
        }
        assert!(s.contains(&format!("At most {MAX_PANES} panes")));
        assert!(s.contains(&format!("at most {MAX_DEPTH} levels")));
        assert!(s.contains(&format!("less than {} %", (MIN_RATIO * 100.0).round())));
        assert!(s.contains(&format!("`{FOLDER}/NAME.json`")));
        assert!(s.contains(&format!("`{CURRENT_FILE}`")));
        assert!(s.contains(&format!("at most {MAX_NAME} characters")));
        assert!(s.contains(&format!("At most {MAX_SAVED} are listed")));
        assert!(s.contains(&format!("at most {} KiB", MAX_FILE_BYTES / 1024)));
    }

    fn mesh_tools() -> Vec<Value> {
        let mut list = agora_mesh::mcp::tools();
        agora_mesh::mcp::with_fleet(&mut list);
        list.extend(agora_mesh::mcp::spawn_tools());
        list
    }

    fn properties(tool: &Value) -> Vec<String> {
        tool["inputSchema"]["properties"]
            .as_object()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    }

    #[test]
    fn the_mesh_topic_lists_every_tool_with_its_arguments() {
        let s = topic("mesh");
        let rows: Vec<&str> = s
            .lines()
            .skip_while(|l| !l.starts_with("| Tool |"))
            .skip(2)
            .take_while(|l| l.starts_with("| `"))
            .collect();
        let tools = mesh_tools();
        let named: BTreeSet<&str> = rows.iter().filter_map(|r| r.split('`').nth(1)).collect();
        let real: BTreeSet<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        assert_eq!(named, real);
        for t in &tools {
            let name = t["name"].as_str().unwrap_or_default();
            let row = rows
                .iter()
                .find(|r| r.starts_with(&format!("| `{name}` |")))
                .unwrap();
            let input = row.split('|').nth(2).unwrap_or_default();
            for p in properties(t) {
                assert!(input.contains(&format!("`{p}`")), "{name}: {p}");
            }
        }
    }

    #[test]
    fn the_mesh_example_calls_real_tools_with_real_arguments() {
        let tools = mesh_tools();
        let calls: Vec<Value> = blocks(topic("mesh"))
            .into_iter()
            .filter(|(info, _)| info == "json mcp-calls")
            .flat_map(|(_, b)| serde_json::from_str::<Vec<Value>>(&b).unwrap())
            .collect();
        assert_eq!(calls.len(), 3);
        for c in calls {
            let name = c["name"].as_str().unwrap();
            let tool = tools
                .iter()
                .find(|t| t["name"] == name)
                .unwrap_or_else(|| panic!("{name}"));
            let props = properties(tool);
            let args = c["arguments"].as_object().unwrap();
            for k in args.keys() {
                assert!(props.contains(k), "{name}: {k}");
            }
            for r in tool["inputSchema"]["required"]
                .as_array()
                .into_iter()
                .flatten()
            {
                assert!(args.contains_key(r.as_str().unwrap()), "{name} needs {r}");
            }
        }
    }

    #[test]
    fn the_mesh_topic_states_the_brokers_limits_and_real_error_codes() {
        let l = agora_mesh::broker::Limits::default();
        let s = topic("mesh");
        assert!(s.contains(&format!("at most {} KiB", l.max_body_bytes / 1024)));
        assert!(s.contains(&format!("at most {} `refs`", l.max_refs)));
        assert_eq!(l.window_ms, 60_000);
        assert!(s.contains(&format!(
            "at most {} messages a minute",
            l.messages_per_window
        )));
        assert!(s.contains(&format!("at most {} open `ask`s", l.max_open_asks)));
        assert!(s.contains(&format!("newest {} messages", l.inbox_capacity)));
        assert!(s.contains(&format!("(default {})", l.default_ask_timeout_ms / 1000)));
        let errors = s
            .split("## Errors")
            .nth(1)
            .and_then(|r| r.split("\n## ").next())
            .unwrap();
        let mut n = 0;
        for code in errors.split('`').skip(1).step_by(2) {
            if code.contains('.')
                && code
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '.' || c == '_')
            {
                assert!(
                    !matches!(
                        agora_protocol::ErrorCode::parse(code),
                        agora_protocol::ErrorCode::Unknown(_)
                    ),
                    "{code}"
                );
                n += 1;
            }
        }
        assert!(n >= 10);
        assert!(s.contains(&format!("`{}`", agora_mesh::mcp::SCOPE_HUB)));
    }

    #[test]
    fn the_files_topic_names_the_real_files() {
        let s = topic("files");
        for name in [
            format!("`{}/NAME.json`", agora_tui::theme::folder::FOLDER),
            format!("`{}/NAME.json`", agora_tui::layout::store::FOLDER),
            format!("`{}`", agora_tui::theme::folder::CHOICE_FILE),
            format!("`{}`", agora_tui::layout::store::CURRENT_FILE),
            format!("`{}`", crate::frontdoor::MACHINE_FILE),
            format!("`data/{}`", agora_node::daemon::POLICY_FILE),
            format!("`data/{}`", agora_node::approvals::TRUSTED_DEVICES_FILE),
            format!("at most {}", agora_tui::theme::folder::MAX_FILES),
            format!("over {} KiB", agora_tui::theme::file::MAX_FILE_BYTES / 1024),
            format!(
                "over {} KiB",
                agora_tui::layout::store::MAX_FILE_BYTES / 1024
            ),
            format!("at most {} listed", agora_tui::layout::store::MAX_SAVED),
            format!("over {} MiB", crate::frontdoor::LOG_LIMIT >> 20),
        ] {
            assert!(s.contains(&name), "{name}");
        }
        // The folders are where the commands put things.
        let home = crate::home();
        assert_eq!(crate::socket_path(&Opts::default()), home.join("node.sock"));
        for dir in [
            "data/",
            "link/",
            "hub/",
            "worktrees/",
            "manifests/",
            "agents/",
            "adapters/",
            "m/",
            "logs/",
        ] {
            assert!(s.contains(&format!("`{dir}")), "{dir}");
        }
        let main = include_str!("main.rs");
        for dir in ["\"data\"", "\"worktrees\"", "\"manifests\""] {
            assert!(main.contains(&format!("h.join({dir})")), "{dir}");
        }
        let front = include_str!("frontdoor.rs");
        assert!(front.contains("home.join(\"hub\")") && front.contains("home.join(\"logs\")"));
        assert!(include_str!("adaptercmd.rs").contains("h.join(\"adapters\")"));
    }

    #[test]
    fn the_list_and_a_topic_print_plain_text() {
        // `print` writes each line as it is: a topic's first line is its title, and no line holds an escape sequence.
        for t in &TOPICS {
            assert!(!t.text.contains('\u{1b}'), "{}", t.id);
            assert!(
                t.text.ends_with('\n') && !t.text.ends_with("\n\n"),
                "{}",
                t.id
            );
        }
        let say = Say(Catalog::english());
        assert!(say
            .text(key!("cli-manual-heading"), &[])
            .contains("agora manual TOPIC"));
        let unknown = say.text(
            key!("cli-manual-unknown"),
            &[
                ("topic", Arg::Str("x".into())),
                ("topics", Arg::Str("a, b".into())),
            ],
        );
        assert!(
            unknown.contains("\"x\"") && unknown.ends_with("a, b."),
            "{unknown}"
        );
    }
}
