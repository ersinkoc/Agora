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

enum Matcher {
    Contains(String),
    Regex(Regex),
}

impl Matcher {
    fn from(c: &Condition) -> Option<Self> {
        Self::of(c.contains.as_deref(), c.regex.as_deref())
    }

    fn of(contains: Option<&str>, regex: Option<&str>) -> Option<Self> {
        match (contains, regex) {
            (Some(s), None) => Some(Self::Contains(s.to_string())),
            (None, Some(r)) => compile(r).ok().map(Self::Regex),
            _ => None,
        }
    }

    fn is_match(&self, text: &str) -> bool {
        match self {
            Self::Contains(s) => text.contains(s.as_str()),
            Self::Regex(r) => r.is_match(text),
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
    fn matches(&self, text: &str) -> bool {
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

    /// The state the highest-priority matching rule reports, or `None` when no rule matches (the caller keeps
    /// the previous state or falls back to `Unknown`). Region text is built once per distinct region.
    pub fn detect(&self, term: &dyn Terminal) -> Option<Detection> {
        let mut cache: Vec<(Region, String)> = Vec::new();
        for rule in &self.rules {
            let idx = match cache.iter().position(|(r, _)| *r == rule.region) {
                Some(i) => i,
                None => {
                    cache.push((rule.region, rule.region.text(term)));
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
