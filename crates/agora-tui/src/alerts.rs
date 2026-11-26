//! M6.7: the terminal client tells the person when an agent finishes or needs them, as the web window does since #1227
//! (`apps/web/src/net/finished.ts`, `net/sounds.ts`; docs/ux/main-window.md, "Sounds, and a finished mark that lasts until seen").
//!
//! Pure: what changed between two looks at the fleet ([`changes`]), the "finished" mark that lasts until the agent is seen
//! ([`next_unseen`]), the burst rule ([`gate`], [`flush`]), this client's two settings ([`Prefs`], kept in [`FILE`] in the Agora folder)
//! and the bytes an alert writes to the terminal ([`alert_bytes`]). The caller owns the clock, the file and the terminal.
//!
//! The web's rules, unchanged: an agent *finishes* when it goes from working to idle or finished and ended cleanly (still running, or
//! its program exited by itself with status 0: an agent that failed, was stopped or was interrupted with its machine did not finish
//! anything). An agent *needs you* when it comes onto the Needs you list (a request waiting, or blocked in its terminal on a machine that
//! is online: the tree's `!`), once until it leaves the list; a machine asking to join never alerts. The first look (a console just
//! started) is taken as it is. Never for the agent in focus; the mark is neither set nor kept for an agent on screen.
//!
//! What differs, since a terminal is not a browser: the terminal client cannot tell whether its window is in front, so it takes it as
//! in front (the web's "Also while this window is in front"); the bell is on by default (it is the terminal's own, which the person
//! already tuned or silenced), desktop notifications are off; at most one alert in [`GAP_MS`] (a bell and a notification are louder
//! than the web's soft notes). There is no volume, per-sound switch, per-agent mute or safe mode here.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

use agora_i18n::{key, Arg};
use agora_protocol::v1::{AgentSnapshot, AgentState, Snapshot};

use crate::render::Text;

/// The settings file in the Agora folder (`$AGORA_HOME/tui-settings`, else `~/.agora/tui-settings`), beside `tui-theme`.
pub const FILE: &str = "tui-settings";
/// A settings file larger than this is not read (the defaults hold).
pub const MAX_FILE_BYTES: u64 = 4096;

/// An agent as the person sees it (the web's `UiState`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shown {
    Working,
    NeedsYou,
    Idle,
    Finished,
    Unknown,
}

fn asks(s: &Snapshot, agent: &str) -> bool {
    s.approvals.iter().any(|x| x.agent_id == agent)
}

/// What an agent shows: a closed one is finished whatever state it had, a request waiting makes it "needs you", else what it reported
/// (`apps/web/src/net/view.ts`, `tiles`).
pub fn shown(s: &Snapshot, a: &AgentSnapshot) -> Shown {
    if !a.running {
        return Shown::Finished;
    }
    if asks(s, &a.agent_id) {
        return Shown::NeedsYou;
    }
    match AgentState::try_from(a.state).unwrap_or(AgentState::Unspecified) {
        AgentState::Working => Shown::Working,
        AgentState::Blocked => Shown::NeedsYou,
        AgentState::Idle => Shown::Idle,
        AgentState::Done => Shown::Finished,
        AgentState::Unspecified => Shown::Unknown,
    }
}

/// Who is on the Needs you list, as the tree marks it (`split::app_rows`, `Row::needs_you`): each agent with a request waiting (a request
/// names its agent even before the snapshot lists it), and each agent blocked in its terminal on a machine that is online.
fn asking(s: &Snapshot) -> HashSet<String> {
    let mut out: HashSet<String> = s.approvals.iter().map(|x| x.agent_id.clone()).collect();
    for a in &s.agents {
        let offline = s.nodes.iter().any(|n| n.node_id == a.node_id && !n.online);
        if a.running && !offline && a.state == AgentState::Blocked as i32 {
            out.insert(a.agent_id.clone());
        }
    }
    out
}

/// A closed agent finished only when its program exited by itself with status 0 (`apps/web/src/net/finished.ts`, `endedCleanly`).
fn ended_cleanly(a: &AgentSnapshot) -> bool {
    a.running || (a.exit_code == Some(0) && !a.interrupted)
}

/// What an alert is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Finished,
    NeedsYou,
}

/// One alert that may be given: its kind and the hub-wide id of its agent (empty for a request that names none).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub kind: Kind,
    pub agent: String,
}

/// The last look at the fleet: each agent's state and who was on the Needs you list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seen {
    states: HashMap<String, Shown>,
    asking: HashSet<String>,
}

/// What changed between two looks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    /// Alerts that may be given, at most one per agent and change.
    pub events: Vec<Event>,
    /// Agents that finished now.
    pub finished: Vec<String>,
    /// Agents at work again or waiting on the person: a mark they had is out of date.
    pub resumed: Vec<String>,
    /// Agents no longer listed.
    pub gone: Vec<String>,
    pub next: Seen,
}

/// What changed since `prev` (`None`: the first look, taken as it is).
pub fn changes(prev: Option<&Seen>, s: &Snapshot) -> Changes {
    let next = Seen {
        states: s
            .agents
            .iter()
            .map(|a| (a.agent_id.clone(), shown(s, a)))
            .collect(),
        asking: asking(s),
    };
    let Some(prev) = prev else {
        return Changes {
            next,
            ..Changes::default()
        };
    };
    let mut c = Changes::default();
    for a in &s.agents {
        let now = shown(s, a);
        let Some(&was) = prev.states.get(&a.agent_id) else {
            continue;
        };
        if was == now {
            continue;
        }
        if matches!(now, Shown::Working | Shown::NeedsYou) {
            c.resumed.push(a.agent_id.clone());
        } else if was == Shown::Working
            && matches!(now, Shown::Idle | Shown::Finished)
            && ended_cleanly(a)
        {
            c.finished.push(a.agent_id.clone());
            c.events.push(Event {
                kind: Kind::Finished,
                agent: a.agent_id.clone(),
            });
        }
    }
    let mut gone: Vec<String> = prev
        .states
        .keys()
        .filter(|id| !next.states.contains_key(*id))
        .cloned()
        .collect();
    gone.sort();
    c.gone = gone;
    let mut new_asking: Vec<&String> = next
        .asking
        .iter()
        .filter(|who| !prev.asking.contains(*who))
        .collect();
    new_asking.sort();
    for who in new_asking {
        c.events.push(Event {
            kind: Kind::NeedsYou,
            agent: who.clone(),
        });
    }
    c.next = next;
    c
}

/// The mark's rules (`nextUnseen`): an agent that finished while it was not on screen is marked; being on screen, working again (or
/// waiting on the person) or leaving the list takes the mark away.
pub fn next_unseen(
    cur: &BTreeSet<String>,
    finished: &[String],
    resumed: &[String],
    gone: &[String],
    on_screen: &[String],
) -> BTreeSet<String> {
    let mut out = cur.clone();
    for id in finished {
        if !on_screen.contains(id) {
            out.insert(id.clone());
        }
    }
    for id in resumed.iter().chain(gone).chain(on_screen) {
        out.remove(id);
    }
    out
}

// ---- Settings -------------------------------------------------------------------------------------------------------------------------

/// How a desktop notification is asked of the terminal. Only one is ever sent: a terminal that knows both would show two.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Desktop {
    #[default]
    Off,
    /// `OSC 9 ; text`: iTerm2, Windows Terminal, WezTerm, Ghostty, kitty and others.
    Osc9,
    /// `OSC 777 ; notify ; title ; text`: foot, rxvt-unicode, VTE-based terminals, WezTerm, Ghostty.
    Osc777,
}

/// This client's alert settings, from [`FILE`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Prefs {
    /// The terminal's bell (BEL) when an agent finishes or needs you. On by default.
    pub sound: bool,
    /// A desktop notification through the terminal. Off by default.
    pub desktop: Desktop,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            sound: true,
            desktop: Desktop::Off,
        }
    }
}

fn switch(v: &str) -> Option<bool> {
    match v {
        "on" | "true" | "yes" => Some(true),
        "off" | "false" | "no" => Some(false),
        _ => None,
    }
}

impl Prefs {
    /// Reads the settings text: one `name = value` a line, `#` starts a comment. `sound = on|off`; `desktop_notify = off|on|osc9|osc777`
    /// (`on` is `osc9`). Anything unknown, and a value it does not know, leaves that setting at its default.
    pub fn parse(text: &str) -> Prefs {
        let mut p = Prefs::default();
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            let Some((name, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim().trim_matches('"').to_ascii_lowercase();
            match name.trim() {
                "sound" => {
                    if let Some(on) = switch(&value) {
                        p.sound = on;
                    }
                }
                "desktop_notify" => {
                    p.desktop = match value.as_str() {
                        "osc9" => Desktop::Osc9,
                        "osc777" => Desktop::Osc777,
                        v => match switch(v) {
                            Some(true) => Desktop::Osc9,
                            Some(false) => Desktop::Off,
                            None => p.desktop,
                        },
                    };
                }
                _ => {}
            }
        }
        p
    }

    /// The settings as [`FILE`] keeps them.
    pub fn text(&self) -> String {
        let desktop = match self.desktop {
            Desktop::Off => "off",
            Desktop::Osc9 => "osc9",
            Desktop::Osc777 => "osc777",
        };
        format!(
            "sound = {}\ndesktop_notify = {desktop}\n",
            if self.sound { "on" } else { "off" }
        )
    }

    /// Whether any alert is given at all.
    pub fn any(&self) -> bool {
        self.sound || self.desktop != Desktop::Off
    }
}

/// The settings kept at `file`: the defaults when there is none, it is too large or cannot be read.
pub fn load(file: &Path) -> Prefs {
    match std::fs::metadata(file) {
        Ok(m) if m.len() <= MAX_FILE_BYTES => std::fs::read_to_string(file)
            .map(|t| Prefs::parse(&t))
            .unwrap_or_default(),
        _ => Prefs::default(),
    }
}

/// Keeps the settings at `file`, the folder made when missing.
pub fn save(file: &Path, prefs: &Prefs) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(file, prefs.text())
}

// ---- When an alert is given -----------------------------------------------------------------------------------------------------------

/// Whether this event may alert at all (the burst rule is [`gate`]): never when both are off, never for the agent in focus.
pub fn audible(ev: &Event, prefs: &Prefs, viewing: Option<&str>) -> bool {
    prefs.any() && !(!ev.agent.is_empty() && viewing == Some(ev.agent.as_str()))
}

/// The most urgent of a burst: needs you before finished, the first of a kind.
pub fn most_urgent(evs: &[Event]) -> Option<&Event> {
    evs.iter().fold(None, |best: Option<&Event>, e| match best {
        Some(b) if b.kind >= e.kind => Some(b),
        _ => Some(e),
    })
}

/// At most one alert in this long.
pub const GAP_MS: u64 = 2000;
/// What waits for the end of the gap, bounded for each kind, so many "finished" never push a "needs you" out.
const PENDING_MAX: usize = 32;

fn bounded(evs: Vec<Event>) -> Vec<Event> {
    let keep = |k: Kind| {
        evs.iter()
            .filter(|e| e.kind == k)
            .count()
            .saturating_sub(PENDING_MAX)
    };
    let (mut drop_finished, mut drop_needs) = (keep(Kind::Finished), keep(Kind::NeedsYou));
    evs.into_iter()
        .filter(|e| {
            let d = match e.kind {
                Kind::Finished => &mut drop_finished,
                Kind::NeedsYou => &mut drop_needs,
            };
            if *d > 0 {
                *d -= 1;
                false
            } else {
                true
            }
        })
        .collect()
}

/// The burst rule's memory: when the last alert was given, and what came during the gap since.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Gate {
    pub last_ms: Option<u64>,
    pub pending: Vec<Event>,
}

/// Events that may alert (already [`audible`]) at `now_ms`: the most urgent now when the gap since the last alert is over, else they
/// wait for [`flush`]. Answers the alert to give and the gate after it.
pub fn gate(g: &Gate, evs: Vec<Event>, now_ms: u64) -> (Option<Event>, Gate) {
    let mut all = g.pending.clone();
    all.extend(evs);
    let all = bounded(all);
    if all.is_empty() {
        return (None, g.clone());
    }
    match g.last_ms {
        Some(last) if now_ms.saturating_sub(last) < GAP_MS => (
            None,
            Gate {
                last_ms: Some(last),
                pending: all,
            },
        ),
        _ => (
            most_urgent(&all).cloned(),
            Gate {
                last_ms: Some(now_ms),
                pending: Vec::new(),
            },
        ),
    }
}

/// At the end of the gap: what waited, checked again against the rules as they are now, given as one alert. Nothing before the gap is
/// over.
pub fn flush(g: &Gate, now_ms: u64, still: impl Fn(&Event) -> bool) -> (Option<Event>, Gate) {
    if g.pending.is_empty() || g.last_ms.is_some_and(|l| now_ms.saturating_sub(l) < GAP_MS) {
        return (None, g.clone());
    }
    let pending = g.pending.iter().filter(|e| still(e)).cloned().collect();
    gate(
        &Gate {
            last_ms: g.last_ms,
            pending: Vec::new(),
        },
        pending,
        now_ms,
    )
}

/// The console's alerts: the settings, the last look and the gate. Kept for as long as the console runs (a reconnect is not a first
/// look; only a console just started is).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Alerts {
    pub prefs: Prefs,
    seen: Option<Seen>,
    gate: Gate,
}

impl Alerts {
    pub fn new(prefs: Prefs) -> Self {
        Alerts {
            prefs,
            ..Alerts::default()
        }
    }

    /// A new look at the fleet: the marks in `unseen` follow it (agents in `on_screen` are seen), and answers the alert to give now.
    /// `viewing` is the agent in focus.
    pub fn look(
        &mut self,
        s: &Snapshot,
        unseen: &mut BTreeSet<String>,
        on_screen: &[String],
        viewing: Option<&str>,
        now_ms: u64,
    ) -> Option<Event> {
        let c = changes(self.seen.as_ref(), s);
        self.seen = Some(c.next);
        *unseen = next_unseen(unseen, &c.finished, &c.resumed, &c.gone, on_screen);
        let evs: Vec<Event> = c
            .events
            .into_iter()
            .filter(|e| audible(e, &self.prefs, viewing))
            .collect();
        if evs.is_empty() {
            return None;
        }
        let (play, next) = gate(&self.gate, evs, now_ms);
        self.gate = next;
        play
    }

    /// Whether an alert waits for the end of the gap.
    pub fn waiting(&self) -> bool {
        !self.gate.pending.is_empty()
    }

    /// The alert that waited, once the gap is over, if what it says is still true and still not about the agent in focus.
    pub fn due(&mut self, now_ms: u64, viewing: Option<&str>) -> Option<Event> {
        if !self.waiting() {
            return None;
        }
        let (prefs, seen) = (self.prefs, self.seen.as_ref());
        let still = |e: &Event| {
            audible(e, &prefs, viewing)
                && seen.is_some_and(|s| match e.kind {
                    Kind::NeedsYou => s.asking.contains(&e.agent),
                    Kind::Finished => {
                        matches!(s.states.get(&e.agent), Some(Shown::Idle | Shown::Finished))
                    }
                })
        };
        let (play, next) = flush(&self.gate, now_ms, still);
        self.gate = next;
        play
    }
}

// ---- What is written to the terminal --------------------------------------------------------------------------------------------------

/// The most characters of an agent's name in a notification.
pub const NAME_MAX: usize = 40;
/// The most characters of a notification's title or text.
pub const PAYLOAD_MAX: usize = 160;

/// Text made safe to go inside an OSC sequence: line breaks and other spaces become one space; control characters (BEL, ESC, C1 among
/// them), bidirectional and invisible characters and `;` (OSC 777's separator) are dropped; it is cut to `max` characters with an
/// ellipsis.
pub fn sanitize(text: &str, max: usize) -> String {
    let kept: String = text
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|&c| {
            !c.is_control()
                && !agora_core::text::is_hidden(c)
                && !matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200e}' | '\u{200f}' | '\u{061c}')
                && c != ';'
        })
        .collect();
    let one = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= max {
        return one;
    }
    let mut cut: String = one.chars().take(max.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// A notification's title and text: a fixed phrase in the person's language with the agent's name, made safe and cut.
pub fn message(kind: Kind, name: &str, t: Text<'_>) -> (String, String) {
    let name = sanitize(name, NAME_MAX);
    let body = match (kind, name.is_empty()) {
        (Kind::Finished, false) => t(key!("tui-alert-finished"), &[("agent", Arg::Str(name))]),
        (Kind::NeedsYou, false) => t(key!("tui-alert-needs-you"), &[("agent", Arg::Str(name))]),
        (Kind::Finished, true) => t(key!("tui-alert-finished-unnamed"), &[]),
        (Kind::NeedsYou, true) => t(key!("tui-alert-needs-you-unnamed"), &[]),
    };
    (t(key!("tui-alert-title"), &[]), body)
}

/// The bytes one alert writes to the terminal: the bell when sounds are on, and the desktop notification asked for. Nothing at all when
/// the output is not a terminal (`tty`): no sequence ever goes into a file or a pipe. Title and text are made safe here again, whatever
/// the caller passed.
pub fn alert_bytes(prefs: &Prefs, tty: bool, title: &str, body: &str) -> Vec<u8> {
    let mut out = Vec::new();
    if !tty {
        return out;
    }
    if prefs.sound {
        out.push(0x07);
    }
    let (title, body) = (sanitize(title, PAYLOAD_MAX), sanitize(body, PAYLOAD_MAX));
    match prefs.desktop {
        Desktop::Off => {}
        // Each ends with the String Terminator (ESC \), never BEL: the bell is the sound setting's alone.
        Desktop::Osc9 => {
            out.extend_from_slice(b"\x1b]9;");
            out.extend_from_slice(body.as_bytes());
            out.extend_from_slice(b"\x1b\\");
        }
        Desktop::Osc777 => {
            out.extend_from_slice(b"\x1b]777;notify;");
            out.extend_from_slice(title.as_bytes());
            out.push(b';');
            out.extend_from_slice(body.as_bytes());
            out.extend_from_slice(b"\x1b\\");
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;
