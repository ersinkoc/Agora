//! Which agent each pane shows, and which panes hold a live terminal stream (ADR-0068 decision 7, ADR-0022). Only panes on screen
//! subscribe: a tab behind another, a pane hidden by a maximized one, or every pane but the one in focus in a window too small to
//! tile holds none. The same rules as the web window's `apps/web/src/layout/streams.ts`; no I/O here, the caller opens and closes.

use std::collections::BTreeMap;

use super::tree::{Follow, Pane};

/// What the panes that follow something resolve to now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bindings {
    /// The agent selected in the agent list.
    pub selection: Option<String>,
    /// Pane id to the agent that pane follows for "needs you".
    pub attention: BTreeMap<String, String>,
}

/// The agent a pane shows now, if any.
pub fn agent_of<'a>(pane: &'a Pane, b: &'a Bindings) -> Option<&'a str> {
    match pane.follow() {
        Some(Follow::Selection) => b.selection.as_deref(),
        Some(Follow::Attention) => b.attention.get(&pane.id).map(String::as_str),
        None => pane.bound_agent(),
    }
}

/// Panes that follow attention each show a different agent that needs you, oldest first. A pane keeps its agent while that agent
/// still needs you (it never jumps away mid-read); when it stops, the pane moves on to the next one nobody shows.
pub fn assign_attention(
    panes: &[String],
    prev: &BTreeMap<String, String>,
    waiting: &[String],
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut taken: Vec<&str> = Vec::new();
    for p in panes {
        if let Some(a) = prev.get(p) {
            if waiting.contains(a) && !taken.contains(&a.as_str()) {
                out.insert(p.clone(), a.clone());
                taken.push(a);
            }
        }
    }
    let mut free = waiting.iter().filter(|a| !taken.contains(&a.as_str()));
    for p in panes {
        if out.contains_key(p) {
            continue;
        }
        if let Some(a) = free.next() {
            out.insert(p.clone(), a.clone());
        }
    }
    out
}

/// Pane id to agent id, for every pane on screen (`visible`, in drawing order) whose panel draws a terminal (`streams`) and whose
/// agent has one.
pub fn stream_plan(
    visible: &[&Pane],
    b: &Bindings,
    streams: impl Fn(&str) -> bool,
    has_terminal: impl Fn(&str) -> bool,
) -> BTreeMap<String, String> {
    let mut plan = BTreeMap::new();
    for p in visible {
        if !streams(&p.panel) {
            continue;
        }
        if let Some(a) = agent_of(p, b).filter(|a| has_terminal(a)) {
            plan.insert(p.id.clone(), a.to_string());
        }
    }
    plan
}

/// The live terminal streams, kept in step with what is on screen: one per agent shown (two panes of the same agent share it), so a
/// pane that keeps its agent keeps its stream through a resize, a move or a preset, and anything no longer shown is closed. `H` is
/// the caller's handle (a hub stream in the terminal client, a fake in tests).
#[derive(Debug)]
pub struct Streams<H> {
    open: BTreeMap<String, H>,
}

impl<H> Default for Streams<H> {
    fn default() -> Self {
        Self {
            open: BTreeMap::new(),
        }
    }
}

impl<H> Streams<H> {
    /// Takes out the streams of agents not in `want`, for the caller to close.
    pub fn unwanted(&mut self, want: &[&str]) -> Vec<H> {
        let gone: Vec<String> = self
            .open
            .keys()
            .filter(|a| !want.contains(&a.as_str()))
            .cloned()
            .collect();
        gone.into_iter()
            .filter_map(|a| self.open.remove(&a))
            .collect()
    }

    /// Opens the streams of agents in `want` that have none. `open` may fail (`None`): it is asked again at the next call, so the
    /// caller decides how often to try.
    pub fn open_missing(&mut self, want: &[&str], mut open: impl FnMut(&str) -> Option<H>) {
        for a in want {
            if !self.open.contains_key(*a) {
                if let Some(h) = open(a) {
                    self.open.insert((*a).to_string(), h);
                }
            }
        }
    }

    /// Closes the streams of agents not in `want` and opens the missing ones.
    pub fn sync<'a>(
        &mut self,
        want: impl IntoIterator<Item = &'a str>,
        open: impl FnMut(&str) -> Option<H>,
        mut close: impl FnMut(H),
    ) {
        let want: Vec<&str> = want.into_iter().collect();
        for h in self.unwanted(&want) {
            close(h);
        }
        self.open_missing(&want, open);
    }

    /// Every open stream, by agent.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &H)> {
        self.open.iter().map(|(a, h)| (a.as_str(), h))
    }

    pub fn get(&self, agent: &str) -> Option<&H> {
        self.open.get(agent)
    }

    pub fn len(&self) -> usize {
        self.open.len()
    }

    pub fn is_empty(&self) -> bool {
        self.open.is_empty()
    }

    pub fn agents(&self) -> impl Iterator<Item = &str> {
        self.open.keys().map(String::as_str)
    }

    /// Every open stream, to pump. Each pump is bounded by the caller (the terminal client's `Preview::pump` takes at most a batch),
    /// so a flood in one stream never holds up another or the keyboard.
    pub fn each_mut(&mut self) -> impl Iterator<Item = (&str, &mut H)> {
        self.open.iter_mut().map(|(a, h)| (a.as_str(), h))
    }

    /// Drops one stream that ended (closed by the caller already).
    pub fn forget(&mut self, agent: &str) -> Option<H> {
        self.open.remove(agent)
    }

    /// Closes them all (the console quits or reconnects).
    pub fn close_all(&mut self, mut close: impl FnMut(H)) {
        for (_, h) in std::mem::take(&mut self.open) {
            close(h);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::engine::{reduce, visible_panes, Action, Preset, Workspace};
    use super::super::tree::{panes_of, Layout};
    use super::*;
    use std::cell::RefCell;

    /// A fake hub: counts what is connected, per agent.
    #[derive(Default)]
    struct Fake {
        live: RefCell<BTreeMap<String, i32>>,
        connects: RefCell<usize>,
    }

    impl Fake {
        fn open(&self, a: &str) -> Option<String> {
            *self.connects.borrow_mut() += 1;
            *self.live.borrow_mut().entry(a.to_string()).or_default() += 1;
            Some(a.to_string())
        }
        fn close(&self, h: String) {
            *self.live.borrow_mut().entry(h).or_default() -= 1;
        }
        fn live(&self) -> Vec<String> {
            self.live
                .borrow()
                .iter()
                .filter(|(_, n)| **n > 0)
                .map(|(a, _)| a.clone())
                .collect()
        }
    }

    fn streams_panel(p: &str) -> bool {
        p == "agent" || p == "terminal"
    }

    fn terminals(a: &str) -> bool {
        !a.starts_with("chat")
    }

    fn sync(s: &mut Streams<String>, f: &Fake, ws: &Workspace, narrow: bool, b: &Bindings) {
        let plan = stream_plan(&visible_panes(ws, narrow), b, streams_panel, terminals);
        s.sync(
            plan.values().map(String::as_str),
            |a| f.open(a),
            |h| f.close(h),
        );
    }

    fn grid(preset: Preset, list: &[&str]) -> Workspace {
        reduce(
            &Workspace::of(Layout::default_layout()),
            &Action::Preset {
                preset,
                agents: list.iter().map(|a| a.to_string()).collect(),
            },
        )
    }

    fn id_of(ws: &Workspace, agent: &str) -> String {
        panes_of(&ws.layout.root)
            .iter()
            .find(|p| p.bound_agent() == Some(agent))
            .map(|p| p.id.clone())
            .unwrap_or_default()
    }

    #[test]
    fn a_two_by_two_subscribes_four_a_background_tab_none_and_closing_closes_all() {
        let f = Fake::default();
        let mut s = Streams::default();
        let none = Bindings::default();
        let mut ws = grid(Preset::TwoByTwo, &["a", "b", "c", "d", "e"]);
        sync(&mut s, &f, &ws, false, &none);
        assert_eq!(f.live(), ["a", "b", "c", "d"]); // e is a tab behind d
        ws = reduce(
            &ws,
            &Action::Activate {
                pane: id_of(&ws, "e"),
            },
        );
        sync(&mut s, &f, &ws, false, &none);
        assert_eq!(f.live(), ["a", "b", "c", "e"]);
        s.close_all(|h| f.close(h));
        assert!(f.live().is_empty());
    }

    #[test]
    fn maximizing_unsubscribes_the_others_and_restoring_subscribes_them_again() {
        let f = Fake::default();
        let mut s = Streams::default();
        let none = Bindings::default();
        let mut ws = grid(Preset::TwoByTwo, &["a", "b", "c", "d"]);
        sync(&mut s, &f, &ws, false, &none);
        ws = reduce(
            &ws,
            &Action::Maximize {
                pane: id_of(&ws, "b"),
            },
        );
        sync(&mut s, &f, &ws, false, &none);
        assert_eq!(f.live(), ["b"]);
        assert_eq!(*f.connects.borrow(), 4); // b kept its stream
        ws = reduce(&ws, &Action::Restore);
        sync(&mut s, &f, &ws, false, &none);
        assert_eq!(f.live(), ["a", "b", "c", "d"]);
    }

    #[test]
    fn resizing_swapping_moving_and_a_preset_keep_every_stream() {
        let f = Fake::default();
        let mut s = Streams::default();
        let none = Bindings::default();
        let mut ws = grid(Preset::TwoByTwo, &["a", "b", "c", "d"]);
        sync(&mut s, &f, &ws, false, &none);
        let steps = [
            Action::Resize {
                path: vec![0],
                index: 0,
                delta: 0.15,
            },
            Action::Resize {
                path: vec![],
                index: 0,
                delta: -0.2,
            },
            Action::Swap {
                a: id_of(&ws, "a"),
                b: id_of(&ws, "d"),
            },
            Action::Move {
                pane: id_of(&ws, "c"),
                target: id_of(&ws, "b"),
                place: super::super::engine::Place::Bottom,
            },
            Action::Preset {
                preset: Preset::ThreeByTwo,
                agents: ["a", "b", "c", "d"].iter().map(|x| x.to_string()).collect(),
            },
        ];
        for step in &steps {
            ws = reduce(&ws, step);
            sync(&mut s, &f, &ws, false, &none);
        }
        assert_eq!(f.live(), ["a", "b", "c", "d"]);
        assert_eq!(*f.connects.borrow(), 4);
    }

    #[test]
    fn a_window_too_small_to_tile_shows_one_pane_so_one_stream() {
        let f = Fake::default();
        let mut s = Streams::default();
        let ws = grid(Preset::TwoByTwo, &["a", "b", "c", "d"]);
        let ws = reduce(
            &ws,
            &Action::Focus {
                pane: id_of(&ws, "c"),
            },
        );
        sync(&mut s, &f, &ws, true, &Bindings::default());
        assert_eq!(f.live(), ["c"]);
    }

    #[test]
    fn chat_agents_and_other_panels_hold_no_stream_and_two_panes_of_one_agent_share_one() {
        let mut ws = grid(Preset::TwoByTwo, &["a", "chat-1"]);
        let third = panes_of(&ws.layout.root)[2].id.clone();
        let fourth = panes_of(&ws.layout.root)[3].id.clone();
        ws = reduce(
            &ws,
            &Action::Bind {
                pane: third,
                bind: None,
                panel: Some("approvals".into()),
            },
        );
        ws = reduce(
            &ws,
            &Action::Bind {
                pane: fourth,
                bind: Some(super::super::tree::Bind::Agent("a".into())),
                panel: Some("terminal".into()),
            },
        );
        let b = Bindings::default();
        let plan = stream_plan(&visible_panes(&ws, false), &b, streams_panel, terminals);
        assert_eq!(plan.len(), 2);
        let f = Fake::default();
        let mut s = Streams::default();
        sync(&mut s, &f, &ws, false, &b);
        assert_eq!(f.live(), ["a"]);
        assert_eq!(*f.connects.borrow(), 1);
    }

    #[test]
    fn panes_that_follow_resolve_through_the_bindings() {
        let mut ws = grid(Preset::TwoByOne, &[]);
        let (p1, p2) = (
            panes_of(&ws.layout.root)[0].id.clone(),
            panes_of(&ws.layout.root)[1].id.clone(),
        );
        ws = reduce(
            &ws,
            &Action::Bind {
                pane: p1.clone(),
                bind: Some(super::super::tree::Bind::Follow(Follow::Attention)),
                panel: None,
            },
        );
        ws = reduce(
            &ws,
            &Action::Bind {
                pane: p2.clone(),
                bind: Some(super::super::tree::Bind::Follow(Follow::Selection)),
                panel: None,
            },
        );
        let b = Bindings {
            selection: Some("sel".into()),
            attention: [(p1.clone(), "waits".to_string())].into_iter().collect(),
        };
        let plan = stream_plan(&visible_panes(&ws, false), &b, streams_panel, terminals);
        assert_eq!(plan.get(&p1).map(String::as_str), Some("waits"));
        assert_eq!(plan.get(&p2).map(String::as_str), Some("sel"));
    }

    #[test]
    fn attention_panes_keep_their_agent_while_it_waits_then_move_on() {
        let panes = vec!["p1".to_string(), "p2".to_string()];
        let w = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let first = assign_attention(&panes, &BTreeMap::new(), &w(&["a", "b", "c"]));
        assert_eq!(first.get("p1").map(String::as_str), Some("a"));
        assert_eq!(first.get("p2").map(String::as_str), Some("b"));
        // b still waits: p2 keeps it even though it is no longer second; a stopped: p1 moves on to c.
        let next = assign_attention(&panes, &first, &w(&["b", "c"]));
        assert_eq!(next.get("p2").map(String::as_str), Some("b"));
        assert_eq!(next.get("p1").map(String::as_str), Some("c"));
        // Nothing waits: no pane shows anything.
        assert!(assign_attention(&panes, &next, &[]).is_empty());
    }

    /// A fake stream that holds `queued` items and hands out at most `batch` per pump, as `Preview::pump` does.
    struct Flood {
        queued: usize,
        seen: usize,
    }

    #[test]
    fn a_flood_in_one_pane_never_starves_another() {
        const BATCH: usize = 256;
        let mut s: Streams<Flood> = Streams::default();
        s.sync(
            ["flood", "quiet"],
            |a| {
                Some(Flood {
                    queued: if a == "flood" { 1_000_000 } else { 1 },
                    seen: 0,
                })
            },
            |_| {},
        );
        // One turn of the console: every stream is pumped once, each at most a batch, before the keyboard is read again.
        let mut work = 0;
        for (_, h) in s.each_mut() {
            let n = h.queued.min(BATCH);
            h.queued -= n;
            h.seen += n;
            work += n;
        }
        assert_eq!(s.get("quiet").map(|h| h.seen), Some(1));
        assert!(
            work <= BATCH * 2,
            "one turn handles at most a batch per pane"
        );
    }
}
