//! A chat agent's tool call in words, as the web window says it (`apps/web/src/net/tool.ts`, `toolView`).
//!
//! The node sends a call as `{"tool": <title>, "arguments": <the agent's raw input>}` (ADR-0038), cut at its entry limit. The title
//! and the arguments are the agent's own text: they are shown as text, never interpreted. Anything that cannot be read as a call
//! comes back as `None`, and the caller shows it as it came.

use serde_json::{Map, Value};

/// What kind of thing the agent did; `Other` when nothing says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Read,
    Edit,
    Run,
    Search,
    Fetch,
    Delete,
    Move,
    Think,
    Other,
}

/// A tool call as a person reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Call {
    pub action: Action,
    /// The agent's own title ("Read src/auth.rs", "`cargo test`"); empty when it gave none worth showing (an ACP kind).
    pub title: String,
    /// The one argument a person wants to see first: the command, the search, the address, the file. One line.
    pub subject: String,
}

/// Titles an agent sends when it has no better one: the ACP tool kinds, and the bridge's own fallback.
fn kind(title: &str) -> Option<Action> {
    Some(match title.trim().to_lowercase().as_str() {
        "read" => Action::Read,
        "edit" => Action::Edit,
        "delete" => Action::Delete,
        "move" => Action::Move,
        "search" => Action::Search,
        "execute" => Action::Run,
        "think" => Action::Think,
        "fetch" => Action::Fetch,
        "other" | "tool" => Action::Other,
        _ => return None,
    })
}

/// The arguments that say what a call is about, in the order they are looked for, and what each suggests the call does. A search
/// names the folder it looks in too, so what it looks for comes before any path.
const SUBJECTS: [(&str, Action); 12] = [
    ("command", Action::Run),
    ("cmd", Action::Run),
    ("pattern", Action::Search),
    ("query", Action::Search),
    ("url", Action::Fetch),
    ("uri", Action::Fetch),
    ("file_path", Action::Read),
    ("filePath", Action::Read),
    ("notebook_path", Action::Read),
    ("path", Action::Read),
    ("abs_path", Action::Read),
    ("description", Action::Other),
];

const EDIT_KEYS: [&str; 10] = [
    "old_string",
    "new_string",
    "oldText",
    "newText",
    "content",
    "edits",
    "patch",
    "diff",
    "new_str",
    "old_str",
];

/// A value as one would read it: text as it is, a command given as words joined (a `sh -c` wrapper dropped), anything else as JSON.
fn as_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Array(a) if a.iter().all(Value::is_string) => {
            let words: Vec<&str> = a.iter().filter_map(Value::as_str).collect();
            if words.len() == 3 && is_shell(words[0]) && is_shell_flag(words[1]) {
                return words[2].to_string();
            }
            words.join(" ")
        }
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    }
}

/// `bash`, `/bin/zsh`, `sh`, `dash`, `pwsh`, `powershell.exe`, `cmd`…
fn is_shell(word: &str) -> bool {
    let name = word.rsplit('/').next().unwrap_or(word).to_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(&name);
    matches!(
        name,
        "sh" | "bash" | "zsh" | "dash" | "pwsh" | "powershell" | "cmd"
    )
}

fn is_shell_flag(word: &str) -> bool {
    matches!(
        word.to_lowercase().as_str(),
        "-c" | "-lc" | "/c" | "-command"
    )
}

fn pick_subject(args: &Map<String, Value>) -> (String, Action) {
    for (key, hint) in SUBJECTS {
        let Some(v) = args.get(key) else { continue };
        if v.is_null() || v.as_str() == Some("") {
            continue;
        }
        let s = as_text(v);
        if s.is_empty() {
            continue;
        }
        if !s.contains('\n') {
            return (s, hint);
        }
        if key != "description" {
            return (s.lines().next().unwrap_or_default().to_string(), hint);
        }
    }
    (String::new(), Action::Other)
}

fn infer(has_edit_key: bool, hint: Action) -> Action {
    match hint {
        Action::Run | Action::Search | Action::Fetch => hint,
        _ if has_edit_key => Action::Edit,
        Action::Read => Action::Read,
        _ => Action::Other,
    }
}

/// The bridge's fallback title is the ACP kind or `tool`: a word to say, not a title.
fn titled(tool: &str) -> (String, Option<Action>) {
    match kind(tool) {
        Some(k) => (String::new(), Some(k)),
        None => (tool.trim().to_string(), None),
    }
}

/// The JSON string a key holds in text that is not quite JSON (cut short at the node's limit).
fn string_field(text: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let mut from = 0;
    while let Some(at) = text[from..].find(&needle) {
        let after = from + at + needle.len();
        let rest = text[after..].trim_start();
        if let Some(rest) = rest.strip_prefix(':') {
            let rest = rest.trim_start();
            if rest.starts_with('"') {
                // The string literal up to its unescaped closing quote.
                let mut escaped = false;
                for (i, ch) in rest.char_indices().skip(1) {
                    match ch {
                        '\\' if !escaped => escaped = true,
                        '"' if !escaped => return serde_json::from_str(&rest[..=i]).ok(),
                        _ => escaped = false,
                    }
                }
                return None;
            }
        }
        from = after;
    }
    None
}

/// The call in `text`, or `None` when it does not read as one.
pub fn call(text: &str) -> Option<Call> {
    if let Ok(Value::Object(o)) = serde_json::from_str::<Value>(text) {
        let tool = o.get("tool")?.as_str()?;
        let (title, k) = titled(tool);
        return Some(match o.get("arguments") {
            Some(Value::Object(args)) => {
                let (subject, hint) = pick_subject(args);
                let edit = args.keys().any(|key| EDIT_KEYS.contains(&key.as_str()));
                let action = match k {
                    Some(k) if k != Action::Other => k,
                    _ => infer(edit, hint),
                };
                Call {
                    action,
                    title,
                    subject,
                }
            }
            other => {
                let shown = other
                    .filter(|v| !v.is_null())
                    .map(as_text)
                    .unwrap_or_default();
                Call {
                    action: k.unwrap_or(Action::Other),
                    title,
                    subject: if shown.contains('\n') {
                        String::new()
                    } else {
                        shown
                    },
                }
            }
        });
    }
    // Cut short, or arguments that were never JSON: read what can be read.
    let tool = string_field(text, "tool")?;
    let (title, k) = titled(&tool);
    let (subject, hint) = SUBJECTS
        .iter()
        .find_map(|(key, hint)| {
            string_field(text, key)
                .filter(|s| !s.is_empty())
                .map(|s| (s.lines().next().unwrap_or_default().to_string(), *hint))
        })
        .unwrap_or((String::new(), Action::Other));
    let edit = EDIT_KEYS
        .iter()
        .any(|key| text.contains(&format!("\"{key}\"")));
    let action = match k {
        Some(k) if k != Action::Other => k,
        _ => infer(edit, hint),
    };
    Some(Call {
        action,
        title,
        subject,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn c(action: Action, title: &str, subject: &str) -> Option<Call> {
        Some(Call {
            action,
            title: title.into(),
            subject: subject.into(),
        })
    }

    #[test]
    fn a_call_is_its_action_title_and_main_argument() {
        assert_eq!(
            call(
                r#"{"tool":"Read src/auth.rs","arguments":{"file_path":"/w/src/auth.rs","limit":40}}"#
            ),
            c(Action::Read, "Read src/auth.rs", "/w/src/auth.rs")
        );
        assert_eq!(
            call(r#"{"tool":"execute","arguments":{"command":"cargo test"}}"#),
            c(Action::Run, "", "cargo test")
        );
        // A command given as words: the shell wrapper is noise.
        assert_eq!(
            call(r#"{"tool":"tool","arguments":{"command":["bash","-lc","cargo test -p agora"]}}"#),
            c(Action::Run, "", "cargo test -p agora")
        );
        // A search names its folder too: what it looks for wins.
        assert_eq!(
            call(r#"{"tool":"Grep","arguments":{"path":"src","pattern":"TODO"}}"#),
            c(Action::Search, "Grep", "TODO")
        );
        assert_eq!(
            call(r#"{"tool":"other","arguments":{"url":"https://example.com"}}"#),
            c(Action::Fetch, "", "https://example.com")
        );
        // "Edit" is an ACP kind, not a title (as in the web window); edit keys make an edit of a call that names a file.
        assert_eq!(
            call(
                r#"{"tool":"Edit","arguments":{"file_path":"a.rs","old_string":"x","new_string":"y"}}"#
            ),
            c(Action::Edit, "", "a.rs")
        );
        // A multi-line command shows its first line.
        assert_eq!(
            call(r#"{"tool":"execute","arguments":{"command":"cd x\nmake"}}"#),
            c(Action::Run, "", "cd x")
        );
    }

    #[test]
    fn arguments_that_are_not_an_object_and_text_that_is_not_a_call() {
        assert_eq!(
            call(r#"{"tool":"think","arguments":"weighing two fixes"}"#),
            c(Action::Think, "", "weighing two fixes")
        );
        assert_eq!(call(r#"{"tool":"ls"}"#), c(Action::Other, "ls", ""));
        assert_eq!(call("plain words"), None);
        assert_eq!(call(r#"{"no_tool":1}"#), None);
    }

    #[test]
    fn a_call_cut_at_the_nodes_limit_still_says_its_command() {
        assert_eq!(
            call(
                r#"{"tool":"execute","arguments":{"command":"cargo build --release","description":"Bui"#
            ),
            c(Action::Run, "", "cargo build --release")
        );
        assert_eq!(
            call(r##"{"tool":"Write","arguments":{"file_path":"x.md","content":"# Ti"##),
            c(Action::Edit, "Write", "x.md")
        );
        // An escaped quote inside the value does not end it.
        assert_eq!(
            call(r##"{"tool":"execute","arguments":{"command":"echo \"hi\"","x":"##),
            c(Action::Run, "", "echo \"hi\"")
        );
    }
}
