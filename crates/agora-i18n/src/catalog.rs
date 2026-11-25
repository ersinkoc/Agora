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

/// Locales with a translation in this build, besides the English source.
pub const SUPPORTED_LOCALES: &[&str] = &["en", "tr"];

/// `tr_TR.UTF-8` / `tr-TR` / `TR` -> `tr`.
pub fn primary_language(tag: &str) -> String {
    tag.split(['_', '-', '.', '@'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// The user's locale from the environment: `AGORA_LANG` first, then the usual `LC_ALL`, `LC_MESSAGES`, `LANG`.
/// Empty and `C`/`POSIX` count as unset.
pub fn locale_from_env() -> String {
    ["AGORA_LANG", "LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.is_empty() && v != "C" && v != "POSIX")
        .unwrap_or_else(|| "en".to_string())
}

impl Catalog {
    /// Stores one message. Fluent trims a message's value when it is parsed, before anything is
    /// interpolated; trimming the rendered text instead would eat the whitespace a string literal asks for
    /// (`{" "}· removed`).
    fn store(&mut self, id: String, body: String) {
        self.messages.insert(id, body.trim().to_string());
    }

    /// Parse Fluent source and add its messages (later sources override earlier ones).
    pub fn add_source(&mut self, src: &str) {
        let mut current: Option<(String, String)> = None;
        for line in src.lines() {
            if let Some((id, rest)) = starts_message(line) {
                if let Some((id, body)) = current.take() {
                    self.store(id, body);
                }
                current = Some((id.to_string(), rest.to_string()));
            } else if line.starts_with('#') || line.trim().is_empty() {
                // Comments end a message; blank lines inside a selector are harmless to keep.
                if line.starts_with('#') {
                    if let Some((id, body)) = current.take() {
                        self.store(id, body);
                    }
                }
            } else if let Some((_, body)) = current.as_mut() {
                body.push('\n');
                body.push_str(line);
            }
        }
        if let Some((id, body)) = current {
            self.store(id, body);
        }
    }

    /// The English catalog built into the binary: `locales/en/*.ftl`.
    pub fn english() -> Self {
        let mut c = Self::default();
        c.add_source(include_str!("../../../locales/en/main.ftl"));
        c.add_source(include_str!("../../../locales/en/errors.ftl"));
        c.add_source(include_str!("../../../locales/en/cli.ftl"));
        c.add_source(include_str!("../../../locales/en/notify.ftl"));
        c.add_source(include_str!("../../../locales/en/tui.ftl"));
        c
    }

    /// The Turkish translation on top of English: a message missing from `locales/tr` falls back to English, key by
    /// key, so a partly translated locale never shows a bare key.
    pub fn turkish() -> Self {
        let mut c = Self::english();
        c.add_source(include_str!("../../../locales/tr/main.ftl"));
        c.add_source(include_str!("../../../locales/tr/errors.ftl"));
        c.add_source(include_str!("../../../locales/tr/cli.ftl"));
        c.add_source(include_str!("../../../locales/tr/notify.ftl"));
        c.add_source(include_str!("../../../locales/tr/tui.ftl"));
        c
    }

    /// The catalog for a locale tag such as `tr`, `tr_TR.UTF-8` or `en-US`; anything unsupported gets English.
    pub fn for_locale(tag: &str) -> Self {
        match primary_language(tag).as_str() {
            "tr" => Self::turkish(),
            _ => Self::english(),
        }
    }

    pub fn contains(&self, id: &str) -> bool {
        self.messages.contains_key(id)
    }

    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.messages.keys().map(String::as_str)
    }

    /// Render message `id`. Missing placeholders render as the empty string. A message's value is trimmed where it
    /// is parsed (Fluent's rule, in [`add_source`](Self::add_source)), never after: a string literal at either end
    /// asks for the whitespace it holds (`{" "}· removed`), so the rendered text keeps it.
    pub fn render(&self, id: &str, args: &[(&str, Arg)]) -> Option<String> {
        let body = self.messages.get(id)?;
        Some(render_text(body, args))
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
    // A string literal before the selector check: its text may itself hold `->`.
    if let Some(literal) = string_literal(inner) {
        return literal;
    }
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

/// A Fluent string literal, quotes included (`{" "}`, `{ "" }`): its text with the spec's escapes (`\\`, `\"`,
/// `\uXXXX`) undone. `None` when `inner` is something else. An escape the spec does not define keeps its
/// backslash, so an unknown one shows up instead of changing the text unseen.
fn string_literal(inner: &str) -> Option<String> {
    let text = inner.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.peek().copied() {
            Some('\\') | Some('"') => {
                out.extend(chars.next());
            }
            Some('u') => {
                chars.next();
                let hex: String = (0..4).filter_map(|_| chars.next()).collect();
                match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    Some(c) if hex.len() == 4 => out.push(c),
                    _ => {
                        out.push_str("\\u");
                        out.push_str(&hex);
                    }
                }
            }
            _ => out.push('\\'),
        }
    }
    Some(out)
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
    fn string_literals_keep_the_whitespace_they_hold() {
        // A Fluent string literal is text, not a placeholder: `{" "}` is one space (locales/en/cli.ftl:198).
        let c = cat("a = {\" \"}· removed\nb = {\"\"}\nc = x{ \"  \" }y\n");
        assert_eq!(c.render("a", &[]).as_deref(), Some(" · removed"));
        assert_eq!(c.render("b", &[]).as_deref(), Some(""));
        assert_eq!(c.render("c", &[]).as_deref(), Some("x  y"));
    }

    #[test]
    fn a_string_literal_keeps_the_space_at_an_end_of_a_line() {
        // The real messages: the state is the last thing on the line, and the space before the bullet is
        // what keeps it from running into the timestamp (`agora hub devices`, `agora hub metrics-token list`).
        let en = Catalog::english();
        assert_eq!(
            en.render("cli-device-state-revoked", &[]).as_deref(),
            Some(" · removed")
        );
        assert_eq!(
            en.render("cli-metrics-token-state-revoked", &[]).as_deref(),
            Some(" · revoked")
        );
        let state = en
            .render("cli-device-state-revoked", &[])
            .unwrap_or_default();
        let args: Vec<(&str, Arg)> = vec![
            ("name", "laptop".into()),
            ("fingerprint", "SHA256:abc".into()),
            ("id", "d1".into()),
            ("user", "u1".into()),
            ("created", "2026-01-01 00:00".into()),
            ("used", "2026-11-24 10:00".into()),
            ("state", state.into()),
        ];
        assert_eq!(
            en.render("cli-hub-device-line", &args).as_deref(),
            Some("laptop · SHA256:abc · d1 · u1 · registered 2026-01-01 00:00 · last used 2026-11-24 10:00 · removed")
        );
        // The Turkish lines use the same literals.
        assert_eq!(
            Catalog::turkish()
                .render("cli-device-state-revoked", &[])
                .as_deref(),
            Some(" · kaldırıldı")
        );
    }

    #[test]
    fn a_string_literal_escape_is_undone_and_an_unknown_one_is_not_dropped() {
        let c = cat(
            "q = {\"a\\\"b\"}\nbs = {\"a\\\\b\"}\nu = {\"\\u2192\"}\nbad = {\"\\uZZZZ\"}\nmixed = {\"x\"}y\n",
        );
        assert_eq!(c.render("q", &[]).as_deref(), Some("a\"b"));
        assert_eq!(c.render("bs", &[]).as_deref(), Some("a\\b"));
        assert_eq!(c.render("u", &[]).as_deref(), Some("→"));
        // `\t` is not a Fluent escape: it stays visible as written instead of becoming an unseen tab.
        assert_eq!(c.render("bad", &[]).as_deref(), Some("\\uZZZZ"));
        // A literal sits inside a message's text: only the quoted part is literal.
        assert_eq!(c.render("mixed", &[]).as_deref(), Some("xy"));
    }

    #[test]
    fn multiline_values_still_start_and_end_at_their_text() {
        // Fluent trims a message value where it is parsed. Moving that trim must not leave the indentation of
        // the first line or a trailing newline in what a caller prints (`agora --help` prints `cli-usage`).
        let en = Catalog::english();
        let usage = en.render("cli-usage", &[]).unwrap_or_default();
        assert!(usage.starts_with("Usage:"), "{usage:?}");
        assert_eq!(usage, usage.trim_end());
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

    #[test]
    fn turkish_overlays_english_and_unknown_locales_fall_back() {
        let tr = Catalog::for_locale("tr_TR.UTF-8");
        assert_eq!(
            tr.render("cli-agent-stopped", &[("agent_id", "a1".into())])
                .as_deref(),
            Some("a1 ajanı durduruldu")
        );
        assert_eq!(
            Catalog::for_locale("TR")
                .render("agent-state-done", &[])
                .as_deref(),
            Some("Bitti")
        );
        let en = Catalog::for_locale("de-DE");
        assert_eq!(en.render("agent-state-done", &[]).as_deref(), Some("Done"));
        assert_eq!(primary_language("en-US"), "en");
        assert_eq!(primary_language("tr_TR.UTF-8@euro"), "tr");
    }

    #[test]
    fn every_turkish_message_renders_cleanly_and_keeps_its_placeholders() {
        let tr = Catalog::turkish();
        let en = Catalog::english();
        for id in en.ids() {
            let args: Vec<(&str, Arg)> = [
                "count", "detail", "agent", "machine", "name", "task", "percent", "version",
                "socket", "agent_id", "worktree", "code", "message", "status", "text", "node_id",
                "path", "limit", "kind",
            ]
            .iter()
            .map(|k| (*k, Arg::Str("X".into())))
            .collect();
            let t = tr
                .render(id, &args)
                .unwrap_or_else(|| panic!("{id} missing"));
            assert!(
                !t.contains("{ $") && !t.contains("->") && !t.contains("*["),
                "{id}: {t}"
            );
        }
        // Plural categories work in Turkish too.
        assert_eq!(
            tr.render("nodes-connected", &[("count", 1.into())])
                .as_deref(),
            Some("1 makine bağlı")
        );
    }
}
