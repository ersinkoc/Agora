//! A terminal agent's own session id, read from the session file the agent writes (ADR-0060, manifest `[session.file]`). Used for
//! agents whose id the node cannot choose (Codex: no flag sets it), so that Resume after a node restart can continue the conversation.
//!
//! A file is the agent's when it was written after the agent started, its first line passes `select`, names the agent's folder as its
//! working directory, and its id is no other agent's. This is a guess that can be wrong, so it is made only when it cannot be confused.
//! Two agents waiting in one folder are rivals when they are of one kind, or of kinds whose globs could match the same files (the fixed
//! directory of one glob, before its first `*`, is the other's or holds it), unless both `select`s ask for different literal values at
//! the same path. This is decided from the manifests, before any file is read: rivals get no id at all, so kinds that share a session
//! folder lose Resume in that folder rather than risk continuing the wrong conversation. Neither does an agent for which more than one
//! new file fits (the window then offers Start again). An agent that has no file after [`GIVE_UP_AFTER`] gets none.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use agora_adapters::manifest::SessionFile;
use agora_core::AgentId;
use serde_json::Value;

/// How often the files are looked at while an agent waits for its id.
const SCAN_EVERY: Duration = Duration::from_secs(3);
/// How long an agent waits for its session file (the tool may write it only once it has been asked something).
pub const GIVE_UP_AFTER: Duration = Duration::from_secs(30 * 60);
/// A file may have been written this long before the node noticed the agent.
const SLACK: Duration = Duration::from_secs(2);
/// Longest first line read.
const FIRST_LINE_MAX: u64 = 64 * 1024;
/// Longest id accepted; it is passed to the agent's `[resume]` argv, so only plain characters.
const MAX_ID: usize = 128;

/// A running terminal agent without a session id whose kind names its session file.
#[derive(Debug, Clone)]
pub struct Target {
    pub agent: AgentId,
    pub kind: String,
    pub folder: PathBuf,
    pub spec: SessionFile,
}

struct Waiting {
    folder: PathBuf,
    since: SystemTime,
    give_up_at: Instant,
    next_scan: Instant,
}

pub struct SessionWatcher {
    home: Option<PathBuf>,
    waiting: HashMap<AgentId, Waiting>,
    /// Agents that will get no id from a file (ambiguous, or none came in time), until they end.
    gave_up: HashSet<AgentId>,
}

impl Default for SessionWatcher {
    fn default() -> Self {
        Self::new(crate::usage_files::home_dir())
    }
}

fn at<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(v, |v, k| v.get(k))
}

/// Whether a session id may be put on an agent's command line (`[resume]` argv, `[session] id_args`): it starts with an ASCII letter or
/// digit (so it can never be read as an option such as `--dangerously-bypass-approvals-and-sandbox`), then only letters, digits, `.`,
/// `_` and `-`, at most 128 characters. The one check every id passes before it reaches an argv (ADR-0060). An adapter's id travels in
/// JSON (`session/load`) and need not pass it, but it never goes on a command line unless it does.
pub fn argv_safe(id: &str) -> bool {
    id.len() <= MAX_ID
        && id.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// A UUID in its usual spelling (8-4-4-4-12 hex digits): what Codex and Claude Code session ids are. A session file's id must be one.
pub fn is_uuid(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    canon(a) == canon(b)
}

/// `file`'s first line as JSON, when it is complete within [`FIRST_LINE_MAX`].
fn first_line(file: &Path) -> Option<Value> {
    let f = std::fs::File::open(file).ok()?;
    let mut line = String::new();
    // Two bytes of lookahead tell a complete line (`\n` or `\r\n`) from a valid JSON prefix at the cap.
    BufReader::new(f.take(FIRST_LINE_MAX + 2))
        .read_line(&mut line)
        .ok()?;
    // The terminator ends the line and is not part of it (F1181, Codex review of #791): a line of exactly the cap is accepted whether
    // it ends in LF, in CRLF, or is not ended yet.
    let body = line
        .strip_suffix("\r\n")
        .or_else(|| line.strip_suffix('\n'))
        .unwrap_or(&line);
    if body.len() as u64 > FIRST_LINE_MAX {
        return None;
    }
    serde_json::from_str(body.trim_end()).ok()
}

/// Whether a first line passes `spec`'s `select`.
fn selected(v: &Value, spec: &SessionFile) -> bool {
    spec.select
        .as_ref()
        .is_none_or(|sel| at(v, &sel.path).and_then(Value::as_str) == Some(sel.equals.as_str()))
}

/// The id in a first line when that line is a session's start in `folder`.
fn session_of(v: &Value, spec: &SessionFile, folder: &Path) -> Option<String> {
    if !selected(v, spec) {
        return None;
    }
    let cwd = at(v, &spec.cwd)?.as_str()?;
    if !same_dir(Path::new(cwd), folder) {
        return None;
    }
    let id = at(v, &spec.id)?.as_str()?;
    (argv_safe(id) && is_uuid(id)).then(|| id.to_string())
}

/// The id in `file`'s first line when that line is a session's start in `folder`.
fn session_in(file: &Path, spec: &SessionFile, folder: &Path) -> Option<String> {
    session_of(&first_line(file)?, spec, folder)
}

/// The fixed directory part of a session glob: its directories before the first one with a `*` (the file name never counts), with
/// `{home}` replaced, `.` and `..` resolved, and canonicalized when it exists.
fn session_root(glob: &str, home: &Path) -> PathBuf {
    let glob = glob.replacen("{home}", &home.to_string_lossy(), 1);
    let glob = Path::new(&glob);
    let mut root = PathBuf::new();
    for comp in glob.parent().map(Path::components).into_iter().flatten() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                root.pop();
            }
            c if c.as_os_str().to_string_lossy().contains('*') => break,
            c => root.push(c),
        }
    }
    std::fs::canonicalize(&root).unwrap_or(root)
}

/// Whether two agents in one folder could have written each other's session files: they are of one kind, or their globs' fixed
/// directories are the same or one holds the other, unless both `select`s ask for different literal values at the same path.
fn rivals(a: &Target, b: &Target, home: &Path) -> bool {
    if a.kind == b.kind {
        return true;
    }
    if let (Some(x), Some(y)) = (&a.spec.select, &b.spec.select) {
        if x.path == y.path && x.equals != y.equals {
            return false;
        }
    }
    let (ra, rb) = (
        session_root(&a.spec.glob, home),
        session_root(&b.spec.glob, home),
    );
    ra.starts_with(&rb) || rb.starts_with(&ra)
}

impl SessionWatcher {
    pub fn new(home: Option<PathBuf>) -> Self {
        Self {
            home,
            waiting: HashMap::new(),
            gave_up: HashSet::new(),
        }
    }

    /// Looks for the session ids of `targets` (when due) and answers the ones found, each once. `known` are the session ids the node's
    /// agents already have: never given to another.
    pub fn poll(
        &mut self,
        now: Instant,
        targets: &[Target],
        known: &HashSet<String>,
    ) -> Vec<(AgentId, String)> {
        let live: HashSet<&AgentId> = targets.iter().map(|t| &t.agent).collect();
        self.waiting.retain(|a, _| live.contains(a));
        self.gave_up.retain(|a| live.contains(a));
        let Some(home) = self.home.clone() else {
            return Vec::new();
        };
        for t in targets {
            if self.gave_up.contains(&t.agent) {
                continue;
            }
            self.waiting
                .entry(t.agent.clone())
                .or_insert_with(|| Waiting {
                    folder: t.folder.clone(),
                    since: SystemTime::now(),
                    give_up_at: now + GIVE_UP_AFTER,
                    next_scan: now,
                });
        }
        let mut found = Vec::new();
        let mut claimed: HashSet<String> = known.clone();
        for t in targets {
            let Some(w) = self.waiting.get(&t.agent) else {
                continue;
            };
            if now >= w.give_up_at {
                self.waiting.remove(&t.agent);
                self.gave_up.insert(t.agent.clone());
                continue;
            }
            if now < w.next_scan {
                continue;
            }
            // Rivals in this folder could have written each other's files: none of them gets one. Decided from the manifests alone,
            // before any file is read, so the answer is the same in every target order. A live agent can still write its file after
            // discovery gave up on it, so agents that gave up count too.
            let rivals: Vec<AgentId> = targets
                .iter()
                .filter(|other| {
                    other.agent != t.agent
                        && same_dir(&other.folder, &w.folder)
                        && rivals(t, other, &home)
                })
                .map(|other| other.agent.clone())
                .collect();
            if !rivals.is_empty() {
                for a in rivals.into_iter().chain([t.agent.clone()]) {
                    self.waiting.remove(&a);
                    self.gave_up.insert(a);
                }
                continue;
            }
            let since = w.since.checked_sub(SLACK).unwrap_or(SystemTime::UNIX_EPOCH);
            let glob = t.spec.glob.replacen("{home}", &home.to_string_lossy(), 1);
            let mut ids: Vec<String> = crate::usage_files::expand(Path::new(&glob))
                .into_iter()
                .filter(|p| {
                    std::fs::metadata(p)
                        .and_then(|m| m.created().or_else(|_| m.modified()))
                        .is_ok_and(|t| t >= since)
                })
                .filter_map(|p| session_in(&p, &t.spec, &t.folder))
                .filter(|id| !claimed.contains(id))
                .collect();
            ids.sort();
            ids.dedup();
            if let Some(w) = self.waiting.get_mut(&t.agent) {
                w.next_scan = now + SCAN_EVERY;
            }
            match ids.as_slice() {
                [] => {}
                [one] => {
                    claimed.insert(one.clone());
                    self.waiting.remove(&t.agent);
                    found.push((t.agent.clone(), one.clone()));
                }
                _ => {
                    self.waiting.remove(&t.agent);
                    self.gave_up.insert(t.agent.clone());
                }
            }
        }
        found
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use agora_adapters::manifest::UsageSelect;

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("agora-sessions-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(p.join("home/.tool/s")).unwrap();
        std::fs::create_dir_all(p.join("work/a")).unwrap();
        std::fs::create_dir_all(p.join("work/b")).unwrap();
        p
    }

    fn spec() -> SessionFile {
        SessionFile {
            glob: "{home}/.tool/s/rollout-*.jsonl".into(),
            select: Some(UsageSelect {
                path: "type".into(),
                equals: "session_meta".into(),
            }),
            id: "payload.id".into(),
            cwd: "payload.cwd".into(),
        }
    }

    fn write(root: &Path, name: &str, id: &str, cwd: &Path) {
        write_typed(root, name, "session_meta", id, cwd);
    }

    fn write_typed(root: &Path, name: &str, ty: &str, id: &str, cwd: &Path) {
        let line = serde_json::json!({"type": ty, "payload": {"id": id, "cwd": cwd}});
        std::fs::write(
            root.join("home/.tool/s").join(name),
            format!("{line}\n{{\"type\":\"other\"}}\n"),
        )
        .unwrap();
    }

    fn target(agent: &str, folder: PathBuf) -> Target {
        Target {
            agent: AgentId::new(agent),
            kind: "tool".into(),
            folder,
            spec: spec(),
        }
    }

    #[test]
    fn the_new_file_of_the_agents_folder_names_its_session_once() {
        let root = tmp("one");
        let mut w = SessionWatcher::new(Some(root.join("home")));
        let t = vec![target("agent-1", root.join("work/a"))];
        let now = Instant::now();
        assert!(w.poll(now, &t, &HashSet::new()).is_empty());
        // Another folder's session, a line that is not a session start, and an id with characters argv must not get are not its.
        write(&root, "rollout-1.jsonl", "other-id", &root.join("work/b"));
        write(&root, "rollout-2.jsonl", "bad id;rm", &root.join("work/a"));
        write(
            &root,
            "rollout-3.jsonl",
            "0199aaaa-0000-4000-8000-000000000001",
            &root.join("work/a"),
        );
        let found = w.poll(now + SCAN_EVERY, &t, &HashSet::new());
        assert_eq!(
            found,
            vec![(
                AgentId::new("agent-1"),
                "0199aaaa-0000-4000-8000-000000000001".to_string()
            )]
        );
        // The node now knows it as that agent's: it is not offered again (the agent is no longer a target either).
        let known: HashSet<String> = ["0199aaaa-0000-4000-8000-000000000001".to_string()].into();
        assert!(w.poll(now + SCAN_EVERY * 2, &t, &known).is_empty());
        assert!(w.poll(now + SCAN_EVERY * 3, &[], &known).is_empty());
    }

    #[test]
    fn a_planted_file_can_never_put_an_option_on_the_agents_command_line() {
        let long = "a".repeat(129);
        let bad = [
            "--dangerously-bypass-approvals-and-sandbox",
            "-x",
            "a b",
            "a;b",
            "ünïcode-0000-4000-8000-000000000001",
            "",
            long.as_str(),
            // Argv-safe, but not a UUID: a session file's id must be one.
            "abc",
        ];
        for id in bad {
            let root = tmp("planted");
            let mut w = SessionWatcher::new(Some(root.join("home")));
            let t = vec![target("agent-1", root.join("work/a"))];
            write(&root, "rollout-1.jsonl", id, &root.join("work/a"));
            assert!(
                w.poll(Instant::now(), &t, &HashSet::new()).is_empty(),
                "{id:?}"
            );
        }
        for id in &bad[..7] {
            assert!(!argv_safe(id), "{id:?}");
        }
        assert!(argv_safe("0199aaaa-0000-4000-8000-000000000001") && argv_safe("ses_9.x-Y"));
        assert!(argv_safe(&"a".repeat(128)));
    }

    #[test]
    fn two_agents_in_one_folder_or_two_fitting_files_get_no_session() {
        let root = tmp("two");
        let mut w = SessionWatcher::new(Some(root.join("home")));
        let now = Instant::now();
        let twins = vec![
            target("agent-1", root.join("work/a")),
            target("agent-2", root.join("work/a")),
        ];
        write(
            &root,
            "rollout-1.jsonl",
            "0199aaaa-0000-4000-8000-000000000011",
            &root.join("work/a"),
        );
        assert!(w.poll(now, &twins, &HashSet::new()).is_empty());
        assert!(w.poll(now + SCAN_EVERY, &twins, &HashSet::new()).is_empty());
        // Alone in another folder, but two new files fit: no guess either.
        let mut w = SessionWatcher::new(Some(root.join("home")));
        write(
            &root,
            "rollout-2.jsonl",
            "0199aaaa-0000-4000-8000-000000000012",
            &root.join("work/b"),
        );
        write(
            &root,
            "rollout-3.jsonl",
            "0199aaaa-0000-4000-8000-000000000013",
            &root.join("work/b"),
        );
        let alone = vec![target("agent-3", root.join("work/b"))];
        assert!(w.poll(now, &alone, &HashSet::new()).is_empty());
        // A session another agent already has is never given again.
        let mut w = SessionWatcher::new(Some(root.join("home")));
        let known: HashSet<String> = ["0199aaaa-0000-4000-8000-000000000012".to_string()].into();
        assert_eq!(
            w.poll(now, &alone, &known),
            vec![(
                AgentId::new("agent-3"),
                "0199aaaa-0000-4000-8000-000000000013".to_string()
            )]
        );
    }

    #[test]
    fn live_agents_whose_discovery_gave_up_keep_a_newcomers_session_ambiguous() {
        for reverse in [false, true] {
            let root = tmp(if reverse { "late-reversed" } else { "late" });
            let mut w = SessionWatcher::new(Some(root.join("home")));
            let now = Instant::now();
            let mut targets = vec![
                target("older-a", root.join("work/a")),
                target("older-b", root.join("work/a")),
            ];
            // Both older agents give up before a third agent starts or a file exists.
            assert!(w.poll(now, &targets, &HashSet::new()).is_empty());
            let (release, gate) = std::sync::mpsc::channel();
            let writer_root = root.clone();
            let writer = std::thread::spawn(move || {
                gate.recv().unwrap();
                write(
                    &writer_root,
                    "rollout-late.jsonl",
                    "0199aaaa-0000-4000-8000-000000000091",
                    &writer_root.join("work/a"),
                );
            });
            targets.push(target("newcomer", root.join("work/a")));
            if reverse {
                targets.reverse();
            }
            assert!(w
                .poll(now + SCAN_EVERY, &targets, &HashSet::new())
                .is_empty());
            // Release the older agent's file only after the newcomer is registered.
            release.send(()).unwrap();
            writer.join().unwrap();
            assert!(w
                .poll(now + SCAN_EVERY * 2, &targets, &HashSet::new())
                .is_empty());
        }
    }

    #[test]
    fn a_live_timed_out_agent_still_owns_its_possible_session_until_it_leaves() {
        let root = tmp("expired-live");
        let mut w = SessionWatcher::new(Some(root.join("home")));
        let now = Instant::now();
        let mut targets = vec![target("expired", root.join("work/a"))];
        assert!(w.poll(now, &targets, &HashSet::new()).is_empty());
        let expired = now + GIVE_UP_AFTER;
        assert!(w.poll(expired, &targets, &HashSet::new()).is_empty());
        targets.push(target("newcomer", root.join("work/a")));
        assert!(w
            .poll(expired + SCAN_EVERY, &targets, &HashSet::new())
            .is_empty());
        write(
            &root,
            "rollout-late.jsonl",
            "0199aaaa-0000-4000-8000-000000000092",
            &root.join("work/a"),
        );
        assert!(w
            .poll(expired + SCAN_EVERY * 2, &targets, &HashSet::new())
            .is_empty());
        // Leaving the target set clears give-up state; a later unambiguous agent can discover its file.
        assert!(w
            .poll(expired + SCAN_EVERY * 3, &[], &HashSet::new())
            .is_empty());
        std::fs::remove_file(root.join("home/.tool/s/rollout-late.jsonl")).unwrap();
        let fresh = [target("fresh", root.join("work/a"))];
        assert!(w
            .poll(expired + SCAN_EVERY * 4, &fresh, &HashSet::new())
            .is_empty());
        write(
            &root,
            "rollout-fresh.jsonl",
            "0199aaaa-0000-4000-8000-000000000093",
            &root.join("work/a"),
        );
        assert_eq!(
            w.poll(expired + SCAN_EVERY * 5, &fresh, &HashSet::new()),
            vec![(
                fresh[0].agent.clone(),
                "0199aaaa-0000-4000-8000-000000000093".into()
            )]
        );
    }

    #[test]
    fn agents_of_two_kinds_reading_the_same_session_files_in_one_folder_get_no_session() {
        // F1180: a user's manifest copied from another kind names the same session files; a file cannot say which agent wrote it.
        for reverse in [false, true] {
            let root = tmp(if reverse { "kinds-reversed" } else { "kinds" });
            let mut w = SessionWatcher::new(Some(root.join("home")));
            let now = Instant::now();
            let mut targets = vec![
                target("tool-1", root.join("work/a")),
                Target {
                    kind: "tool-variant".into(),
                    ..target("variant-1", root.join("work/a"))
                },
            ];
            if reverse {
                targets.reverse();
            }
            let (release, gate) = std::sync::mpsc::channel();
            let writer_root = root.clone();
            let writer = std::thread::spawn(move || {
                gate.recv().unwrap();
                write(
                    &writer_root,
                    "rollout-1.jsonl",
                    "0199aaaa-0000-4000-8000-000000000095",
                    &writer_root.join("work/a"),
                );
            });
            assert!(w.poll(now, &targets, &HashSet::new()).is_empty());
            // The file appears only after both agents are registered.
            release.send(()).unwrap();
            writer.join().unwrap();
            assert!(w
                .poll(now + SCAN_EVERY, &targets, &HashSet::new())
                .is_empty());
            // In different folders each still gets its own.
            let mut w = SessionWatcher::new(Some(root.join("home")));
            targets[1].folder = root.join("work/b");
            let (a, b) = (targets[0].agent.clone(), targets[1].agent.clone());
            write(
                &root,
                "rollout-2.jsonl",
                "0199aaaa-0000-4000-8000-000000000096",
                &root.join("work/b"),
            );
            let mut found = w.poll(now, &targets, &HashSet::new());
            found.sort();
            let mut want = vec![
                (a, "0199aaaa-0000-4000-8000-000000000095".to_string()),
                (b, "0199aaaa-0000-4000-8000-000000000096".to_string()),
            ];
            want.sort();
            assert_eq!(found, want);
        }
    }

    #[test]
    fn agents_whose_globs_are_spelled_differently_but_match_the_same_file_get_no_session() {
        // Codex review of #791: `sessions/*.jsonl` and `sessions/./*.jsonl` name the same files.
        for reverse in [false, true] {
            let root = tmp(if reverse {
                "spelling-reversed"
            } else {
                "spelling"
            });
            let mut w = SessionWatcher::new(Some(root.join("home")));
            let now = Instant::now();
            let mut targets = vec![
                target("tool-1", root.join("work/a")),
                Target {
                    kind: "tool-variant".into(),
                    spec: SessionFile {
                        glob: "{home}/.tool/s/./rollout-*.jsonl".into(),
                        ..spec()
                    },
                    ..target("variant-1", root.join("work/a"))
                },
            ];
            if reverse {
                targets.reverse();
            }
            let (release, gate) = std::sync::mpsc::channel();
            let writer_root = root.clone();
            let writer = std::thread::spawn(move || {
                gate.recv().unwrap();
                write(
                    &writer_root,
                    "rollout-1.jsonl",
                    "0199aaaa-0000-4000-8000-000000000097",
                    &writer_root.join("work/a"),
                );
            });
            assert!(w.poll(now, &targets, &HashSet::new()).is_empty());
            // The file appears only after both agents are registered.
            release.send(()).unwrap();
            writer.join().unwrap();
            assert!(w
                .poll(now + SCAN_EVERY, &targets, &HashSet::new())
                .is_empty());
            // Neither takes it later either, even once the other agent has left.
            let alone = [targets[0].clone()];
            assert!(w
                .poll(now + SCAN_EVERY * 2, &alone, &HashSet::new())
                .is_empty());
        }
    }

    #[test]
    fn agents_reading_one_glob_with_exclusive_selectors_in_one_folder_each_get_their_own() {
        // Codex review of #791: no first line can pass both `select`s, so each file can only be one of them.
        for reverse in [false, true] {
            let root = tmp(if reverse {
                "selectors-reversed"
            } else {
                "selectors"
            });
            let mut w = SessionWatcher::new(Some(root.join("home")));
            let now = Instant::now();
            let mut targets = vec![
                target("tool-1", root.join("work/a")),
                Target {
                    kind: "tool-variant".into(),
                    spec: SessionFile {
                        select: Some(UsageSelect {
                            path: "type".into(),
                            equals: "variant_meta".into(),
                        }),
                        ..spec()
                    },
                    ..target("variant-1", root.join("work/a"))
                },
            ];
            if reverse {
                targets.reverse();
            }
            let (release, gate) = std::sync::mpsc::channel();
            let writer_root = root.clone();
            let writer = std::thread::spawn(move || {
                gate.recv().unwrap();
                let folder = writer_root.join("work/a");
                write_typed(
                    &writer_root,
                    "rollout-1.jsonl",
                    "session_meta",
                    "0199aaaa-0000-4000-8000-000000000098",
                    &folder,
                );
                write_typed(
                    &writer_root,
                    "rollout-2.jsonl",
                    "variant_meta",
                    "0199aaaa-0000-4000-8000-000000000099",
                    &folder,
                );
            });
            assert!(w.poll(now, &targets, &HashSet::new()).is_empty());
            release.send(()).unwrap();
            writer.join().unwrap();
            let mut found = w.poll(now + SCAN_EVERY, &targets, &HashSet::new());
            found.sort();
            let mut want = vec![
                (
                    AgentId::new("tool-1"),
                    "0199aaaa-0000-4000-8000-000000000098".to_string(),
                ),
                (
                    AgentId::new("variant-1"),
                    "0199aaaa-0000-4000-8000-000000000099".to_string(),
                ),
            ];
            want.sort();
            assert_eq!(found, want);
        }
    }

    fn with_glob(t: Target, kind: &str, glob: &str) -> Target {
        Target {
            kind: kind.into(),
            spec: SessionFile {
                glob: glob.into(),
                ..spec()
            },
            ..t
        }
    }

    /// Polls `targets` once to register them, writes `files` (name relative to the home, id, folder) only then, and polls again.
    fn found_after_gated_write(
        root: &Path,
        targets: &[Target],
        files: &[(&str, &str, &str)],
    ) -> Vec<(AgentId, String)> {
        let mut w = SessionWatcher::new(Some(root.join("home")));
        let now = Instant::now();
        let (release, gate) = std::sync::mpsc::channel::<()>();
        let writer_root = root.to_path_buf();
        let files: Vec<(String, String, String)> = files
            .iter()
            .map(|(n, i, f)| (n.to_string(), i.to_string(), f.to_string()))
            .collect();
        let writer = std::thread::spawn(move || {
            gate.recv().unwrap();
            for (name, id, folder) in files {
                let path = writer_root.join("home").join(&name);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                let line = serde_json::json!({"type": "session_meta",
                    "payload": {"id": id, "cwd": writer_root.join(folder)}});
                std::fs::write(path, format!("{line}\n")).unwrap();
            }
        });
        assert!(w.poll(now, targets, &HashSet::new()).is_empty());
        release.send(()).unwrap();
        writer.join().unwrap();
        let mut found = w.poll(now + SCAN_EVERY, targets, &HashSet::new());
        found.sort();
        found
    }

    #[test]
    fn agents_whose_globs_differ_under_one_session_root_get_no_session() {
        // Codex review of #791: `*/*.jsonl` and `*/rollout-*.jsonl` under one root can both match a file.
        for reverse in [false, true] {
            let root = tmp(if reverse {
                "one-root-reversed"
            } else {
                "one-root"
            });
            let mut targets = vec![
                with_glob(
                    target("tool-1", root.join("work/a")),
                    "tool",
                    "{home}/.tool/s/*/rollout-*.jsonl",
                ),
                with_glob(
                    target("variant-1", root.join("work/a")),
                    "tool-variant",
                    "{home}/.tool/s/*/*.jsonl",
                ),
            ];
            if reverse {
                targets.reverse();
            }
            let found = found_after_gated_write(
                &root,
                &targets,
                &[(
                    ".tool/s/2026/rollout-1.jsonl",
                    "0199aaaa-0000-4000-8000-000000000101",
                    "work/a",
                )],
            );
            assert_eq!(found, vec![]);
        }
    }

    #[test]
    fn agents_whose_session_roots_are_nested_get_no_session() {
        // One root inside the other: whether a file matches both depends on names the manifests do not fix, so neither gets one,
        // even for a file only one glob matches.
        for reverse in [false, true] {
            let root = tmp(if reverse { "nested-reversed" } else { "nested" });
            let mut targets = vec![
                with_glob(
                    target("tool-1", root.join("work/a")),
                    "tool",
                    "{home}/.tool/s/rollout-*.jsonl",
                ),
                with_glob(
                    target("variant-1", root.join("work/a")),
                    "tool-variant",
                    "{home}/.tool/s/sub/*.jsonl",
                ),
            ];
            if reverse {
                targets.reverse();
            }
            let found = found_after_gated_write(
                &root,
                &targets,
                &[
                    (
                        ".tool/s/rollout-1.jsonl",
                        "0199aaaa-0000-4000-8000-000000000102",
                        "work/a",
                    ),
                    (
                        ".tool/s/sub/other-1.jsonl",
                        "0199aaaa-0000-4000-8000-000000000103",
                        "work/a",
                    ),
                ],
            );
            assert_eq!(found, vec![]);
        }
    }

    #[test]
    fn agents_whose_session_roots_are_disjoint_each_get_their_own() {
        for reverse in [false, true] {
            let root = tmp(if reverse {
                "disjoint-reversed"
            } else {
                "disjoint"
            });
            let mut targets = vec![
                with_glob(
                    target("tool-1", root.join("work/a")),
                    "tool",
                    "{home}/.tool/s/rollout-*.jsonl",
                ),
                with_glob(
                    target("variant-1", root.join("work/a")),
                    "tool-variant",
                    "{home}/.other/s/rollout-*.jsonl",
                ),
            ];
            if reverse {
                targets.reverse();
            }
            let found = found_after_gated_write(
                &root,
                &targets,
                &[
                    (
                        ".tool/s/rollout-1.jsonl",
                        "0199aaaa-0000-4000-8000-000000000104",
                        "work/a",
                    ),
                    (
                        ".other/s/rollout-1.jsonl",
                        "0199aaaa-0000-4000-8000-000000000105",
                        "work/a",
                    ),
                ],
            );
            assert_eq!(
                found,
                vec![
                    (
                        AgentId::new("tool-1"),
                        "0199aaaa-0000-4000-8000-000000000104".to_string()
                    ),
                    (
                        AgentId::new("variant-1"),
                        "0199aaaa-0000-4000-8000-000000000105".to_string()
                    ),
                ]
            );
        }
    }

    #[test]
    fn who_gets_a_session_does_not_depend_on_target_order() {
        // Codex review of #791: the answer was decided while targets were walked, so their (HashMap) order could change it.
        let mut results = Vec::new();
        for reverse in [false, true] {
            let root = tmp(if reverse { "order-reversed" } else { "order" });
            let mut targets = vec![
                with_glob(
                    target("tool-1", root.join("work/a")),
                    "tool",
                    "{home}/.tool/s/rollout-*.jsonl",
                ),
                with_glob(
                    target("variant-1", root.join("work/a")),
                    "tool-variant",
                    "{home}/.tool/s/sub/*.jsonl",
                ),
                with_glob(
                    target("other-1", root.join("work/a")),
                    "other",
                    "{home}/.other/s/rollout-*.jsonl",
                ),
            ];
            if reverse {
                targets.reverse();
            }
            results.push(found_after_gated_write(
                &root,
                &targets,
                &[
                    (
                        ".tool/s/rollout-1.jsonl",
                        "0199aaaa-0000-4000-8000-000000000106",
                        "work/a",
                    ),
                    (
                        ".tool/s/sub/variant-1.jsonl",
                        "0199aaaa-0000-4000-8000-000000000107",
                        "work/a",
                    ),
                    (
                        ".other/s/rollout-1.jsonl",
                        "0199aaaa-0000-4000-8000-000000000108",
                        "work/a",
                    ),
                ],
            ));
        }
        assert_eq!(results[0], results[1]);
        assert_eq!(
            results[0],
            vec![(
                AgentId::new("other-1"),
                "0199aaaa-0000-4000-8000-000000000108".to_string()
            )]
        );
    }

    #[test]
    fn a_crlf_first_line_counts_like_an_lf_one_at_the_read_limit() {
        // Codex review of #791: the terminator is not part of the line, whether it is `\n` or `\r\n`.
        let root = tmp("crlf-limit");
        let folder = root.join("work/a");
        let id = "0199aaaa-0000-4000-8000-000000000100";
        let file = root.join("home/.tool/s/rollout-crlf.jsonl");
        let line =
            serde_json::json!({"type":"session_meta","payload":{"id":id,"cwd":folder}}).to_string();
        for length in [FIRST_LINE_MAX - 1, FIRST_LINE_MAX, FIRST_LINE_MAX + 1] {
            for end in ["\n", "\r\n"] {
                let mut padded = line.as_bytes().to_vec();
                padded.resize(length as usize, b' ');
                padded.extend_from_slice(end.as_bytes());
                padded.extend_from_slice(b"{\"type\":\"other\"}\r\n");
                std::fs::write(&file, &padded).unwrap();
                assert_eq!(
                    session_in(&file, &spec(), &folder),
                    (length <= FIRST_LINE_MAX).then(|| id.to_string()),
                    "{length} {end:?}"
                );
            }
        }
        // A carriage return that is not followed by a newline is part of an overlong line.
        let mut padded = line.as_bytes().to_vec();
        padded.resize(FIRST_LINE_MAX as usize, b' ');
        padded.extend_from_slice(b"\rx\n");
        std::fs::write(&file, &padded).unwrap();
        assert!(session_in(&file, &spec(), &folder).is_none());
    }

    #[test]
    fn a_session_first_line_must_fit_completely_within_the_read_limit() {
        let root = tmp("line-limit");
        let folder = root.join("work/a");
        let id = "0199aaaa-0000-4000-8000-000000000094";
        let file = root.join("home/.tool/s/rollout-limit.jsonl");
        let line =
            serde_json::json!({"type":"session_meta","payload":{"id":id,"cwd":folder}}).to_string();
        for length in [FIRST_LINE_MAX - 1, FIRST_LINE_MAX, FIRST_LINE_MAX + 1] {
            let mut padded = line.as_bytes().to_vec();
            padded.resize(length as usize, b' ');
            std::fs::write(&file, &padded).unwrap();
            assert_eq!(
                session_in(&file, &spec(), &folder),
                (length <= FIRST_LINE_MAX).then(|| id.to_string())
            );
        }
        // The prefix at the limit is valid JSON, but the complete physical line is not.
        let mut malformed = line.as_bytes().to_vec();
        malformed.resize(FIRST_LINE_MAX as usize, b' ');
        malformed.extend_from_slice(b"NOT_JSON\n");
        assert!(serde_json::from_slice::<Value>(&malformed).is_err());
        std::fs::write(&file, malformed).unwrap();
        assert!(session_in(&file, &spec(), &folder).is_none());
        // A bounded first line is accepted even when later lines are large.
        std::fs::write(
            &file,
            format!("{line}\n{}", "x".repeat(FIRST_LINE_MAX as usize * 2)),
        )
        .unwrap();
        assert_eq!(session_in(&file, &spec(), &folder), Some(id.to_string()));
    }
}
