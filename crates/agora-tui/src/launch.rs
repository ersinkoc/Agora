//! ADR-0066: the settings a start may give (model, effort, ...), asked in the terminal client as the web window's start dialog asks
//! them (`apps/web/src/components/LaunchSettings.tsx`). Pure: what one question shows and what a key does; the caller asks with its
//! dialog, signs the chosen values with the launch (`device::Device::authorize_launch_with_settings`, `directory-action-v4`) and sends
//! them in `AgentLaunch.settings`.
//!
//! One setting at a time, in the manifest's order. A setting with values is a numbered list whose first line keeps the tool's own
//! default; a setting without values (a model name the tool checks itself) is typed. Enter keeps the default, Backspace goes back to the
//! setting before, Esc cancels the start. Nothing chosen sends no setting at all, so a start against a node that offers none (or an
//! older one, which sends none) is the start of before.

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

/// A value a start may send: 1 to [`MAX_VALUE`] characters, none of them a space or a control character (one argument, never through
/// a shell; the node checks it against its manifest again, `agent.setting_invalid`).
pub fn valid_value(v: &str) -> bool {
    let n = v.chars().count();
    (1..=MAX_VALUE).contains(&n) && !v.chars().any(|c| c.is_control() || c.is_whitespace())
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
        let specs = usable_specs(specs);
        (!specs.is_empty()).then(|| Self {
            chosen: vec![None; specs.len()],
            specs,
            at: 0,
            page: 0,
            typed: String::new(),
        })
    }

    fn spec(&self) -> &LaunchSettingSpec {
        // `at` is always below the number of settings while the question is asked (`key` returns Done at the end).
        &self.specs[self.at.min(self.specs.len() - 1)]
    }

    fn pages(&self) -> usize {
        self.spec().values.len().div_ceil(PAGE).max(1)
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
                let value = free.then(|| self.typed.trim().to_string());
                match value {
                    Some(v) if !v.is_empty() && !valid_value(&v) => Step::Ask,
                    Some(v) if !v.is_empty() => self.answer(Some(v)),
                    _ => self.answer(None),
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
                if (0x21..=0x7e).contains(&b) && self.typed.chars().count() < MAX_VALUE {
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

    /// A typed setting shows what was typed for it before (going back keeps it).
    fn prefill(&mut self) {
        if self.spec().values.is_empty() {
            self.typed = self.chosen[self.at].clone().unwrap_or_default();
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
            out.push(t(
                key!("tui-start-settings-type"),
                &[("setting", Arg::Str(name)), ("default", Arg::Str(default))],
            ));
            out.push(format!(
                "{} {}",
                t(key!("tui-chat-input"), &[]),
                plain(&self.typed)
            ));
        } else {
            out.push(t(
                key!("tui-start-settings-pick"),
                &[("setting", Arg::Str(name))],
            ));
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
                out.push(t(
                    key!("tui-start-item"),
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
        // `q` and digits are typed into a field, not keys of the list.
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
        let s = usable_specs(vec![spec("m", &["a", "a", "", "with space", "b\n"], "")]);
        assert_eq!(s[0].values, vec!["a".to_string()]);
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
}
