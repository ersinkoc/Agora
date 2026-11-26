//! ADR-0066: the settings a start may give (model, effort, ...), asked in the terminal client as the web window's start dialog asks
//! them (`apps/web/src/components/LaunchSettings.tsx`). Pure: what one question shows and what a key does; the caller asks with its
//! dialog, signs the chosen values with the launch (`device::Device::authorize_launch_with_settings`, `directory-action-v4`) and sends
//! them in `AgentLaunch.settings`.
//!
//! One setting at a time, in the manifest's order. A setting with values is a numbered list whose first line keeps the tool's own
//! default; a setting without values (a model name the tool checks itself) is typed. Enter keeps the default, Backspace goes back to the
//! setting before, Esc cancels the start. Nothing chosen sends no setting at all, so a start against a node that offers none (or an
//! older one, which sends none) is the start of before.
//!
//! ADR-0066 decision 3: the last choice per kind per machine is remembered in [`MEMORY_FILE`] in the client's Agora folder (as the
//! web window keeps its own on its device) and preselected the next time: Enter then keeps it, `1` still picks the tool's default. Only
//! settings are kept, never how the agent acts (an option that skips permissions is asked and confirmed every time), and a remembered
//! value the machine no longer offers is dropped.

use std::path::Path;

use agora_i18n::{key, Arg};
use agora_protocol::v1::LaunchSettingSpec;

use crate::render::{plain, Text};

/// The most settings asked, and the most values of one offered (a manifest has a few of each).
pub const MAX_SETTINGS: usize = 16;
pub const MAX_VALUES: usize = 64;
/// The longest typed value, as the web window's field takes.
pub const MAX_VALUE: usize = 128;
/// Values on one page of a list: keys 2 to 9 (1 keeps the default).
pub const PAGE: usize = 8;

/// The settings a node offered, as they can be asked: each with an id, once, at most [`MAX_SETTINGS`], each with at most
/// [`MAX_VALUES`] distinct non-empty values that a start could send.
pub fn usable_specs(specs: Vec<LaunchSettingSpec>) -> Vec<LaunchSettingSpec> {
    let mut out: Vec<LaunchSettingSpec> = Vec::new();
    for mut s in specs {
        if s.id.is_empty() || out.iter().any(|o| o.id == s.id) {
            continue;
        }
        let mut values: Vec<String> = Vec::new();
        for v in s.values {
            if valid_value(&v) && !values.contains(&v) && values.len() < MAX_VALUES {
                values.push(v);
            }
        }
        s.values = values;
        out.push(s);
        if out.len() == MAX_SETTINGS {
            break;
        }
    }
    out
}

/// A character a value may hold: the node's rule (`agora-adapters` `manifest::valid_setting_value`).
fn value_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '/' | '@' | '+' | '-')
}

/// A value a start may send, exactly as the node takes one (`agora-adapters` `manifest::valid_setting_value`): a plain token of letters,
/// digits, `.`, `_`, `:`, `/`, `@`, `+` and `-`, 1 to [`MAX_VALUE`] long, starting with a letter or digit (it can never become a flag).
/// Anything else would only be refused there (`agent.setting_invalid`).
pub fn valid_value(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= MAX_VALUE
        && v.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && v.chars().all(value_char)
}

/// What the start's settings question does after a key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Ask again (draw [`Settings::lines`]).
    Ask,
    /// Every setting is answered: these `(id, value)` pairs, in the manifest's order, only those not left at the default.
    Done(Vec<(String, String)>),
    /// Esc: no start.
    Cancelled,
}

/// The start's settings question.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    specs: Vec<LaunchSettingSpec>,
    at: usize,
    page: usize,
    chosen: Vec<Option<String>>,
    typed: String,
}

const ESC: u8 = 0x1b;
const CTRL_C: u8 = 3;

impl Settings {
    /// `None` when there is nothing to ask.
    pub fn new(specs: Vec<LaunchSettingSpec>) -> Option<Self> {
        Self::with_remembered(specs, &[])
    }

    /// Like [`new`](Self::new), with what was chosen last time for this kind on this machine ([`recall`]) preselected: a listed setting
    /// takes a remembered value only when it is still in its list, a typed one only when it is still a value a start may send.
    /// Anything else is left at the tool's default.
    pub fn with_remembered(
        specs: Vec<LaunchSettingSpec>,
        remembered: &[(String, String)],
    ) -> Option<Self> {
        let specs = usable_specs(specs);
        if specs.is_empty() {
            return None;
        }
        let chosen = specs
            .iter()
            .map(|s| {
                let (_, v) = remembered.iter().find(|(id, _)| *id == s.id)?;
                let offered = if s.values.is_empty() {
                    valid_value(v)
                } else {
                    s.values.contains(v)
                };
                offered.then(|| v.clone())
            })
            .collect();
        let mut q = Self {
            chosen,
            specs,
            at: 0,
            page: 0,
            typed: String::new(),
        };
        q.prefill();
        Some(q)
    }

    fn spec(&self) -> &LaunchSettingSpec {
        // `at` is always below the number of settings while the question is asked (`key` returns Done at the end).
        &self.specs[self.at.min(self.specs.len() - 1)]
    }

    fn pages(&self) -> usize {
        self.spec().values.len().div_ceil(PAGE).max(1)
    }

    /// Whether the setting asked now is typed (it has no values): its keys are text, so arrows must not stand for letters there.
    pub fn typing(&self) -> bool {
        self.spec().values.is_empty()
    }

    /// The pairs chosen so far, in the manifest's order.
    pub fn chosen(&self) -> Vec<(String, String)> {
        self.specs
            .iter()
            .zip(&self.chosen)
            .filter_map(|(s, v)| v.as_ref().map(|v| (s.id.clone(), v.clone())))
            .collect()
    }

    /// One key, as the dialog reads it (a byte; Enter is `\r` or `\n`, Backspace 0x7f or 0x08, Esc 0x1b).
    pub fn key(&mut self, k: u8) -> Step {
        if matches!(k, ESC | CTRL_C) {
            return Step::Cancelled;
        }
        let free = self.spec().values.is_empty();
        match k {
            b'\r' | b'\n' => {
                // Exactly what is shown is what is sent (and signed): only characters a value may hold are ever typed.
                let value = free.then(|| self.typed.clone());
                match value {
                    Some(v) if !v.is_empty() && !valid_value(&v) => Step::Ask,
                    Some(v) if !v.is_empty() => self.answer(Some(v)),
                    Some(_) => self.answer(None),
                    // A list: Enter keeps what is preselected (remembered, or chosen before going back), else the default.
                    None => {
                        let kept = self.chosen[self.at].clone();
                        self.answer(kept)
                    }
                }
            }
            0x7f | 0x08 if free && !self.typed.is_empty() => {
                self.typed.pop();
                Step::Ask
            }
            0x7f | 0x08 => {
                self.back();
                Step::Ask
            }
            b if free => {
                if value_char(char::from(b)) && self.typed.len() < MAX_VALUE {
                    self.typed.push(char::from(b));
                }
                Step::Ask
            }
            b'1' => self.answer(None),
            b'2'..=b'9' => {
                let i = self.page * PAGE + usize::from(k - b'2');
                match self.spec().values.get(i).cloned() {
                    Some(v) => self.answer(Some(v)),
                    None => Step::Ask,
                }
            }
            b'n' if self.pages() > 1 => {
                self.page = (self.page + 1) % self.pages();
                Step::Ask
            }
            b'q' => Step::Cancelled,
            _ => Step::Ask,
        }
    }

    fn answer(&mut self, value: Option<String>) -> Step {
        self.chosen[self.at] = value;
        self.at += 1;
        self.page = 0;
        self.typed.clear();
        if self.at == self.specs.len() {
            self.at -= 1;
            return Step::Done(self.chosen());
        }
        self.prefill();
        Step::Ask
    }

    fn back(&mut self) {
        if self.at > 0 {
            self.at -= 1;
            self.page = 0;
            self.typed.clear();
            self.prefill();
        }
    }

    /// A typed setting shows what was typed for it before (going back keeps it, as does a remembered value); a list opens on the
    /// page of its preselected value.
    fn prefill(&mut self) {
        let current = self.chosen[self.at].clone();
        if self.spec().values.is_empty() {
            self.typed = current.unwrap_or_default();
        } else if let Some(i) =
            current.and_then(|v| self.spec().values.iter().position(|x| *x == v))
        {
            self.page = i / PAGE;
        }
    }

    /// The setting's name: its label's words, else its id.
    fn name(s: &LaunchSettingSpec, t: Text<'_>) -> String {
        if s.label.is_empty() {
            return plain(&s.id);
        }
        let said = t(&s.label, &[]);
        plain(if said == s.label { &s.id } else { &said })
    }

    /// The question as lines: the settings answered so far, then this one's list or field.
    pub fn lines(&self, t: Text<'_>) -> Vec<String> {
        let s = self.spec();
        let name = Self::name(s, t);
        let mut out = vec![t(
            key!("tui-start-settings-step"),
            &[
                ("n", Arg::Num(self.at as i64 + 1)),
                ("total", Arg::Num(self.specs.len() as i64)),
            ],
        )];
        for (done, v) in self.specs.iter().zip(&self.chosen).take(self.at) {
            let value = v.as_ref().map_or_else(
                || t(key!("tui-start-settings-default-word"), &[]),
                |v| plain(v),
            );
            out.push(t(
                key!("tui-start-setting"),
                &[
                    ("name", Arg::Str(Self::name(done, t))),
                    ("value", Arg::Str(value)),
                ],
            ));
        }
        out.push(String::new());
        let default = if s.default_value.is_empty() {
            t(key!("tui-start-settings-default-tool"), &[])
        } else {
            t(
                key!("tui-start-settings-default"),
                &[("value", Arg::Str(plain(&s.default_value)))],
            )
        };
        if s.values.is_empty() {
            // The field opens with last time's value (or the one typed before going back): Enter sends it, which is said.
            let kept = self.chosen[self.at]
                .as_deref()
                .filter(|v| *v == self.typed && !v.is_empty());
            out.push(match kept {
                Some(v) => t(
                    key!("tui-start-settings-type-last"),
                    &[
                        ("setting", Arg::Str(name)),
                        ("value", Arg::Str(plain(v))),
                        ("default", Arg::Str(default)),
                    ],
                ),
                None => t(
                    key!("tui-start-settings-type"),
                    &[("setting", Arg::Str(name)), ("default", Arg::Str(default))],
                ),
            });
            out.push(format!(
                "{} {}",
                t(key!("tui-chat-input"), &[]),
                plain(&self.typed)
            ));
            if !self.typed.is_empty() && !valid_value(&self.typed) {
                out.push(t(key!("tui-start-settings-invalid"), &[]));
            }
        } else {
            // What Enter keeps is said, and its line is marked, so what is signed is what is shown.
            let kept = self.chosen[self.at].as_deref();
            out.push(match kept {
                Some(v) => t(
                    key!("tui-start-settings-pick-last"),
                    &[("setting", Arg::Str(name)), ("value", Arg::Str(plain(v)))],
                ),
                None => t(
                    key!("tui-start-settings-pick"),
                    &[("setting", Arg::Str(name))],
                ),
            });
            out.push(String::new());
            out.push(t(
                key!("tui-start-item"),
                &[("n", Arg::Num(1)), ("name", Arg::Str(default))],
            ));
            for (i, v) in s
                .values
                .iter()
                .skip(self.page * PAGE)
                .take(PAGE)
                .enumerate()
            {
                let item = if kept == Some(v.as_str()) {
                    key!("tui-start-item-last")
                } else {
                    key!("tui-start-item")
                };
                out.push(t(
                    item,
                    &[("n", Arg::Num(i as i64 + 2)), ("name", Arg::Str(plain(v)))],
                ));
            }
            if self.pages() > 1 {
                out.push(t(
                    key!("tui-start-settings-more"),
                    &[
                        ("page", Arg::Num(self.page as i64 + 1)),
                        ("pages", Arg::Num(self.pages() as i64)),
                    ],
                ));
            }
        }
        if self.at > 0 {
            out.push(t(key!("tui-start-settings-back"), &[]));
        }
        out
    }
}

/// The file in the client's Agora folder (`$AGORA_HOME`, else `~/.agora`, beside `tui-theme`) that keeps the last start choice per
/// kind per machine (ADR-0066 decision 3). One line per machine and kind: `machine TAB kind TAB id=value TAB ...`, newest last.
pub const MEMORY_FILE: &str = "tui-start-settings";
/// The most bytes read from [`MEMORY_FILE`] (asked of the file system first): a larger file is ignored and rewritten.
pub const MAX_MEMORY_BYTES: u64 = 64 * 1024;
/// The most machine and kind pairs kept; the oldest go first.
pub const MAX_REMEMBERED: usize = 256;

/// A machine or kind name the file can hold: one field, no tab, newline or other control character.
fn memory_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)
}

/// A setting id the file keeps: the ids a start signs (`[a-z0-9-]`, at most 32).
fn memory_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 32
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

type Remembered = (String, String, Vec<(String, String)>);

/// The file's lines that hold a choice; anything else is skipped.
fn parse_memory(text: &str) -> Vec<Remembered> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let node = fields.next().filter(|n| memory_name(n))?;
            let kind = fields.next().filter(|k| memory_name(k))?;
            let pairs: Vec<(String, String)> = fields
                .filter_map(|f| f.split_once('='))
                .filter(|(id, v)| memory_id(id) && valid_value(v))
                .map(|(id, v)| (id.to_string(), v.to_string()))
                .take(MAX_SETTINGS)
                .collect();
            (!pairs.is_empty()).then(|| (node.to_string(), kind.to_string(), pairs))
        })
        .collect()
}

fn read_memory(file: &Path) -> Vec<Remembered> {
    match std::fs::metadata(file) {
        Ok(m) if m.len() <= MAX_MEMORY_BYTES => std::fs::read_to_string(file)
            .map(|t| parse_memory(&t))
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// What was chosen last time for `kind` on `node`, as kept in `file`; empty when nothing was (or the file is missing, too large or
/// unreadable). Still to be checked against what the machine offers now ([`Settings::with_remembered`]).
pub fn recall(file: &Path, node: &str, kind: &str) -> Vec<(String, String)> {
    read_memory(file)
        .into_iter()
        .rev()
        .find(|(n, k, _)| n == node && k == kind)
        .map(|(_, _, pairs)| pairs)
        .unwrap_or_default()
}

/// Keeps `chosen` (the settings a start just sent) as the last choice for `kind` on `node`; nothing chosen forgets it. Only settings
/// are kept: how the agent acts is never passed here.
pub fn remember(
    file: &Path,
    node: &str,
    kind: &str,
    chosen: &[(String, String)],
) -> std::io::Result<()> {
    if !memory_name(node) || !memory_name(kind) {
        return Ok(());
    }
    let mut all: Vec<Remembered> = read_memory(file)
        .into_iter()
        .filter(|(n, k, _)| !(n == node && k == kind))
        .collect();
    let pairs: Vec<(String, String)> = chosen
        .iter()
        .filter(|(id, v)| memory_id(id) && valid_value(v))
        .take(MAX_SETTINGS)
        .cloned()
        .collect();
    if !pairs.is_empty() {
        all.push((node.to_string(), kind.to_string(), pairs));
    }
    let skip = all.len().saturating_sub(MAX_REMEMBERED);
    let mut text = String::new();
    for (n, k, pairs) in all.into_iter().skip(skip) {
        text.push_str(&n);
        text.push('\t');
        text.push_str(&k);
        for (id, v) in pairs {
            text.push('\t');
            text.push_str(&id);
            text.push('=');
            text.push_str(&v);
        }
        text.push('\n');
    }
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(file, text)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn t(id: &str, args: &[(&str, Arg)]) -> String {
        let a: Vec<String> = args
            .iter()
            .map(|(k, v)| match v {
                Arg::Str(s) => format!("{k}={s}"),
                Arg::Num(n) => format!("{k}={n}"),
            })
            .collect();
        if a.is_empty() {
            id.to_string()
        } else {
            format!("{id}[{}]", a.join(","))
        }
    }

    fn spec(id: &str, values: &[&str], default: &str) -> LaunchSettingSpec {
        LaunchSettingSpec {
            id: id.into(),
            label: format!("setting-{id}"),
            values: values.iter().map(|v| v.to_string()).collect(),
            default_value: default.into(),
        }
    }

    fn keys(s: &mut Settings, ks: &[u8]) -> Step {
        let mut last = Step::Ask;
        for k in ks {
            last = s.key(*k);
        }
        last
    }

    #[test]
    fn nothing_offered_asks_nothing() {
        assert!(Settings::new(Vec::new()).is_none());
        assert!(Settings::new(vec![spec("", &["a"], "")]).is_none());
    }

    #[test]
    fn a_list_is_picked_by_number_and_enter_keeps_the_default() {
        let mut s = Settings::new(vec![
            spec("model", &["opus", "sonnet"], "sonnet"),
            spec("effort", &["low", "medium", "high"], ""),
        ])
        .unwrap();
        assert_eq!(s.key(b'2'), Step::Ask);
        assert_eq!(
            s.key(b'\r'),
            Step::Done(vec![("model".into(), "opus".into())])
        );
        let mut s = Settings::new(vec![
            spec("model", &["opus", "sonnet"], "sonnet"),
            spec("effort", &["low", "medium", "high"], ""),
        ])
        .unwrap();
        assert_eq!(
            keys(&mut s, b"14"),
            Step::Done(vec![("effort".into(), "high".into())])
        );
    }

    #[test]
    fn a_number_past_the_list_does_nothing_and_esc_cancels() {
        let mut s = Settings::new(vec![spec("effort", &["low"], "")]).unwrap();
        assert_eq!(s.key(b'3'), Step::Ask);
        assert_eq!(s.key(b'x'), Step::Ask);
        assert_eq!(s.key(0x1b), Step::Cancelled);
        let mut s = Settings::new(vec![spec("effort", &["low"], "")]).unwrap();
        assert_eq!(s.key(b'q'), Step::Cancelled);
    }

    #[test]
    fn a_free_setting_is_typed_and_backspace_goes_back() {
        let mut s = Settings::new(vec![
            spec("effort", &["low", "high"], ""),
            spec("model", &[], "gpt-5"),
        ])
        .unwrap();
        assert_eq!(s.key(b'3'), Step::Ask);
        // `q` and digits are typed into a field, not keys of the list; a space is not a character a value may hold.
        assert_eq!(keys(&mut s, b"o3 q"), Step::Ask);
        assert!(s.lines(&t).iter().any(|l| l.ends_with(" o3q")));
        assert_eq!(keys(&mut s, &[0x7f, 0x7f, 0x7f]), Step::Ask);
        assert!(s
            .lines(&t)
            .iter()
            .any(|l| l.contains("tui-start-settings-type")));
        // Empty: Backspace goes back to the setting before.
        assert_eq!(s.key(0x7f), Step::Ask);
        assert!(s
            .lines(&t)
            .iter()
            .any(|l| l.contains("tui-start-settings-pick")));
        assert_eq!(
            keys(&mut s, b"2o4\r"),
            Step::Done(vec![
                ("effort".into(), "low".into()),
                ("model".into(), "o4".into())
            ])
        );
    }

    #[test]
    fn an_empty_field_keeps_the_default() {
        let mut s = Settings::new(vec![spec("model", &[], "")]).unwrap();
        assert_eq!(s.key(b'\n'), Step::Done(Vec::new()));
    }

    #[test]
    fn a_long_list_pages_with_n() {
        let values: Vec<String> = (0..10).map(|i| format!("v{i}")).collect();
        let refs: Vec<&str> = values.iter().map(String::as_str).collect();
        let mut s = Settings::new(vec![spec("model", &refs, "")]).unwrap();
        let first = s.lines(&t);
        assert!(first.iter().any(|l| l.contains("name=v7")));
        assert!(!first.iter().any(|l| l.contains("name=v8")));
        assert!(first.iter().any(|l| l.contains("page=1,pages=2")));
        assert_eq!(s.key(b'n'), Step::Ask);
        assert_eq!(s.key(b'3'), Step::Done(vec![("model".into(), "v9".into())]));
    }

    #[test]
    fn what_the_node_offers_is_bounded_and_cleaned() {
        let mut many: Vec<LaunchSettingSpec> =
            (0..40).map(|i| spec(&format!("s{i}"), &[], "")).collect();
        many.insert(1, spec("s0", &[], ""));
        assert_eq!(usable_specs(many).len(), MAX_SETTINGS);
        let s = usable_specs(vec![spec(
            "m",
            &[
                "a",
                "a",
                "",
                "with space",
                "b\n",
                "-flag",
                "x=y",
                "gpt-5.1:fast",
            ],
            "",
        )]);
        assert_eq!(
            s[0].values,
            vec!["a".to_string(), "gpt-5.1:fast".to_string()]
        );
        assert!(!valid_value(&"x".repeat(MAX_VALUE + 1)));
        assert!(valid_value(&"x".repeat(MAX_VALUE)));
    }

    #[test]
    fn the_question_names_the_setting_and_what_was_chosen() {
        let mut s = Settings::new(vec![
            spec("model", &["opus"], "sonnet"),
            spec("effort", &[], ""),
        ])
        .unwrap();
        let first = s.lines(&t);
        assert_eq!(first[0], "tui-start-settings-step[n=1,total=2]");
        // The label is a message id; a missing one (the test's lookup echoes it) shows the id.
        assert!(first.contains(&"tui-start-settings-pick[setting=model]".to_string()));
        assert!(first.contains(
            &"tui-start-item[n=1,name=tui-start-settings-default[value=sonnet]]".to_string()
        ));
        s.key(b'2');
        let second = s.lines(&t);
        assert!(second.contains(&"tui-start-setting[name=model,value=opus]".to_string()));
        assert!(second.contains(
            &"tui-start-settings-type[setting=effort,default=tui-start-settings-default-tool]"
                .to_string()
        ));
        assert!(second.contains(&"tui-start-settings-back".to_string()));
    }

    /// Review of #1242: a typed value is held to the node's rule, so what is shown is what is signed and sent; one the node would
    /// refuse is not sent, and the question says why.
    #[test]
    fn a_typed_value_follows_the_nodes_rule() {
        for (v, ok) in [
            ("o4-mini", true),
            ("anthropic/claude:3@x+y_z.1", true),
            ("-rf", false),
            (".hidden", false),
            ("a b", false),
            ("a=b", false),
            ("é", false),
            ("", false),
        ] {
            assert_eq!(valid_value(v), ok, "{v}");
        }
        let mut s = Settings::new(vec![spec("model", &[], "")]).unwrap();
        assert!(s.typing());
        assert_eq!(keys(&mut s, b"-x=\"y"), Step::Ask);
        assert!(s
            .lines(&t)
            .iter()
            .any(|l| l.contains("tui-start-settings-invalid")));
        // Enter keeps the question while the value cannot be sent.
        assert_eq!(s.key(b'\r'), Step::Ask);
        assert_eq!(keys(&mut s, &[0x7f, 0x7f, 0x7f]), Step::Ask);
        assert_eq!(
            keys(&mut s, b"o4\r"),
            Step::Done(vec![("model".into(), "o4".into())])
        );
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("agora-tui-launch-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d.join(MEMORY_FILE)
    }

    fn pairs(p: &[(&str, &str)]) -> Vec<(String, String)> {
        p.iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    /// ADR-0066 decision 3: the last choice is preselected; Enter keeps it, `1` picks the tool's default, and the line Enter keeps is
    /// said and marked.
    #[test]
    fn a_remembered_choice_is_preselected_and_enter_keeps_it() {
        let specs = || {
            vec![
                spec("effort", &["low", "medium", "high"], "medium"),
                spec("model", &[], ""),
            ]
        };
        let last = pairs(&[("effort", "high"), ("model", "o4")]);
        let mut s = Settings::with_remembered(specs(), &last).unwrap();
        let lines = s.lines(&t);
        assert!(
            lines.contains(&"tui-start-settings-pick-last[setting=effort,value=high]".to_string())
        );
        assert!(lines.contains(&"tui-start-item-last[n=4,name=high]".to_string()));
        assert!(lines.contains(&"tui-start-item[n=3,name=medium]".to_string()));
        assert_eq!(s.key(b'\r'), Step::Ask);
        // The typed setting opens with last time's value in its field, and says Enter sends it.
        let typed = s.lines(&t);
        assert!(typed.iter().any(|l| l.ends_with(" o4")));
        assert!(typed.contains(
            &"tui-start-settings-type-last[setting=model,value=o4,default=tui-start-settings-default-tool]"
                .to_string()
        ));
        assert_eq!(s.key(b'\r'), Step::Done(last.clone()));
        // `1` is the tool's default; clearing the field too.
        let mut s = Settings::with_remembered(specs(), &last).unwrap();
        assert_eq!(
            keys(&mut s, &[b'1', 0x7f, 0x7f, b'\r']),
            Step::Done(Vec::new())
        );
    }

    /// A remembered value the kind no longer offers (or a setting it no longer has, or a value a start may not send) is not preselected.
    #[test]
    fn a_value_no_longer_offered_is_ignored() {
        let last = pairs(&[("effort", "max"), ("model", "-rf"), ("gone", "x")]);
        let mut s = Settings::with_remembered(
            vec![spec("effort", &["low", "high"], ""), spec("model", &[], "")],
            &last,
        )
        .unwrap();
        assert!(s
            .lines(&t)
            .contains(&"tui-start-settings-pick[setting=effort]".to_string()));
        assert_eq!(keys(&mut s, b"\r\r"), Step::Done(Vec::new()));
    }

    /// A preselected value on a later page opens that page.
    #[test]
    fn a_remembered_value_opens_its_page() {
        let values: Vec<String> = (0..10).map(|i| format!("v{i}")).collect();
        let refs: Vec<&str> = values.iter().map(String::as_str).collect();
        let s =
            Settings::with_remembered(vec![spec("model", &refs, "")], &pairs(&[("model", "v9")]))
                .unwrap();
        let lines = s.lines(&t);
        assert!(
            lines.iter().any(|l| l.contains("page=2,pages=2")),
            "{lines:?}"
        );
        assert!(lines.contains(&"tui-start-item-last[n=3,name=v9]".to_string()));
    }

    #[test]
    fn the_memory_file_keeps_the_last_choice_per_machine_and_kind() {
        let f = scratch("memory");
        assert!(recall(&f, "laptop", "codex").is_empty());
        remember(
            &f,
            "laptop",
            "codex",
            &pairs(&[("model", "o4"), ("effort", "high")]),
        )
        .unwrap();
        remember(&f, "vps", "codex", &pairs(&[("effort", "low")])).unwrap();
        remember(&f, "laptop", "claude", &pairs(&[("effort", "max")])).unwrap();
        assert_eq!(
            recall(&f, "laptop", "codex"),
            pairs(&[("model", "o4"), ("effort", "high")])
        );
        assert_eq!(recall(&f, "vps", "codex"), pairs(&[("effort", "low")]));
        assert!(recall(&f, "vps", "claude").is_empty());
        // A new choice replaces the old; starting with the defaults forgets it.
        remember(&f, "laptop", "codex", &pairs(&[("effort", "low")])).unwrap();
        assert_eq!(recall(&f, "laptop", "codex"), pairs(&[("effort", "low")]));
        remember(&f, "laptop", "codex", &[]).unwrap();
        assert!(recall(&f, "laptop", "codex").is_empty());
        assert_eq!(recall(&f, "laptop", "claude"), pairs(&[("effort", "max")]));
        // Nothing that is not a plain value or a setting id is kept, and a name with a tab or newline is not kept at all.
        remember(
            &f,
            "vps",
            "codex",
            &pairs(&[("effort", "a\tb"), ("Model", "x"), ("model", "x=y")]),
        )
        .unwrap();
        assert!(recall(&f, "vps", "codex").is_empty());
        remember(&f, "bad\nnode", "codex", &pairs(&[("effort", "low")])).unwrap();
        assert!(recall(&f, "bad\nnode", "codex").is_empty());
        let _ = std::fs::remove_dir_all(f.parent().unwrap());
    }

    #[test]
    fn the_memory_file_is_bounded() {
        let f = scratch("bounded");
        for i in 0..MAX_REMEMBERED + 5 {
            remember(&f, &format!("n{i}"), "codex", &pairs(&[("effort", "low")])).unwrap();
        }
        assert!(recall(&f, "n0", "codex").is_empty(), "the oldest go first");
        assert_eq!(
            recall(&f, &format!("n{}", MAX_REMEMBERED + 4), "codex"),
            pairs(&[("effort", "low")])
        );
        // A file over the cap is not read (and the next choice rewrites it); a corrupt one gives nothing.
        std::fs::write(&f, "x".repeat(MAX_MEMORY_BYTES as usize + 1)).unwrap();
        assert!(recall(&f, "n9", "codex").is_empty());
        remember(&f, "n1", "codex", &pairs(&[("effort", "high")])).unwrap();
        assert_eq!(recall(&f, "n1", "codex"), pairs(&[("effort", "high")]));
        std::fs::write(&f, "laptop\ncodex\t\t=\n\u{0}\n").unwrap();
        assert!(recall(&f, "laptop", "codex").is_empty());
        let _ = std::fs::remove_dir_all(f.parent().unwrap());
    }
}
