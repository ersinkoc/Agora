//! The keybindings file, version 1 (`docs/ux/keybindings.md`, "The keybindings file"; the web's `keys/file.ts`): the person's own keys
//! for commands, over the defaults. The same file in both clients: the web exports `keybindings.json`, and that file put in the Agora
//! folder (`$AGORA_HOME`, else `~/.agora`) is the terminal client's.
//!
//! Like a theme file it is data, never run: read as JSON only, its fields a closed set, every problem reported at once as a stable code
//! (`tui-keys-error-*` in `locales/`), and any problem refuses the whole file (the defaults stay). A file is valid here exactly when it is
//! valid in the web window. A command this client does not have (a web window's) is kept and binds nothing here.

use std::path::Path;

use agora_i18n::{key, Arg};
use serde_json::Value;

use super::commands::{answers_a_request, is_command_id, FixedReason, FIXED_COMMANDS, FIXED_KEYS};
use super::strokes::{binding_text, parse_binding};
use crate::render::{plain, Text};

/// The file's name in the Agora folder.
pub const FILE: &str = "keybindings.json";
pub const FORMAT: u64 = 1;
pub const MAX_FILE_BYTES: u64 = 64 * 1024;
pub const MAX_ENTRIES: usize = 200;
pub const MAX_KEYS_PER_COMMAND: usize = 4;
pub const MAX_ISSUES: usize = 50;

/// One command's keys as the file gives them (each in its one spelling); `[]` takes every key from it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub command: String,
    pub keys: Vec<String>,
}

/// A file that was read: its entries in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyFile {
    pub entries: Vec<Entry>,
}

/// One problem, by the web's codes (`keys-error-<code>`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Issue {
    Json,
    TooBig,
    Shape,
    Newer,
    TooMany,
    Unreadable,
    Field {
        at: String,
    },
    Command {
        command: String,
    },
    Request {
        command: String,
    },
    FixedCommand {
        command: String,
    },
    Twice {
        command: String,
    },
    TooManyKeys {
        command: String,
    },
    Key {
        command: String,
        key: String,
    },
    FixedKey {
        command: String,
        key: String,
        reason: FixedReason,
    },
    More {
        count: usize,
    },
}

/// Text from the file said back in a problem: no control or direction characters, at most 64 characters.
fn shown(v: &Value) -> String {
    let text = match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    plain(&text)
        .chars()
        .filter(|c| *c != '\n')
        .take(64)
        .collect()
}

fn fixed_reason(stroke_text: &str) -> Option<FixedReason> {
    FIXED_KEYS
        .iter()
        .find(|(k, _)| *k == stroke_text)
        .map(|(_, r)| *r)
}

/// Reads a keybindings file from its text. Never panics.
pub fn read(text: &str) -> Result<KeyFile, Vec<Issue>> {
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(vec![Issue::TooBig]);
    }
    let Ok(raw) = serde_json::from_str::<Value>(text) else {
        return Err(vec![Issue::Json]);
    };
    read_value(&raw)
}

/// An integer version, as JavaScript's `Number.isInteger` reads one (`1.0` is 1).
fn version(v: &Value) -> Option<f64> {
    let n = v.as_f64()?;
    (n.fract() == 0.0 && n >= 1.0).then_some(n)
}

/// Reads a keybindings file already parsed from JSON.
pub fn read_value(raw: &Value) -> Result<KeyFile, Vec<Issue>> {
    let (Some(top), Some(v)) = (raw.as_object(), raw.get("version").and_then(version)) else {
        return Err(vec![Issue::Shape]);
    };
    let Some(bindings) = top.get("bindings").and_then(Value::as_array) else {
        return Err(vec![Issue::Shape]);
    };
    if v > FORMAT as f64 {
        return Err(vec![Issue::Newer]);
    }
    let mut issues = Vec::new();
    for k in top.keys() {
        if k != "version" && k != "bindings" {
            issues.push(Issue::Field {
                at: shown(&Value::String(k.clone())),
            });
        }
    }
    if bindings.len() > MAX_ENTRIES {
        issues.push(Issue::TooMany);
        return Err(issues);
    }
    let mut entries = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for (i, e) in bindings.iter().enumerate() {
        let (Some(obj), Some(command), Some(keys)) = (
            e.as_object(),
            e.get("command").and_then(Value::as_str),
            e.get("keys").and_then(Value::as_array),
        ) else {
            issues.push(Issue::Shape);
            continue;
        };
        for k in obj.keys() {
            if k != "command" && k != "keys" {
                issues.push(Issue::Field {
                    at: format!("bindings[{i}].{}", shown(&Value::String(k.clone()))),
                });
            }
        }
        let command = command.to_string();
        if !is_command_id(&command) {
            issues.push(Issue::Command {
                command: shown(&Value::String(command)),
            });
            continue;
        }
        if answers_a_request(&command) {
            issues.push(Issue::Request { command });
            continue;
        }
        if seen.contains(&command) {
            issues.push(Issue::Twice { command });
            continue;
        }
        seen.push(command.clone());
        if FIXED_COMMANDS.contains(&command.as_str()) {
            issues.push(Issue::FixedCommand { command });
            continue;
        }
        if keys.len() > MAX_KEYS_PER_COMMAND {
            issues.push(Issue::TooManyKeys { command });
            continue;
        }
        let mut texts: Vec<String> = Vec::new();
        for k in keys {
            let Some(b) = k.as_str().and_then(parse_binding) else {
                issues.push(Issue::Key {
                    command: command.clone(),
                    key: shown(k),
                });
                continue;
            };
            let text = binding_text(&b);
            if let Some(reason) = b
                .iter()
                .find_map(|s| fixed_reason(&super::strokes::stroke_text(s)))
            {
                issues.push(Issue::FixedKey {
                    command: command.clone(),
                    key: text,
                    reason,
                });
                continue;
            }
            if !texts.contains(&text) {
                texts.push(text);
            }
        }
        entries.push(Entry {
            command,
            keys: texts,
        });
    }
    if issues.is_empty() {
        return Ok(KeyFile { entries });
    }
    if issues.len() > MAX_ISSUES {
        let more = issues.len() - MAX_ISSUES;
        issues.truncate(MAX_ISSUES);
        issues.push(Issue::More { count: more });
    }
    Err(issues)
}

/// Reads the file at `path`: `Ok(None)` when there is none (the defaults), a problem when it is not a file, larger than
/// [`MAX_FILE_BYTES`] (asked of the file system before reading), not text, or not a valid keybindings file.
pub fn load(path: &Path) -> Result<Option<KeyFile>, Vec<Issue>> {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(vec![Issue::Unreadable]),
    };
    if !meta.is_file() {
        return Err(vec![Issue::Unreadable]);
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(vec![Issue::TooBig]);
    }
    let bytes = std::fs::read(path).map_err(|_| vec![Issue::Unreadable])?;
    // The file may have grown between the two looks.
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(vec![Issue::TooBig]);
    }
    // As the web window reads a chosen file (`File.text()`): a byte-order mark is dropped and a byte that is not UTF-8 becomes U+FFFD
    // (as the theme files, audit F1800), so a file saved by Windows PowerShell 5.1 or an older Notepad reads the same in both clients.
    let text = String::from_utf8_lossy(&bytes);
    read(text.strip_prefix('\u{feff}').unwrap_or(&text)).map(Some)
}

/// A problem in words.
pub fn issue_text(issue: &Issue, t: Text<'_>) -> String {
    let s = |v: &String| Arg::Str(v.clone());
    match issue {
        Issue::Json => t(key!("tui-keys-error-json"), &[]),
        Issue::TooBig => t(key!("tui-keys-error-too-big"), &[]),
        Issue::Shape => t(key!("tui-keys-error-shape"), &[]),
        Issue::Newer => t(key!("tui-keys-error-newer"), &[]),
        Issue::TooMany => t(key!("tui-keys-error-too-many"), &[]),
        Issue::Unreadable => t(key!("tui-keys-error-unreadable"), &[]),
        Issue::Field { at } => t(key!("tui-keys-error-field"), &[("field", s(at))]),
        Issue::Command { command } => t(key!("tui-keys-error-command"), &[("command", s(command))]),
        Issue::Request { command } => t(key!("tui-keys-error-request"), &[("command", s(command))]),
        Issue::FixedCommand { command } => t(
            key!("tui-keys-error-fixed-command"),
            &[("command", s(command))],
        ),
        Issue::Twice { command } => t(key!("tui-keys-error-twice"), &[("command", s(command))]),
        Issue::TooManyKeys { command } => t(
            key!("tui-keys-error-too-many-keys"),
            &[("command", s(command))],
        ),
        Issue::Key { command, key } => t(
            key!("tui-keys-error-key"),
            &[("command", s(command)), ("key", s(key))],
        ),
        Issue::FixedKey {
            command,
            key,
            reason,
        } => t(
            key!("tui-keys-error-fixed-key"),
            &[
                ("command", s(command)),
                ("key", s(key)),
                (
                    "reason",
                    Arg::Str(t(
                        match reason {
                            FixedReason::SafeMode => key!("tui-keys-fixed-safe-mode"),
                            FixedReason::Navigation => key!("tui-keys-fixed-navigation"),
                            FixedReason::Terminal => key!("tui-keys-fixed-terminal"),
                            FixedReason::Editing => key!("tui-keys-fixed-editing"),
                        },
                        &[],
                    )),
                ),
            ],
        ),
        Issue::More { count } => t(
            key!("tui-keys-error-more"),
            &[("count", Arg::Num(i64::try_from(*count).unwrap_or(i64::MAX)))],
        ),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn the_docs_example_reads_and_keys_take_their_one_spelling() {
        let f = read(
            r#"{ "version": 1, "bindings": [
                { "command": "palette.open", "keys": ["ctrl+p", "Meta+P", "ctrl+p"] },
                { "command": "layout.preset-2x2", "keys": ["ctrl+k 2"] },
                { "command": "attention.next", "keys": [] },
                { "command": "go.settings", "keys": ["ctrl+,"] }
            ] }"#,
        )
        .unwrap();
        assert_eq!(f.entries.len(), 4);
        assert_eq!(f.entries[0].keys, vec!["ctrl+p", "meta+p"]);
        assert!(f.entries[2].keys.is_empty());
        // A web window's command is kept.
        assert_eq!(f.entries[3].command, "go.settings");
        assert!(read(r#"{"version": 1.0, "bindings": []}"#).is_ok());
    }

    #[test]
    fn a_bad_file_is_refused_whole_with_every_problem() {
        assert_eq!(read("{"), Err(vec![Issue::Json]));
        assert_eq!(
            read(r#"{"version": 2, "bindings": []}"#),
            Err(vec![Issue::Newer])
        );
        assert_eq!(
            read(r#"{"version": 0, "bindings": []}"#),
            Err(vec![Issue::Shape])
        );
        assert_eq!(
            read(r#"{"version": 1.5, "bindings": []}"#),
            Err(vec![Issue::Shape])
        );
        assert_eq!(read(r#"[]"#), Err(vec![Issue::Shape]));
        let issues = read(
            r#"{ "version": 1, "extra": true, "bindings": [
                { "command": "palette.open", "keys": ["ctrl+p"], "why": 1 },
                { "command": "Bad Id", "keys": [] },
                { "command": "request.allow", "keys": ["y"] },
                { "command": "palette.open", "keys": [] },
                { "command": "window.safe-mode", "keys": [] },
                { "command": "a.b", "keys": ["a", "b", "c", "d", "e"] },
                { "command": "c.d", "keys": ["hyper+x", "ctrl+c", "ctrl+k tab"] },
                7
            ] }"#,
        )
        .unwrap_err();
        assert_eq!(
            issues,
            vec![
                Issue::Field { at: "extra".into() },
                Issue::Field {
                    at: "bindings[0].why".into()
                },
                Issue::Command {
                    command: "Bad Id".into()
                },
                Issue::Request {
                    command: "request.allow".into()
                },
                Issue::Twice {
                    command: "palette.open".into()
                },
                Issue::FixedCommand {
                    command: "window.safe-mode".into()
                },
                Issue::TooManyKeys {
                    command: "a.b".into()
                },
                Issue::Key {
                    command: "c.d".into(),
                    key: "hyper+x".into()
                },
                Issue::FixedKey {
                    command: "c.d".into(),
                    key: "ctrl+c".into(),
                    reason: FixedReason::Editing
                },
                Issue::FixedKey {
                    command: "c.d".into(),
                    key: "ctrl+k tab".into(),
                    reason: FixedReason::Navigation
                },
                Issue::Shape,
            ]
        );
    }

    #[test]
    fn problems_are_capped_and_too_many_entries_refused() {
        let many: Vec<String> = (0..60)
            .map(|i| format!(r#"{{"command": "Bad{i}", "keys": []}}"#))
            .collect();
        let issues = read(&format!(
            r#"{{"version":1,"bindings":[{}]}}"#,
            many.join(",")
        ))
        .unwrap_err();
        assert_eq!(issues.len(), MAX_ISSUES + 1);
        assert_eq!(issues.last(), Some(&Issue::More { count: 10 }));
        let entries: Vec<String> = (0..201)
            .map(|i| format!(r#"{{"command": "c.n{i}", "keys": []}}"#))
            .collect();
        assert_eq!(
            read(&format!(
                r#"{{"version":1,"bindings":[{}]}}"#,
                entries.join(",")
            )),
            Err(vec![Issue::TooMany])
        );
        let big = format!(
            r#"{{"version":1,"bindings":[],"x":"{}"}}"#,
            "a".repeat(70_000)
        );
        assert_eq!(read(&big), Err(vec![Issue::TooBig]));
    }

    #[test]
    fn text_said_back_is_plain_and_short() {
        let issues = read(&format!(
            "{{\"version\":1,\"bindings\":[{{\"command\":\"x\",\"keys\":[\"\\u202e{}\"]}}]}}",
            "k".repeat(100)
        ))
        .unwrap_err();
        let Issue::Key { key, .. } = &issues[0] else {
            panic!("{issues:?}")
        };
        assert_eq!(key.chars().count(), 64);
        assert!(!key.contains('\u{202e}'));
    }

    #[test]
    fn a_missing_file_is_the_defaults_and_a_large_one_is_not_read() {
        let dir = std::env::temp_dir().join(format!("agora-tui-keys-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(load(&dir.join("none.json")), Ok(None));
        assert_eq!(load(&dir), Err(vec![Issue::Unreadable]));
        let big = dir.join("big.json");
        std::fs::write(&big, vec![b' '; 70_000]).unwrap();
        assert_eq!(load(&big), Err(vec![Issue::TooBig]));
        let ok = dir.join(FILE);
        std::fs::write(
            &ok,
            r#"{"version":1,"bindings":[{"command":"app.quit","keys":["ctrl+q"]}]}"#,
        )
        .unwrap();
        assert_eq!(load(&ok).unwrap().unwrap().entries[0].keys, vec!["ctrl+q"]);
        let mut bom = b"\xef\xbb\xbf".to_vec();
        bom.extend_from_slice(br#"{"version":1,"bindings":[]}"#);
        std::fs::write(&ok, bom).unwrap();
        assert_eq!(load(&ok), Ok(Some(KeyFile::default())));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_problem_has_words() {
        let t = |id: &str, _: &[(&str, Arg)]| id.to_string();
        let said = issue_text(
            &Issue::FixedKey {
                command: "a".into(),
                key: "tab".into(),
                reason: FixedReason::Navigation,
            },
            &t,
        );
        assert_eq!(said, "tui-keys-error-fixed-key");
    }
}
