//! The app's own notifications (ADR-0048 decision 5 and Amendment A), pure: what the hub's page may ask for, how the shell checks it,
//! when a notification goes, and what it says. The page names an event (an agent finished, or needs you), the agent's id and its
//! name; the shell keeps nothing else of what the page sent. The words are the shell's own fixed phrases with the name only when it is
//! plain (the page's `plainName`, `apps/web/src/net/attention.ts`), and the id is kept by the shell, never shown, to open that agent
//! when the notification is clicked. The page's title gives a third event, the number of approvals waiting (`waiting` in `main.rs`).

use std::time::{Duration, Instant};

/// At most one notification in this long (the page sends at most one request in this long too: `apps/web/src/net/desktopNotify.ts`).
pub const GAP: Duration = Duration::from_secs(2);
/// A first event waits this long for the ones that come with it (the page's title and its request for the same new approval, two agents
/// finishing together), so they make one notification.
pub const COLLECT: Duration = Duration::from_millis(300);
/// The longest name shown, in UTF-16 units as the page counts it.
pub const NAME_MAX: usize = 24;
/// The longest agent id kept (the page's own bound for an agent key, `MUTED_MAX` entries in `net/sounds.ts`).
pub const AGENT_MAX: usize = 512;
/// The most events read from one request, and kept of each kind while they wait.
pub const BATCH_MAX: usize = 32;
/// The notifications whose click still opens their agent: the newest ones.
pub const CLICKS_KEPT: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Finished,
    NeedsYou,
}

impl Kind {
    /// The page's word for it (`SoundKind` in `net/sounds.ts`); anything else is no event.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "finished" => Some(Self::Finished),
            "needs-you" => Some(Self::NeedsYou),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// An agent finished or needs you, as the page asked; `name` is already checked (`plain_name`).
    Agent {
        kind: Kind,
        agent: String,
        name: Option<String>,
    },
    /// The page's title says this many approvals wait.
    Approvals(u32),
}

/// A name fit to show: letters, digits, spaces and `. _ @ · -`, whitespace runs made one space, at most [`NAME_MAX`]; anything else
/// (a control, bidirectional or invisible character, punctuation that could pose as other text, a long name) is no name, and the
/// notification says "An agent" instead. The same rule as the page's `plainName`.
pub fn plain_name(raw: &str) -> Option<String> {
    let v = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let fits = !v.is_empty() && v.encode_utf16().count() <= NAME_MAX;
    let plain = v.chars().all(|c| {
        c.is_alphabetic() || c.is_numeric() || matches!(c, ' ' | '.' | '_' | '@' | '·' | '-')
    });
    (fits && plain).then_some(v)
}

/// An agent's id as kept: not empty, at most [`AGENT_MAX`] bytes, no control character. It is never shown; it only becomes the address
/// of the agent's view when the notification is clicked (`agent_route`).
pub fn agent_id(raw: &str) -> Option<String> {
    (!raw.is_empty() && raw.len() <= AGENT_MAX && !raw.chars().any(char::is_control))
        .then(|| raw.to_string())
}

/// One event the page asked for, checked; `None` for anything the shell does not take.
pub fn page_event(kind: &str, agent: &str, name: Option<&str>) -> Option<Event> {
    Some(Event::Agent {
        kind: Kind::parse(kind)?,
        agent: agent_id(agent)?,
        name: name.and_then(plain_name),
    })
}

/// The rate rule's memory: when the last notification went, when the next one is due, and what waits for it.
#[derive(Default, Debug)]
pub struct Gate {
    last: Option<Instant>,
    due: Option<Instant>,
    pending: Vec<Event>,
}

impl Gate {
    /// Takes an event at `now`. It waits with the others until the next notification; when none is due yet, one is set for
    /// [`COLLECT`] from now, or for the end of the [`GAP`] since the last, whichever is later, and that time is returned for the caller
    /// to come back then (`take`). An agent's event replaces its earlier one of the same kind, a count of approvals the earlier count,
    /// and each kind keeps at most [`BATCH_MAX`] agents.
    pub fn offer(&mut self, ev: Event, now: Instant) -> Option<Instant> {
        match &ev {
            Event::Approvals(_) => self.pending.retain(|e| !matches!(e, Event::Approvals(_))),
            Event::Agent { kind, agent, .. } => {
                let same = |e: &Event| matches!(e, Event::Agent { kind: k, agent: a, .. } if k == kind && a == agent);
                self.pending.retain(|e| !same(e));
                let of_kind = self
                    .pending
                    .iter()
                    .filter(|e| matches!(e, Event::Agent { kind: k, .. } if k == kind))
                    .count();
                if of_kind >= BATCH_MAX {
                    return None;
                }
            }
        }
        self.pending.push(ev);
        if self.due.is_some() {
            return None;
        }
        let at = match self.last {
            Some(last) => (now + COLLECT).max(last + GAP),
            None => now + COLLECT,
        };
        self.due = Some(at);
        Some(at)
    }

    /// At the time `offer` returned: everything that waited, and the gap starts again.
    pub fn take(&mut self, now: Instant) -> Vec<Event> {
        self.due = None;
        self.last = Some(now);
        std::mem::take(&mut self.pending)
    }
}

/// What one notification says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Text {
    /// "<name> finished." / "<name> needs you."
    Named { kind: Kind, name: String },
    /// "An agent finished." / "3 agents need you."
    Agents { kind: Kind, count: usize },
    /// "2 approvals are waiting for you."
    Approvals(u32),
}

/// One notification: its words and the agent a click opens, if it is about one agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub text: Text,
    pub agent: Option<String>,
}

/// One notification for everything that waited: the most urgent first. An agent that needs you outranks a count of approvals (each
/// approval's agent is on the page's Needs you list, so its request says more), which outranks an agent that finished. About one
/// agent it names it (or says "An agent" when its name is not plain) and a click opens it; about several it counts them.
pub fn summary(evs: &[Event]) -> Option<Notice> {
    let agents = |want: Kind| {
        let mut seen: Vec<(&str, Option<&str>)> = Vec::new();
        for e in evs {
            if let Event::Agent { kind, agent, name } = e {
                if *kind == want && !seen.iter().any(|(a, _)| *a == agent.as_str()) {
                    seen.push((agent.as_str(), name.as_deref()));
                }
            }
        }
        seen
    };
    let about = |kind: Kind, seen: Vec<(&str, Option<&str>)>| -> Option<Notice> {
        match seen.as_slice() {
            [] => None,
            [(agent, name)] => Some(Notice {
                text: match name {
                    Some(n) => Text::Named {
                        kind,
                        name: (*n).to_string(),
                    },
                    None => Text::Agents { kind, count: 1 },
                },
                agent: Some((*agent).to_string()),
            }),
            more => Some(Notice {
                text: Text::Agents {
                    kind,
                    count: more.len(),
                },
                agent: None,
            }),
        }
    };
    about(Kind::NeedsYou, agents(Kind::NeedsYou))
        .or_else(|| {
            evs.iter()
                .filter_map(|e| match e {
                    Event::Approvals(n) if *n > 0 => Some(*n),
                    _ => None,
                })
                .max()
                .map(|n| Notice {
                    text: Text::Approvals(n),
                    agent: None,
                })
        })
        .or_else(|| about(Kind::Finished, agents(Kind::Finished)))
}

/// The page's address of an agent's view, `#/agent/<id>` with the id written as the page's `encodeURIComponent` writes it, so the
/// page reads back exactly the id it sent.
pub fn agent_route(agent: &str) -> String {
    let mut out = String::from("#/agent/");
    for b in agent.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The clicks the shell can still answer: a notification's number and the agent it opens, the newest [`CLICKS_KEPT`].
#[derive(Default, Debug)]
pub struct Clicks {
    next: i32,
    kept: std::collections::VecDeque<(i32, Option<String>)>,
}

impl Clicks {
    /// A number for a new notification, remembering the agent its click opens.
    pub fn add(&mut self, agent: Option<String>) -> i32 {
        self.next = self.next.checked_add(1).unwrap_or(1);
        if self.kept.len() >= CLICKS_KEPT {
            self.kept.pop_front();
        }
        self.kept.push_back((self.next, agent));
        self.next
    }

    /// The agent a click on notification `id` opens; `None` for one that names no agent or is no longer kept.
    pub fn agent(&self, id: i32) -> Option<String> {
        self.kept
            .iter()
            .find(|(n, _)| *n == id)
            .and_then(|(_, a)| a.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_name_is_kept_and_anything_else_is_no_name() {
        assert_eq!(plain_name("codex · teal").as_deref(), Some("codex · teal"));
        assert_eq!(
            plain_name("  claude\t amber ").as_deref(),
            Some("claude amber")
        );
        assert_eq!(plain_name("ajan-1_b@x.y").as_deref(), Some("ajan-1_b@x.y"));
        assert_eq!(
            plain_name("Çalışkan ağaç").as_deref(),
            Some("Çalışkan ağaç")
        );
        // Exactly the cap fits, one more does not (UTF-16 units, as the page counts).
        assert_eq!(
            plain_name(&"a".repeat(NAME_MAX)).map(|s| s.len()),
            Some(NAME_MAX)
        );
        assert_eq!(plain_name(&"a".repeat(NAME_MAX + 1)), None);
        assert_eq!(
            plain_name(&"𝒜".repeat(13)),
            None,
            "13 letters outside the BMP are 26 UTF-16 units"
        );
        for bad in [
            "",
            "   ",
            "evil\u{202E}txt",     // right-to-left override
            "a\u{200B}b",          // zero-width space
            "line\nbreak\u{0007}", // a control character survives the whitespace fold
            "Allow? <b>",
            "run: rm -rf /",
            "https://x.example",
            "a;b",
            "\u{1b}[31mred",
        ] {
            assert_eq!(plain_name(bad), None, "{bad:?} must not be shown");
        }
    }

    #[test]
    fn the_page_s_event_is_checked_field_by_field() {
        assert_eq!(
            page_event("finished", "node-1/a7", Some("codex · teal")),
            Some(Event::Agent {
                kind: Kind::Finished,
                agent: "node-1/a7".into(),
                name: Some("codex · teal".into())
            })
        );
        // A name that is not plain makes the generic text, not a refusal.
        assert_eq!(
            page_event("needs-you", "n/a", Some("x\u{202E}y")),
            Some(Event::Agent {
                kind: Kind::NeedsYou,
                agent: "n/a".into(),
                name: None
            })
        );
        assert_eq!(page_event("needs-you", "n/a", None).map(|_| ()), Some(()));
        // Unknown kinds and unusable ids are no event.
        for (kind, agent) in [
            ("Finished", "n/a"),
            ("approval", "n/a"),
            ("", "n/a"),
            ("finished", ""),
            ("finished", "n/\u{0}a"),
        ] {
            assert_eq!(page_event(kind, agent, None), None, "{kind:?} {agent:?}");
        }
        assert_eq!(
            page_event("finished", &"x".repeat(AGENT_MAX + 1), None),
            None
        );
        assert!(page_event("finished", &"x".repeat(AGENT_MAX), None).is_some());
    }

    fn ev(kind: Kind, agent: &str, name: Option<&str>) -> Event {
        Event::Agent {
            kind,
            agent: agent.into(),
            name: name.map(Into::into),
        }
    }

    #[test]
    fn the_first_event_waits_briefly_and_the_next_waits_for_the_gap() {
        let t0 = Instant::now();
        let mut g = Gate::default();
        assert_eq!(
            g.offer(ev(Kind::Finished, "a", None), t0),
            Some(t0 + COLLECT)
        );
        // Another while one is due: it joins, no second time is set.
        assert_eq!(
            g.offer(Event::Approvals(1), t0 + Duration::from_millis(5)),
            None
        );
        let shown = g.take(t0 + COLLECT);
        assert_eq!(shown.len(), 2);
        // The next one, just after: due at the end of the gap since the last.
        let t1 = t0 + COLLECT + Duration::from_millis(100);
        assert_eq!(
            g.offer(ev(Kind::NeedsYou, "b", None), t1),
            Some(t0 + COLLECT + GAP)
        );
        assert_eq!(g.offer(ev(Kind::NeedsYou, "c", None), t1), None);
        assert_eq!(g.take(t0 + COLLECT + GAP).len(), 2);
        // Long after: only the short wait again.
        let t2 = t0 + Duration::from_secs(60);
        assert_eq!(
            g.offer(ev(Kind::Finished, "a", None), t2),
            Some(t2 + COLLECT)
        );
    }

    #[test]
    fn a_burst_is_one_notification_and_bounded() {
        let t0 = Instant::now();
        let mut g = Gate::default();
        let mut due = 0;
        for i in 0..200 {
            due += usize::from(
                g.offer(ev(Kind::Finished, &format!("a{i}"), None), t0)
                    .is_some(),
            );
        }
        // The same agent again replaces its event; a count of approvals replaces the earlier count.
        g.offer(ev(Kind::NeedsYou, "x", Some("one")), t0);
        g.offer(ev(Kind::NeedsYou, "x", Some("two")), t0);
        g.offer(Event::Approvals(1), t0);
        g.offer(Event::Approvals(3), t0);
        assert_eq!(due, 1, "one notification is due for the whole burst");
        let shown = g.take(t0 + COLLECT);
        let finished = shown
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    Event::Agent {
                        kind: Kind::Finished,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(
            finished, BATCH_MAX,
            "many finished agents never crowd out the rest"
        );
        assert!(shown.contains(&ev(Kind::NeedsYou, "x", Some("two"))));
        assert!(!shown.contains(&ev(Kind::NeedsYou, "x", Some("one"))));
        assert!(shown.contains(&Event::Approvals(3)) && !shown.contains(&Event::Approvals(1)));
        assert!(g.take(t0 + COLLECT).is_empty(), "nothing is shown twice");
    }

    #[test]
    fn the_most_urgent_is_said_and_one_agent_is_named() {
        let named = |kind, name: &str| Text::Named {
            kind,
            name: name.into(),
        };
        // One agent: named, and a click opens it.
        assert_eq!(
            summary(&[ev(Kind::Finished, "n/1", Some("codex · teal"))]),
            Some(Notice {
                text: named(Kind::Finished, "codex · teal"),
                agent: Some("n/1".into())
            })
        );
        // Without a plain name: "An agent", still opened by a click.
        assert_eq!(
            summary(&[ev(Kind::NeedsYou, "n/1", None)]),
            Some(Notice {
                text: Text::Agents {
                    kind: Kind::NeedsYou,
                    count: 1
                },
                agent: Some("n/1".into())
            })
        );
        // Needs you before approvals before finished; the title's count and the page's request for one approval make one notice.
        assert_eq!(
            summary(&[
                ev(Kind::Finished, "n/1", Some("a")),
                Event::Approvals(1),
                ev(Kind::NeedsYou, "n/2", Some("b"))
            ]),
            Some(Notice {
                text: named(Kind::NeedsYou, "b"),
                agent: Some("n/2".into())
            })
        );
        assert_eq!(
            summary(&[ev(Kind::Finished, "n/1", Some("a")), Event::Approvals(2)]),
            Some(Notice {
                text: Text::Approvals(2),
                agent: None
            })
        );
        // Several agents: counted, a click only brings the window forward.
        assert_eq!(
            summary(&[
                ev(Kind::Finished, "n/1", Some("a")),
                ev(Kind::Finished, "n/2", Some("b")),
                ev(Kind::Finished, "n/1", Some("a"))
            ]),
            Some(Notice {
                text: Text::Agents {
                    kind: Kind::Finished,
                    count: 2
                },
                agent: None
            })
        );
        assert_eq!(summary(&[]), None);
        assert_eq!(summary(&[Event::Approvals(0)]), None);
    }

    #[test]
    fn the_route_is_the_page_s_encode_uri_component() {
        assert_eq!(agent_route("node-1/a7"), "#/agent/node-1%2Fa7");
        assert_eq!(agent_route("a b\"';<>"), "#/agent/a%20b%22'%3B%3C%3E");
        assert_eq!(agent_route("ç"), "#/agent/%C3%A7");
        assert_eq!(agent_route("A-Z_a.z!~*()0"), "#/agent/A-Z_a.z!~*()0");
    }

    #[test]
    fn a_click_finds_its_agent_among_the_newest() {
        let mut c = Clicks::default();
        let first = c.add(Some("n/1".into()));
        let none = c.add(None);
        assert_eq!(c.agent(first).as_deref(), Some("n/1"));
        assert_eq!(c.agent(none), None);
        assert_eq!(c.agent(9999), None);
        for _ in 0..CLICKS_KEPT {
            c.add(Some("n/2".into()));
        }
        assert_eq!(
            c.agent(first),
            None,
            "an old notification's click is forgotten"
        );
    }
}
