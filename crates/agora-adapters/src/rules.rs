//! Screen rules: evaluate a manifest's rules against the live bottom of a terminal.
//!
//! Detection reads only [`Terminal::title`], [`Terminal::bottom_lines`] and
//! [`Terminal::bottom_non_empty_lines`], which never return scrollback (AGENTS.md).

use regex::{Regex, RegexBuilder};

use agora_core::AgentState;
use agora_vt::Terminal;

use crate::manifest::{Condition, Manifest, Rule, RuleState};

/// Which text a rule looks at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Region {
    Title,
    BottomLines(usize),
    BottomNonEmptyLines(usize),
}

impl Region {
    pub(crate) fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if s == "title" {
            return Some(Self::Title);
        }
        let n = |inner: &str| {
            inner
                .trim()
                .parse::<usize>()
                .ok()
                .filter(|n| (1..=200).contains(n))
        };
        if let Some(arg) = s
            .strip_prefix("bottom_lines(")
            .and_then(|r| r.strip_suffix(')'))
        {
            return n(arg).map(Self::BottomLines);
        }
        if let Some(arg) = s
            .strip_prefix("bottom_non_empty_lines(")
            .and_then(|r| r.strip_suffix(')'))
        {
            return n(arg).map(Self::BottomNonEmptyLines);
        }
        None
    }

    fn text(self, term: &dyn Terminal) -> String {
        match self {
            Self::Title => term.title().unwrap_or_default(),
            Self::BottomLines(n) => term.bottom_lines(n).join("\n"),
            Self::BottomNonEmptyLines(n) => term.bottom_non_empty_lines(n).join("\n"),
        }
    }
}

/// Patterns match per line (`^`/`$` are line anchors); size-limited so a manifest cannot make the node
/// allocate without bound.
pub(crate) fn compile(pattern: &str) -> Result<Regex, regex::Error> {
    RegexBuilder::new(pattern)
        .multi_line(true)
        .size_limit(1 << 20)
        .build()
}

/// The text of a region, and (made only when a rule needs it) the same text with every run of whitespace, line breaks included, as
/// one space. A phrase a terminal user interface wraps at a word ("Yes, I trust this" / "folder" on a narrow screen) is then still
/// found: a wrong Idle is the dangerous direction, an agent waiting for the user would look free.
struct RegionText {
    raw: String,
    collapsed: std::cell::OnceCell<String>,
}

impl RegionText {
    fn new(raw: String) -> Self {
        Self {
            raw,
            collapsed: std::cell::OnceCell::new(),
        }
    }

    fn collapsed(&self) -> &str {
        self.collapsed
            .get_or_init(|| self.raw.split_whitespace().collect::<Vec<_>>().join(" "))
    }
}

enum Matcher {
    /// The phrase, and the same phrase with its whitespace collapsed when it holds more than one word.
    Contains(String, Option<String>),
    Regex(Regex),
}

impl Matcher {
    fn from(c: &Condition) -> Option<Self> {
        Self::of(c.contains.as_deref(), c.regex.as_deref())
    }

    fn of(contains: Option<&str>, regex: Option<&str>) -> Option<Self> {
        match (contains, regex) {
            (Some(s), None) => {
                let words: Vec<&str> = s.split_whitespace().collect();
                let collapsed = (words.len() > 1).then(|| words.join(" "));
                Some(Self::Contains(s.to_string(), collapsed))
            }
            (None, Some(r)) => compile(r).ok().map(Self::Regex),
            _ => None,
        }
    }

    fn is_match(&self, text: &RegionText) -> bool {
        match self {
            Self::Contains(s, collapsed) => {
                text.raw.contains(s.as_str())
                    || collapsed
                        .as_deref()
                        .is_some_and(|c| text.collapsed().contains(c))
            }
            Self::Regex(r) => r.is_match(&text.raw),
        }
    }
}

struct CompiledRule {
    id: String,
    state: AgentState,
    priority: i32,
    region: Region,
    own: Option<Matcher>,
    all: Vec<Matcher>,
    any: Vec<Matcher>,
    not: Vec<Matcher>,
}

impl CompiledRule {
    fn matches(&self, text: &RegionText) -> bool {
        self.own.as_ref().is_none_or(|m| m.is_match(text))
            && self.all.iter().all(|m| m.is_match(text))
            && (self.any.is_empty() || self.any.iter().any(|m| m.is_match(text)))
            && !self.not.iter().any(|m| m.is_match(text))
    }
}

fn state_of(s: RuleState) -> AgentState {
    match s {
        RuleState::Working => AgentState::Working,
        RuleState::Blocked => AgentState::Blocked,
        RuleState::Idle => AgentState::Idle,
        RuleState::Done => AgentState::Done,
    }
}

/// A state found on screen and the rule that proved it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detection {
    pub state: AgentState,
    pub rule_id: String,
}

/// Compiled rules of one manifest. Build once per agent, evaluate on every detection tick.
pub struct Detector {
    rules: Vec<CompiledRule>,
}

impl Detector {
    /// Compile a validated manifest. Rules are ordered by descending priority (stable: file order breaks ties).
    pub fn new(manifest: &Manifest) -> Self {
        let mut rules: Vec<CompiledRule> = manifest.rules.iter().filter_map(compile_rule).collect();
        rules.sort_by_key(|r| std::cmp::Reverse(r.priority));
        Self { rules }
    }

    /// ADR-0057: the manifest's `[[ready]]` rules (the input box is empty and waiting), compiled the same way. Matches nothing when the
    /// manifest has none.
    pub fn ready(manifest: &Manifest) -> Self {
        Self::from_rules(&manifest.ready)
    }

    /// ADR-0057: the manifest's `[[typed]]` rules (typed text in the input box, no dialog or menu).
    pub fn typed(manifest: &Manifest) -> Self {
        Self::from_rules(&manifest.typed)
    }

    fn from_rules(list: &[crate::manifest::Rule]) -> Self {
        let mut rules: Vec<CompiledRule> = list.iter().filter_map(compile_rule).collect();
        rules.sort_by_key(|r| std::cmp::Reverse(r.priority));
        Self { rules }
    }

    /// ADR-0057: the text a manifest's `input_box` regex captures on the screen's bottom (its last match), or `None`.
    pub fn input_box(pattern: &str, term: &dyn Terminal) -> Option<String> {
        let re = compile(pattern).ok()?;
        let text = term.bottom_non_empty_lines(12).join("\n");
        re.captures_iter(&text)
            .last()
            .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
    }

    /// The state the highest-priority matching rule reports, or `None` when no rule matches (the caller keeps
    /// the previous state or falls back to `Unknown`). Region text is built once per distinct region.
    pub fn detect(&self, term: &dyn Terminal) -> Option<Detection> {
        let mut cache: Vec<(Region, RegionText)> = Vec::new();
        for rule in &self.rules {
            let idx = match cache.iter().position(|(r, _)| *r == rule.region) {
                Some(i) => i,
                None => {
                    cache.push((rule.region, RegionText::new(rule.region.text(term))));
                    cache.len() - 1
                }
            };
            if rule.matches(&cache[idx].1) {
                return Some(Detection {
                    state: rule.state,
                    rule_id: rule.id.clone(),
                });
            }
        }
        None
    }
}

fn compile_rule(r: &Rule) -> Option<CompiledRule> {
    let list = |v: &[Condition]| v.iter().map(Matcher::from).collect::<Option<Vec<_>>>();
    Some(CompiledRule {
        id: r.id.clone(),
        state: state_of(r.state),
        priority: r.priority,
        region: Region::parse(&r.region)?,
        own: if r.contains.is_some() || r.regex.is_some() {
            Some(Matcher::of(r.contains.as_deref(), r.regex.as_deref())?)
        } else {
            None
        },
        all: list(&r.all)?,
        any: list(&r.any)?,
        not: list(&r.not)?,
    })
}
