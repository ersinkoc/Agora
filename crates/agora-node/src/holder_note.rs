//! A note the node leaves about the holder it is connected to (ADR-0093 "Build status"): the link version it agreed with it.
//!
//! `agora doctor` and `agora status` are not the node, so the holder does not answer them (it admits only the node holding the role
//! lock, `docs/specs/holder-link.md`). They cannot ask it its version, and they must not attach to it. The node already knows the
//! answer from its `Hello`; it writes it to `<home>/run/hold.link` (owner-only) and the readers check that the holder it names still
//! runs. Nothing here touches the holder or any agent's data.

use std::path::{Path, PathBuf};

use crate::holder_link::LINK_VERSION;

/// `<home>/run/hold.link`: `version pid start` on one line.
pub fn path(home: &Path) -> PathBuf {
    home.join("run").join("hold.link")
}

/// Records the version agreed with the holder `pid` (started at `start`). Best effort: a note that cannot be written only means that
/// doctor stays silent.
pub fn write(home: &Path, version: u16, pid: u32, start: u64) {
    let _ =
        agora_platform::write_private(&path(home), format!("{version} {pid} {start}\n").as_bytes());
}

/// A running holder that speaks only link version 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OldHolder {
    pub pid: u32,
    /// How many agents it keeps (`run/hold.pid`), 0 when that cannot be read.
    pub kept: usize,
}

/// The holder of `home` when the node's note says it speaks only version 1 and that process still runs (same pid and start time).
/// `None` for no note, a newer holder, an ended holder or a stale note. Read only.
pub fn old_holder(home: &Path) -> Option<OldHolder> {
    if !agora_platform::HOLDER_BUILT {
        return None;
    }
    let text = std::fs::read_to_string(path(home)).ok()?;
    let mut w = text.split_whitespace();
    let (version, pid, start): (u16, u32, u64) = (
        w.next()?.parse().ok()?,
        w.next()?.parse().ok()?,
        w.next()?.parse().ok()?,
    );
    if version != LINK_VERSION || agora_platform::process_start_time(pid) != Some(start) {
        return None;
    }
    Some(OldHolder {
        pid,
        kept: kept(home, pid).unwrap_or(0),
    })
}

/// Agents the holder `pid` keeps, from its own `run/hold.pid` (`pid start agents`).
fn kept(home: &Path, pid: u32) -> Option<usize> {
    let text = std::fs::read_to_string(home.join("run").join("hold.pid")).ok()?;
    let mut w = text.split_whitespace();
    let p: u32 = w.next()?.parse().ok()?;
    let _start = w.next()?;
    let n: usize = w.next()?.parse().ok()?;
    (p == pid).then_some(n)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn home(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("agora-holder-note-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("run")).unwrap();
        d
    }

    #[test]
    fn only_a_running_version_1_holder_is_old() {
        if !agora_platform::HOLDER_BUILT {
            return;
        }
        let h = home("old");
        let me = std::process::id();
        let start = agora_platform::process_start_time(me).unwrap();
        // No note.
        assert_eq!(old_holder(&h), None);
        // Version 1 and running (this process stands in for the holder), keeping 2 agents.
        write(&h, 1, me, start);
        std::fs::write(h.join("run/hold.pid"), format!("{me} {start} 2\n")).unwrap();
        assert_eq!(old_holder(&h), Some(OldHolder { pid: me, kept: 2 }));
        // Version 2: nothing to say.
        write(&h, 2, me, start);
        assert_eq!(old_holder(&h), None);
        // Version 1 but a different start time (a recycled pid) or garbage.
        write(&h, 1, me, start + 1);
        assert_eq!(old_holder(&h), None);
        std::fs::write(path(&h), "x\n").unwrap();
        assert_eq!(old_holder(&h), None);
        let _ = std::fs::remove_dir_all(&h);
    }
}
