//! What agents without a terminal say (ADR-0038): a bounded transcript per agent that clients can read and follow.
//!
//! The adapter host reports chat items to the node; the node keeps the latest of them, plus the requests users sent, in a
//! ring per agent (like a terminal's screen, kept in memory and gone when the node restarts). A subscriber gets the ring
//! first and every entry after it. Nothing here touches the hub's event store.

use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;

use agora_adapters::host::ChatItem;
use agora_core::AgentId;
use agora_protocol::v1::{ChatEntry, ChatKind, ChatRole};

/// Most entries and most bytes of text a transcript keeps; older entries are dropped first.
pub const MAX_ENTRIES: usize = 500;
pub const MAX_BYTES: usize = 256 * 1024;
/// Longest text of one entry; more is cut.
pub const MAX_ENTRY_BYTES: usize = 16 * 1024;

#[derive(Default)]
struct Transcript {
    entries: VecDeque<ChatEntry>,
    bytes: usize,
    next_id: u64,
    followers: Vec<Sender<ChatEntry>>,
}

/// The transcripts of every chat agent on this node.
#[derive(Default)]
pub struct ChatBoard {
    agents: Mutex<HashMap<AgentId, Transcript>>,
}

fn cut(mut text: String) -> String {
    if text.len() > MAX_ENTRY_BYTES {
        let mut end = MAX_ENTRY_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

/// What an adapter's chat item looks like as an entry (kind, text, path).
pub fn describe(item: &ChatItem) -> (ChatKind, String, String) {
    match item {
        ChatItem::Text(t) => (ChatKind::Text, t.clone(), String::new()),
        ChatItem::ToolCall {
            tool,
            arguments_json,
        } => (
            ChatKind::ToolCall,
            format!(
                "{{\"tool\":{},\"arguments\":{}}}",
                json_string(tool),
                if arguments_json.is_empty() {
                    "null"
                } else {
                    arguments_json
                }
            ),
            String::new(),
        ),
        ChatItem::Plan(lines) => (ChatKind::Plan, lines.join("\n"), String::new()),
        ChatItem::Diff { path, unified_diff } => {
            (ChatKind::Diff, unified_diff.clone(), path.clone())
        }
    }
}

fn json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

impl ChatBoard {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<AgentId, Transcript>> {
        self.agents.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Adds one entry to an agent's transcript and hands it to everyone following it.
    pub fn push(
        &self,
        agent: &AgentId,
        role: ChatRole,
        kind: ChatKind,
        text: String,
        path: String,
        at_ms: u64,
    ) {
        let mut g = self.lock();
        let t = g.entry(agent.clone()).or_default();
        t.next_id += 1;
        let entry = ChatEntry {
            entry_id: t.next_id,
            at_ms,
            role: role as i32,
            kind: kind as i32,
            text: cut(text),
            path,
        };
        t.bytes += entry.text.len() + entry.path.len();
        t.entries.push_back(entry.clone());
        while t.entries.len() > MAX_ENTRIES || t.bytes > MAX_BYTES {
            let Some(old) = t.entries.pop_front() else {
                break;
            };
            t.bytes = t.bytes.saturating_sub(old.text.len() + old.path.len());
        }
        // A follower that went away is dropped here.
        t.followers.retain(|f| f.send(entry.clone()).is_ok());
    }

    /// What an adapter said.
    pub fn push_item(&self, agent: &AgentId, item: &ChatItem, at_ms: u64) {
        let (kind, text, path) = describe(item);
        self.push(agent, ChatRole::Agent, kind, text, path, at_ms);
    }

    /// The transcript so far.
    pub fn snapshot(&self, agent: &AgentId) -> Vec<ChatEntry> {
        self.lock()
            .get(agent)
            .map(|t| t.entries.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// The transcript so far and a channel with every entry from now on, taken together so none is missed or repeated.
    pub fn follow(&self, agent: &AgentId) -> (Vec<ChatEntry>, Receiver<ChatEntry>) {
        let mut g = self.lock();
        let t = g.entry(agent.clone()).or_default();
        let (tx, rx) = channel();
        t.followers.push(tx);
        (t.entries.iter().cloned().collect(), rx)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn entries_get_increasing_ids_and_followers_see_exactly_what_comes_after() {
        let b = ChatBoard::new();
        let a = AgentId::new("agent-1");
        b.push_item(&a, &ChatItem::Text("one".into()), 1);
        let (so_far, rx) = b.follow(&a);
        assert_eq!(
            so_far.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(),
            ["one"]
        );
        b.push(
            &a,
            ChatRole::User,
            ChatKind::Text,
            "two".into(),
            String::new(),
            2,
        );
        let e = rx.try_recv().unwrap();
        assert_eq!(
            (e.entry_id, e.text.as_str(), e.role),
            (2, "two", ChatRole::User as i32)
        );
        assert!(rx.try_recv().is_err());
        assert!(b.snapshot(&AgentId::new("other")).is_empty());
    }

    #[test]
    fn the_ring_is_bounded_by_count_and_bytes_and_an_entry_by_size() {
        let b = ChatBoard::new();
        let a = AgentId::new("a");
        for i in 0..(MAX_ENTRIES + 20) {
            b.push(
                &a,
                ChatRole::Agent,
                ChatKind::Text,
                format!("m{i}"),
                String::new(),
                0,
            );
        }
        let s = b.snapshot(&a);
        assert_eq!(s.len(), MAX_ENTRIES);
        assert_eq!(s[0].text, "m20", "oldest dropped first");
        let big = "é".repeat(MAX_ENTRY_BYTES); // two bytes each: the cut must land on a character boundary
        b.push(&a, ChatRole::Agent, ChatKind::Text, big, String::new(), 0);
        let last = b.snapshot(&a).pop().unwrap();
        assert!(last.text.len() <= MAX_ENTRY_BYTES && last.text.chars().all(|c| c == 'é'));
        let c = AgentId::new("c");
        for _ in 0..40 {
            b.push(
                &c,
                ChatRole::Agent,
                ChatKind::Text,
                "x".repeat(MAX_ENTRY_BYTES),
                String::new(),
                0,
            );
        }
        assert!(b.snapshot(&c).iter().map(|e| e.text.len()).sum::<usize>() <= MAX_BYTES);
    }

    #[test]
    fn items_become_entries_with_their_kind() {
        let (k, t, p) = describe(&ChatItem::Diff {
            path: "a.rs".into(),
            unified_diff: "@@".into(),
        });
        assert_eq!((k, t.as_str(), p.as_str()), (ChatKind::Diff, "@@", "a.rs"));
        let (k, t, _) = describe(&ChatItem::ToolCall {
            tool: "run \"x\"".into(),
            arguments_json: "{\"a\":1}".into(),
        });
        assert_eq!(k, ChatKind::ToolCall);
        assert_eq!(t, "{\"tool\":\"run \\\"x\\\"\",\"arguments\":{\"a\":1}}");
        let (k, t, _) = describe(&ChatItem::Plan(vec!["a".into(), "b".into()]));
        assert_eq!((k, t.as_str()), (ChatKind::Plan, "a\nb"));
    }
}
