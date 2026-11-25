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
pub enum Region {
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

    /// The region as a manifest writes it (`title`, `bottom_lines(8)`): data, shown by `agora agent explain`.
    pub fn name(self) -> String {
        match self {
            Self::Title => "title".to_string(),
            Self::BottomLines(n) => format!("bottom_lines({n})"),
            Self::BottomNonEmptyLines(n) => format!("bottom_non_empty_lines({n})"),
        }
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

    /// The region's words with every run of whitespace as one space, and one space at each end: the region's edges are boundaries
    /// too (rows are read without their trailing blanks, so a phrase padded at its end still matches at the end of the last row).
    fn collapsed(&self) -> &str {
        self.collapsed.get_or_init(|| {
            let mut out = String::with_capacity(self.raw.len() + 2);
            out.push(' ');
            for word in self.raw.split_whitespace() {
                out.push_str(word);
                out.push(' ');
            }
            out
        })
    }
}

// A phrase's whitespace collapsed, keeping one space for a run at either end as well. A manifest can
// use that space to require a boundary around a phrase, even when the terminal wraps the phrase over
// several rows.
fn collapse_whitespace<'a>(text: &str, words: impl IntoIterator<Item = &'a str>) -> String {
    let mut normalized = String::with_capacity(text.len());
    let mut space = text.starts_with(char::is_whitespace);
    for word in words {
        if space {
            normalized.push(' ');
        }
        normalized.push_str(word);
        space = true;
    }
    if text.ends_with(char::is_whitespace) {
        normalized.push(' ');
    }
    normalized
}

enum Matcher {
    /// The phrase, and the same phrase with its whitespace collapsed when it holds more than one word or has whitespace at an end.
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
                // A phrase of whitespace only has no word to bound: it keeps its raw match only.
                let padded = !words.is_empty()
                    && (s.starts_with(char::is_whitespace) || s.ends_with(char::is_whitespace));
                let collapsed = (words.len() > 1 || padded).then(|| collapse_whitespace(s, words));
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

impl CompiledRule {
    /// The first condition that kept the rule from matching, checked in the order [`CompiledRule::matches`] reads them, or
    /// [`RuleOutcome::Matched`]. Only `agora agent explain` asks; detection uses `matches`.
    fn outcome(&self, text: &RegionText) -> RuleOutcome {
        if self.own.as_ref().is_some_and(|m| !m.is_match(text)) {
            return RuleOutcome::OwnMissing;
        }
        if let Some(i) = self.all.iter().position(|m| !m.is_match(text)) {
            return RuleOutcome::AllMissing(i);
        }
        if !self.any.is_empty() && !self.any.iter().any(|m| m.is_match(text)) {
            return RuleOutcome::AnyMissing;
        }
        if let Some(i) = self.not.iter().position(|m| m.is_match(text)) {
            return RuleOutcome::NotPresent(i);
        }
        RuleOutcome::Matched
    }
}

/// Why one rule did or did not match a screen (`agora agent explain`). Indexes count from 0 in the manifest's list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleOutcome {
    /// Every condition held.
    Matched,
    /// The rule's own `contains`/`regex` was not found.
    OwnMissing,
    /// This `all` condition was not found.
    AllMissing(usize),
    /// None of the `any` conditions was found.
    AnyMissing,
    /// This `not` condition was found.
    NotPresent(usize),
}

/// One rule checked against a screen, in the order detection reads them (highest priority first, file order on ties).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleCheck {
    pub id: String,
    pub state: AgentState,
    pub priority: i32,
    pub region: Region,
    pub outcome: RuleOutcome,
}

/// Every rule of a [`Detector`] checked against one screen, the text of each region they read, and what detection decides: the
/// first matching rule, exactly as [`Detector::detect`] would.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Explanation {
    pub checks: Vec<RuleCheck>,
    /// Each region the rules read, once, in the order first read, with its text.
    pub regions: Vec<(Region, String)>,
    pub decided: Option<Detection>,
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

impl Detector {
    /// Every rule checked against `term`, not only up to the first match, with the reason each one failed. For `agora agent explain`
    /// and manifest authors; detection calls [`Detector::detect`], which stops at the first match.
    pub fn explain(&self, term: &dyn Terminal) -> Explanation {
        let mut regions: Vec<(Region, RegionText)> = Vec::new();
        let mut checks = Vec::with_capacity(self.rules.len());
        let mut decided = None;
        for rule in &self.rules {
            let idx = match regions.iter().position(|(r, _)| *r == rule.region) {
                Some(i) => i,
                None => {
                    regions.push((rule.region, RegionText::new(rule.region.text(term))));
                    regions.len() - 1
                }
            };
            let outcome = rule.outcome(&regions[idx].1);
            if outcome == RuleOutcome::Matched && decided.is_none() {
                decided = Some(Detection {
                    state: rule.state,
                    rule_id: rule.id.clone(),
                });
            }
            checks.push(RuleCheck {
                id: rule.id.clone(),
                state: rule.state,
                priority: rule.priority,
                region: rule.region,
                outcome,
            });
        }
        Explanation {
            checks,
            regions: regions.into_iter().map(|(r, t)| (r, t.raw)).collect(),
            decided,
        }
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
