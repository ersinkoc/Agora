//! A small message catalog for the Fluent subset Agora's own locale files use: `{ $name }` placeholders and
//! `{ $name -> [one] … *[other] … }` selectors (English plural rule: exactly 1 is `one`). It exists so
//! command-line output can come from `locales/` today; a full Fluent runtime can replace it behind the same
//! `render` call when richer grammar is needed.
//!
//! A message that is unknown renders as `None`; callers fall back to showing the id, which is still
//! debuggable and never prose hard-coded in Rust.

use std::collections::BTreeMap;

/// A value for a placeholder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arg {
    Str(String),
    Num(i64),
}

impl Arg {
    fn text(&self) -> String {
        match self {
            Arg::Str(s) => s.clone(),
            Arg::Num(n) => n.to_string(),
        }
    }
}

impl From<&str> for Arg {
    fn from(v: &str) -> Self {
        Arg::Str(v.to_string())
    }
}

impl From<String> for Arg {
    fn from(v: String) -> Self {
        Arg::Str(v)
    }
}

impl From<i64> for Arg {
    fn from(v: i64) -> Self {
        Arg::Num(v)
    }
}

/// Messages by id.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    messages: BTreeMap<String, String>,
}

fn starts_message(line: &str) -> Option<(&str, &str)> {
    if line.starts_with(char::is_whitespace) || line.starts_with('#') || line.starts_with('-') {
        return None;
    }
    let (id, rest) = line.split_once('=')?;
    let id = id.trim_end();
    let mut chars = id.chars();
    let ok = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    ok.then_some((id, rest.trim_start()))
}

impl Catalog {
    /// Parse Fluent source and add its messages (later sources override earlier ones).
    pub fn add_source(&mut self, src: &str) {
        let mut current: Option<(String, String)> = None;
        for line in src.lines() {
            if let Some((id, rest)) = starts_message(line) {
                if let Some((id, body)) = current.take() {
                    self.messages.insert(id, body);
                }
                current = Some((id.to_string(), rest.to_string()));
            } else if line.starts_with('#') || line.trim().is_empty() {
                // Comments end a message; blank lines inside a selector are harmless to keep.
                if line.starts_with('#') {
                    if let Some((id, body)) = current.take() {
                        self.messages.insert(id, body);
                    }
                }
            } else if let Some((_, body)) = current.as_mut() {
                body.push('\n');
                body.push_str(line);
            }
        }
        if let Some((id, body)) = current {
            self.messages.insert(id, body);
        }
    }

    /// The English catalog built into the binary: `locales/en/*.ftl`.
    pub fn english() -> Self {
        let mut c = Self::default();
        c.add_source(include_str!("../../../locales/en/main.ftl"));
        c.add_source(include_str!("../../../locales/en/errors.ftl"));
        c.add_source(include_str!("../../../locales/en/cli.ftl"));
        c.add_source(include_str!("../../../locales/en/notify.ftl"));
        c
    }

    pub fn contains(&self, id: &str) -> bool {
        self.messages.contains_key(id)
    }

    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.messages.keys().map(String::as_str)
    }

    /// Render message `id`. Missing placeholders render as the empty string.
    pub fn render(&self, id: &str, args: &[(&str, Arg)]) -> Option<String> {
        let body = self.messages.get(id)?;
        Some(render_text(body, args).trim().to_string())
    }
}

fn arg<'a>(args: &'a [(&str, Arg)], name: &str) -> Option<&'a Arg> {
    args.iter().find(|(n, _)| *n == name).map(|(_, a)| a)
}

/// Find the `}` matching the `{` at `open`, honouring nesting.
fn matching_brace(s: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in s[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + i);
                }
            }
            _ => {}
        }
    }
    None
}

fn render_text(s: &str, args: &[(&str, Arg)]) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let Some(close) = matching_brace(rest, open) else {
            out.push_str(&rest[open..]);
            return out;
        };
        out.push_str(&render_expr(&rest[open + 1..close], args));
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

fn render_expr(inner: &str, args: &[(&str, Arg)]) -> String {
    let inner = inner.trim();
    match inner.split_once("->") {
        None => inner
            .strip_prefix('$')
            .and_then(|name| arg(args, name.trim()))
            .map(Arg::text)
            .unwrap_or_default(),
        Some((selector, variants)) => {
            let value = selector
                .trim()
                .strip_prefix('$')
                .and_then(|n| arg(args, n.trim()));
            select_variant(variants, value, args)
        }
    }
}

/// Variants look like `[one] text` / `*[other] text`, each starting a (possibly indented) line.
fn select_variant(variants: &str, value: Option<&Arg>, args: &[(&str, Arg)]) -> String {
    let mut parsed: Vec<(String, bool, String)> = Vec::new();
    for line in variants.lines() {
        let t = line.trim_start();
        let (default, t) = t.strip_prefix('*').map_or((false, t), |r| (true, r));
        if let Some(r) = t.strip_prefix('[') {
            if let Some((key, text)) = r.split_once(']') {
                parsed.push((key.trim().to_string(), default, text.trim().to_string()));
                continue;
            }
        }
        if let Some(last) = parsed.last_mut() {
            last.2.push('\n');
            last.2.push_str(line.trim());
        }
    }
    let wanted = value.map(|v| match v {
        Arg::Num(1) => vec!["1".to_string(), "one".to_string()],
        Arg::Num(n) => vec![n.to_string(), "other".to_string()],
        Arg::Str(s) => vec![s.clone()],
    });
    let chosen = wanted
        .iter()
        .flatten()
        .find_map(|k| parsed.iter().find(|(key, _, _)| key == k))
        .or_else(|| parsed.iter().find(|(_, d, _)| *d));
    chosen
        .map(|(_, _, text)| render_text(text, args))
        .unwrap_or_default()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn cat(src: &str) -> Catalog {
        let mut c = Catalog::default();
        c.add_source(src);
        c
    }

    #[test]
    fn plain_and_placeholder_messages() {
        let c = cat("hello = Hello\ngreet = Hi { $name }, you have { $n } items.\n");
        assert_eq!(c.render("hello", &[]).as_deref(), Some("Hello"));
        let out = c.render("greet", &[("name", "Ada".into()), ("n", 3.into())]);
        assert_eq!(out.as_deref(), Some("Hi Ada, you have 3 items."));
        assert_eq!(
            c.render("greet", &[]).as_deref(),
            Some("Hi , you have  items.")
        );
        assert_eq!(c.render("missing", &[]), None);
    }

    #[test]
    fn plural_selectors_choose_one_or_other() {
        let c = cat("n = { $count ->\n    [one] { $count } node\n   *[other] { $count } nodes\n}\nafter = x\n");
        assert_eq!(
            c.render("n", &[("count", 1.into())]).as_deref(),
            Some("1 node")
        );
        assert_eq!(
            c.render("n", &[("count", 0.into())]).as_deref(),
            Some("0 nodes")
        );
        assert_eq!(
            c.render("n", &[("count", 5.into())]).as_deref(),
            Some("5 nodes")
        );
        assert_eq!(c.render("after", &[]).as_deref(), Some("x"));
    }

    #[test]
    fn string_selectors_and_defaults() {
        let c = cat("s = { $state ->\n    [done] Finished\n   *[other] Running\n}\n");
        assert_eq!(
            c.render("s", &[("state", "done".into())]).as_deref(),
            Some("Finished")
        );
        assert_eq!(
            c.render("s", &[("state", "x".into())]).as_deref(),
            Some("Running")
        );
        assert_eq!(c.render("s", &[]).as_deref(), Some("Running"));
    }

    #[test]
    fn comments_and_blank_lines_separate_messages() {
        let c = cat("# comment\na = A\n\n# another\nb = B\n");
        assert_eq!(c.render("a", &[]).as_deref(), Some("A"));
        assert_eq!(c.render("b", &[]).as_deref(), Some("B"));
    }

    #[test]
    fn every_english_message_renders_without_leftover_syntax() {
        let c = Catalog::english();
        assert!(c.contains("app-name") && c.contains("node-resource_limit"));
        for id in c.ids() {
            let body = &c.messages[id];
            let vars: Vec<String> = body
                .split('$')
                .skip(1)
                .map(|r| {
                    r.chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                        .collect()
                })
                .collect();
            for number in [1i64, 2] {
                let args: Vec<(&str, Arg)> = vars
                    .iter()
                    .map(|v| (v.as_str(), Arg::Num(number)))
                    .collect();
                let out = c.render(id, &args).unwrap();
                assert!(
                    !out.contains(['{', '}', '$']) && !out.contains("->"),
                    "{id}: {out:?}"
                );
                assert!(!out.is_empty(), "{id} rendered empty");
            }
        }
    }
}
