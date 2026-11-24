//! #1150 (ADR-0092): the folders an ACP registry kind's package runner (`npx`, `uvx`) starts in, so it never starts in the project
//! and reads none of its package-manager configuration (`Supervisor::runner_folder`).
//!
//! Every start gets a new folder of its own, `<root>/<agent id>.<16 random hex digits>`, made with `create_dir` (an existing name
//! fails the start, never reuses it), so nothing a running or earlier agent left anywhere is in it, and nothing an old process of the
//! same agent still writes reaches it. Nothing is walked on the start path: the agent's earlier folders (their names read under the
//! supervisor's lock, at most [`MAX_LISTED`] of the root's entries) are queued for the node's one cleaner thread
//! ([`remove_in_background`]: a start only queues, a name already queued is not queued twice, one thread at a time), which removes
//! them outside the lock with [`remove_bounded`]: iterative (no recursion, so the depth an agent chooses cannot exhaust a stack),
//! relative to held folder handles (no link is followed: `agora_platform::ConfinedDir`), and bounded in depth ([`MAX_DEPTH`], the
//! open folder handles it holds at once) and in entries ([`MAX_ENTRIES`]). A folder it cannot finish is renamed `<name>.left`
//! ([`LEFT_SUFFIX`]), which no later listing picks up, so it is walked once; the node's log says so once, and `agora doctor` counts
//! them ([`left_count`]).

use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::io;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use agora_platform::{not_confined, ConfinedDir, EntryKind};

/// The name ending of a folder [`remove_bounded`] could not finish: left where it is, never walked again.
pub const LEFT_SUFFIX: &str = ".left";

/// The deepest folder [`remove_bounded`] goes into below the one it removes (one open folder handle per level).
pub const MAX_DEPTH: usize = 64;
/// The most entries [`remove_bounded`] looks at for one folder.
pub const MAX_ENTRIES: usize = 50_000;
/// The most entries of the root read to find an agent's earlier folders.
pub const MAX_LISTED: usize = 4096;
/// Entries read from one folder before they are removed and it is read again.
const BATCH: usize = 256;
/// Entries [`remove_bounded`] may read, per entry it may look at.
const READS_PER_ENTRY: usize = 4;

/// How far [`remove_bounded`] may go.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub depth: usize,
    pub entries: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            depth: MAX_DEPTH,
            entries: MAX_ENTRIES,
        }
    }
}

/// What [`remove_bounded`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Removal {
    /// The folder and everything in it is gone (or was already).
    Removed,
    /// A limit was reached, or an entry could not be removed: the rest is left.
    Partial,
}

/// The folder name `<id>.<hex>` belongs to `id`: its part before the last `.`. Plain ids have no `.`.
fn owner(name: &OsStr) -> Option<&str> {
    name.to_str()?.rsplit_once('.').map(|(id, _)| id)
}

/// Makes a new, empty folder for one start of agent `id` below `root` (created when missing) and returns it. `id` must be a plain
/// id (letters, digits, `-`, `_`; the caller checks). A name that exists already fails (`AlreadyExists`), it is never reused.
pub fn new_folder(root: &Path, id: &str) -> io::Result<PathBuf> {
    let mut raw = [0u8; 8];
    getrandom::getrandom(&mut raw).map_err(|e| io::Error::other(e.to_string()))?;
    let hex: String = raw.iter().map(|b| format!("{b:02x}")).collect();
    std::fs::create_dir_all(root)?;
    let dir = root.join(format!("{id}.{hex}"));
    std::fs::create_dir(&dir)?;
    Ok(dir)
}

/// The names of agent `id`'s folders below `root` other than `keep` (the one this start uses), from at most [`MAX_LISTED`] entries.
pub fn earlier_folders(root: &Path, id: &str, keep: &OsStr) -> Vec<OsString> {
    let Ok(held) = ConfinedDir::open_root(root) else {
        return Vec::new();
    };
    let mut seen = 0;
    let mut out = Vec::new();
    let _ = held.visit(|item| {
        seen += 1;
        let left = item.name.to_str().is_some_and(|n| n.ends_with(LEFT_SUFFIX));
        if item.name != keep && !left && owner(&item.name) == Some(id) {
            out.push(item.name);
        }
        if seen >= MAX_LISTED {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    });
    out
}

/// How many folders below `root` were left in part ([`LEFT_SUFFIX`]), from at most [`MAX_LISTED`] entries.
pub fn left_count(root: &Path) -> usize {
    let Ok(held) = ConfinedDir::open_root(root) else {
        return 0;
    };
    let (mut seen, mut left) = (0, 0);
    let _ = held.visit(|item| {
        seen += 1;
        if item.name.to_str().is_some_and(|n| n.ends_with(LEFT_SUFFIX)) {
            left += 1;
        }
        if seen >= MAX_LISTED {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    });
    left
}

/// The node's one cleaner: folders waiting to be removed, and whether its thread runs.
struct Cleaner {
    queue: std::collections::VecDeque<(PathBuf, OsString)>,
    queued: std::collections::BTreeSet<(PathBuf, OsString)>,
    running: bool,
}

static CLEANER: std::sync::Mutex<Cleaner> = std::sync::Mutex::new(Cleaner {
    queue: std::collections::VecDeque::new(),
    queued: std::collections::BTreeSet::new(),
    running: false,
});

/// Queues `names` below `root` for the node's one cleaner thread, which removes them with [`remove_bounded`] (never on the caller's
/// thread: it holds no lock of the caller's and blocks nothing). A name already waiting is not queued again; one thread runs at a
/// time and ends when nothing waits. A folder it cannot finish is renamed `<name>.left` and the node's log says so once.
pub fn remove_in_background(root: PathBuf, names: Vec<OsString>) {
    let mut cleaner = CLEANER.lock().unwrap_or_else(|e| e.into_inner());
    for name in names {
        let key = (root.clone(), name);
        if cleaner.queued.insert(key.clone()) {
            cleaner.queue.push_back(key);
        }
    }
    if cleaner.running || cleaner.queue.is_empty() {
        return;
    }
    cleaner.running = true;
    let spawned = std::thread::Builder::new()
        .name("agora-runner-clean".into())
        .spawn(clean_queue);
    if let Err(e) = spawned {
        cleaner.running = false;
        eprintln!("package runner folders not removed: {e}");
    }
}

fn clean_queue() {
    loop {
        let next = {
            let mut cleaner = CLEANER.lock().unwrap_or_else(|e| e.into_inner());
            match cleaner.queue.pop_front() {
                Some(key) => key,
                None => {
                    cleaner.running = false;
                    return;
                }
            }
        };
        clean_one(&next.0, &next.1);
        CLEANER
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .queued
            .remove(&next);
    }
}

fn clean_one(root: &Path, name: &OsStr) {
    let Ok(held) = ConfinedDir::open_root(root) else {
        return;
    };
    match remove_bounded(&held, name, Limits::default()) {
        Ok(Removal::Removed) => {}
        Ok(Removal::Partial) => {
            // Never walked again: the listing of earlier folders leaves `.left` names out.
            let mut left = name.to_os_string();
            left.push(LEFT_SUFFIX);
            let _ = std::fs::rename(root.join(name), root.join(&left));
            eprintln!("package runner folder left in part: {left:?}");
        }
        Err(e) => eprintln!("package runner folder not removed: {name:?}: {e}"),
    }
}

/// Waits until the cleaner has nothing queued and no thread runs (tests).
#[cfg(test)]
fn wait_idle() {
    let end = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        {
            let c = CLEANER.lock().unwrap_or_else(|e| e.into_inner());
            if !c.running && c.queue.is_empty() {
                return;
            }
        }
        assert!(
            std::time::Instant::now() < end,
            "the cleaner did not finish"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// Removes the folder `name` inside `parent` and what it holds, within `limits`, without recursion and without following a link
/// (each step is relative to a held folder handle; a link is removed as itself). What cannot be reached (deeper than the depth limit,
/// a name the platform refuses to use, an entry that cannot be removed) is skipped and left with the folders above it, and the rest is
/// still removed; once the entry limit is reached it stops. [`Removal::Partial`] when anything was left.
pub fn remove_bounded(parent: &ConfinedDir, name: &OsStr, limits: Limits) -> io::Result<Removal> {
    let top = match parent.subdir(name, false) {
        Ok(dir) => dir,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Removal::Removed),
        // A link or a file where the folder was: removed as itself.
        Err(e) if not_confined(&e) => {
            return Ok(
                remove_as_itself(parent, name).map_or(Removal::Partial, |()| Removal::Removed)
            )
        }
        Err(e) => return Err(e),
    };
    struct Level {
        dir: ConfinedDir,
        /// Its name in the folder before it on the stack (the first one's in `parent`).
        name: OsString,
        /// Entries of it left in place: never looked at again.
        skip: HashSet<OsString>,
    }
    let mut stack = vec![Level {
        dir: top,
        name: name.to_os_string(),
        skip: HashSet::new(),
    }];
    let mut budget = limits.entries;
    // Entries read, skipped ones included (a folder is read again from its start after each batch): bounded too, so a folder full
    // of entries that cannot be removed costs a bounded amount of reading.
    let mut reads = limits.entries.saturating_mul(READS_PER_ENTRY);
    let mut partial = false;
    while let Some(level) = stack.last_mut() {
        let mut batch: Vec<OsString> = Vec::new();
        level.dir.visit(|item| {
            reads = reads.saturating_sub(1);
            if !level.skip.contains(&item.name) {
                batch.push(item.name);
            }
            if batch.len() >= BATCH || reads == 0 {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        })?;
        if reads == 0 {
            return Ok(Removal::Partial);
        }
        if batch.is_empty() {
            // Nothing more to do here: closed first (Windows deletes an open folder only once its handles close), then removed from
            // the folder before it, unless something was left in it.
            let Some(done) = stack.pop() else {
                break;
            };
            let kept = !done.skip.is_empty();
            drop(done.dir);
            let outer = stack.last().map_or(parent, |l| &l.dir);
            let removed = !kept
                && match outer.remove_dir(&done.name) {
                    Ok(()) => true,
                    Err(e) => e.kind() == io::ErrorKind::NotFound,
                };
            if !removed {
                partial = true;
                if let Some(l) = stack.last_mut() {
                    l.skip.insert(done.name);
                }
            }
            continue;
        }
        let mut deeper = None;
        for entry in batch {
            if budget == 0 {
                return Ok(Removal::Partial);
            }
            budget -= 1;
            let kind = match level.dir.entry(&entry) {
                Ok(Some(e)) => e.kind,
                Ok(None) => continue,
                Err(_) => {
                    partial = true;
                    level.skip.insert(entry);
                    continue;
                }
            };
            if kind == EntryKind::Folder {
                deeper = Some(entry);
                break;
            }
            if remove_as_itself(&level.dir, &entry).is_err() {
                partial = true;
                level.skip.insert(entry);
            }
        }
        let Some(entry) = deeper else {
            continue;
        };
        if stack.len() >= limits.depth {
            partial = true;
            if let Some(l) = stack.last_mut() {
                l.skip.insert(entry);
            }
            continue;
        }
        let Some(level) = stack.last_mut() else {
            break;
        };
        match level.dir.subdir(&entry, false) {
            Ok(sub) => stack.push(Level {
                dir: sub,
                name: entry,
                skip: HashSet::new(),
            }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            // Swapped for a link or a file since it was described: removed as itself.
            Err(e) if not_confined(&e) => {
                if remove_as_itself(&level.dir, &entry).is_err() {
                    partial = true;
                    level.skip.insert(entry);
                }
            }
            Err(_) => {
                partial = true;
                level.skip.insert(entry);
            }
        }
    }
    Ok(if partial {
        Removal::Partial
    } else {
        Removal::Removed
    })
}

/// Removes a file, a link (as itself) or anything else that is not a folder; a folder link on Windows is removed as a folder.
fn remove_as_itself(dir: &ConfinedDir, name: &OsStr) -> io::Result<()> {
    match dir.remove_file(name) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => dir.remove_dir(name),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("agora-runner-folders-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A chain of `depth` folders named `d` below `top`, made one level at a time relative to a held handle (no long paths).
    fn chain(top: &Path, depth: usize) {
        let mut dir = ConfinedDir::open_root(top).unwrap();
        for _ in 0..depth {
            dir = dir.subdir(OsStr::new("d"), true).unwrap();
        }
        dir.create_new(OsStr::new("f"), 0o600).unwrap();
    }

    #[test]
    fn each_start_gets_a_new_empty_folder_and_finds_only_its_own_earlier_ones() {
        let root = tmp("names").join("runner");
        let a = new_folder(&root, "agent-1").unwrap();
        let b = new_folder(&root, "agent-1").unwrap();
        let other = new_folder(&root, "agent-12").unwrap();
        assert_ne!(a, b);
        assert_eq!(std::fs::read_dir(&b).unwrap().count(), 0);
        let earlier = earlier_folders(&root, "agent-1", b.file_name().unwrap());
        assert_eq!(earlier, vec![a.file_name().unwrap().to_os_string()]);
        assert!(earlier_folders(&root, "agent-12", other.file_name().unwrap()).is_empty());
        let _ = std::fs::remove_dir_all(root.parent().unwrap());
    }

    #[test]
    fn a_folder_with_files_folders_and_links_is_removed_without_following_a_link() {
        let base = tmp("links");
        let outside = base.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("keep"), "x").unwrap();
        let victim = base.join("v");
        std::fs::create_dir_all(victim.join("a/b")).unwrap();
        std::fs::write(victim.join("a/b/f"), "x").unwrap();
        std::fs::write(victim.join("g"), "x").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, victim.join("a/to-outside")).unwrap();
            std::os::unix::fs::symlink(outside.join("keep"), victim.join("keep-link")).unwrap();
        }
        let held = ConfinedDir::open_root(&base).unwrap();
        assert_eq!(
            remove_bounded(&held, OsStr::new("v"), Limits::default()).unwrap(),
            Removal::Removed
        );
        assert!(!victim.exists());
        assert!(
            outside.join("keep").exists(),
            "a link's target is never touched"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Re-review of #1181: one cleaner for the node; a folder it cannot finish is renamed `.left`, counted, and not picked up again.
    #[test]
    fn a_folder_left_in_part_is_marked_and_never_queued_again() {
        let root = tmp("left").join("runner");
        let first = new_folder(&root, "agent-7").unwrap();
        chain(&first, MAX_DEPTH + 10);
        let plain = new_folder(&root, "agent-7").unwrap();
        std::fs::write(plain.join("f"), "").unwrap();
        let current = new_folder(&root, "agent-7").unwrap();
        let earlier = earlier_folders(&root, "agent-7", current.file_name().unwrap());
        assert_eq!(earlier.len(), 2);
        // Queued twice (two starts at once): still one walk each.
        remove_in_background(root.clone(), earlier.clone());
        remove_in_background(root.clone(), earlier);
        wait_idle();
        assert!(!plain.exists());
        let mut marked = first.clone().into_os_string();
        marked.push(LEFT_SUFFIX);
        assert!(!first.exists() && PathBuf::from(&marked).exists());
        assert_eq!(left_count(&root), 1);
        assert!(earlier_folders(&root, "agent-7", current.file_name().unwrap()).is_empty());
        let held = ConfinedDir::open_root(&root).unwrap();
        let all = Limits {
            depth: 1000,
            entries: 1_000_000,
        };
        let name = PathBuf::from(marked);
        let _ = remove_bounded(&held, name.file_name().unwrap(), all);
        let _ = std::fs::remove_dir_all(root.parent().unwrap());
    }

    /// Review of #1181 (B1): an agent chooses the depth and width of what it leaves in its folder. A chain far deeper than a small
    /// stack could hold one recursive frame per level of is walked without recursion, on a thread with a 64 KiB stack, and stops at
    /// the depth limit (what is deeper stays); a wide folder stops at the entry limit. Neither aborts nor runs on unbounded.
    #[test]
    fn a_deep_chain_and_a_wide_folder_are_bounded_and_need_no_stack() {
        let base = tmp("deep");
        let deep = base.join("deep");
        std::fs::create_dir_all(&deep).unwrap();
        chain(&deep, 400);
        let wide = base.join("wide");
        std::fs::create_dir_all(&wide).unwrap();
        for i in 0..3000 {
            std::fs::write(wide.join(format!("f{i}")), "").unwrap();
        }
        let b = base.clone();
        let (deep_result, wide_result) = std::thread::Builder::new()
            .stack_size(64 * 1024)
            .spawn(move || {
                let held = ConfinedDir::open_root(&b).unwrap();
                let small = Limits {
                    depth: MAX_DEPTH,
                    entries: 1000,
                };
                (
                    remove_bounded(&held, OsStr::new("deep"), Limits::default()).unwrap(),
                    remove_bounded(&held, OsStr::new("wide"), small).unwrap(),
                )
            })
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(deep_result, Removal::Partial, "stopped at the depth limit");
        assert_eq!(wide_result, Removal::Partial, "stopped at the entry limit");
        assert!(std::fs::read_dir(&wide).unwrap().count() >= 2000);
        // Within the limits, the same walk removes all of it (still one handle per level, no recursion).
        let held = ConfinedDir::open_root(&base).unwrap();
        let all = Limits {
            depth: 1000,
            entries: 1_000_000,
        };
        assert_eq!(
            remove_bounded(&held, OsStr::new("deep"), all).unwrap(),
            Removal::Removed
        );
        assert_eq!(
            remove_bounded(&held, OsStr::new("wide"), all).unwrap(),
            Removal::Removed
        );
        assert!(!deep.exists() && !wide.exists());
        let _ = std::fs::remove_dir_all(&base);
    }
}
