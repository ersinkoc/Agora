//! ADR-0130 (with Amendment A): accepted-task notices to a project's channel. The owner names a channel per Review-enabled shared
//! project; when a report there is accepted, the hub keeps a notice (the task, the accepted revision and when, the channel; never the
//! dispatcher or the reviewer) in its settings store, in the acceptance's own transaction, and records a content-free hint
//! (`ProjectNoticesChanged`) so an entitled window reads the page again. Notices are rows, not events: the limits (50 per project
//! channel, 10,000 on the hub) and the 30-day expiry really remove them, and every page is read with today's access.

use agora_protocol::v1;
use prost::Message;
use sha2::{Digest, Sha256};

use crate::store::{EventStore, SqliteStore, StoreError};

const SETTINGS_PREFIX: &str = "project_notices.settings.";
const NOTICE_PREFIX: &str = "project_notices.notice.";
/// The most notices kept for one project's channel, and on the hub, and how long one is kept.
pub const PER_CHANNEL: usize = 50;
pub const ON_HUB: usize = 10_000;
pub const KEEP_MS: u64 = 30 * 24 * 3_600_000;
/// The most notices one page gives.
pub const PAGE_MAX: u32 = 50;

/// A channel as a setting names it: `#` and 1 to 32 of `[a-z0-9_-]`.
pub fn valid_channel(channel: &str) -> bool {
    channel.strip_prefix('#').is_some_and(|n| {
        (1..=32).contains(&n.len())
            && n.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
    })
}

fn settings_key(project: &str) -> String {
    format!("{SETTINGS_PREFIX}{project}")
}

/// A project's part of a notice key: the start of the SHA-256 of its id, so no project's keys are a prefix of another's (an id may hold
/// `/` or glob characters; review of e0300a93).
fn project_part(project: &str) -> String {
    Sha256::digest(project.as_bytes())[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The settings key prefix of one project's notices (the store adds the `*`).
fn notices_glob(project: &str) -> String {
    format!("{NOTICE_PREFIX}{}.", project_part(project))
}

fn notice_key(project: &str, notice_id: &str) -> String {
    format!("{NOTICE_PREFIX}{}.{notice_id}", project_part(project))
}

/// The owner's settings for `project` (`dispatcher`, `channel`); defaults (off) when none were saved.
pub fn settings(
    store: &SqliteStore,
    project: &str,
) -> Result<v1::ProjectNoticesSettings, StoreError> {
    let saved = store
        .setting(&settings_key(project))?
        .and_then(|b| v1::ProjectNoticesSettings::decode(b.as_slice()).ok())
        .unwrap_or_default();
    Ok(v1::ProjectNoticesSettings {
        project_id: project.to_string(),
        dispatcher: saved.dispatcher,
        channel: saved.channel,
        active: false,
        inactive_reason: String::new(),
    })
}

/// Saves the owner's settings for `project`. `Err(field)` for a channel that is not one (`#` and 1 to 32 of `[a-z0-9_-]`).
pub fn set_settings(
    store: &SqliteStore,
    set: &v1::ProjectNoticesSettingsSet,
    project: &str,
) -> Result<Result<(), &'static str>, StoreError> {
    if !set.channel.is_empty() && !valid_channel(&set.channel) {
        return Ok(Err("channel"));
    }
    let saved = v1::ProjectNoticesSettings {
        project_id: project.to_string(),
        dispatcher: set.dispatcher,
        channel: set.channel.clone(),
        active: false,
        inactive_reason: String::new(),
    };
    store.set_setting(&settings_key(project), &saved.encode_to_vec())?;
    Ok(Ok(()))
}

/// The stable id of the notice of `revision` of `task` in `project` to `channel`: a retry of the same acceptance never makes a second.
pub fn notice_id(project: &str, task: &str, revision: u64, channel: &str) -> String {
    let mut h = Sha256::new();
    for part in [
        project.as_bytes(),
        task.as_bytes(),
        &revision.to_be_bytes(),
        channel.as_bytes(),
    ] {
        h.update((part.len() as u64).to_be_bytes());
        h.update(part);
    }
    h.finalize()[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The notice to keep with an acceptance (its settings key and bytes), when `project`'s channel notices are on.
pub fn pending(
    store: &SqliteStore,
    project: &str,
    task: &str,
    revision: u64,
    accepted_ms: u64,
) -> Result<Option<(String, Vec<u8>)>, StoreError> {
    let s = settings(store, project)?;
    if s.channel.is_empty() {
        return Ok(None);
    }
    let id = notice_id(project, task, revision, &s.channel);
    let notice = v1::ProjectNotice {
        notice_id: id.clone(),
        project_id: project.to_string(),
        task_id: task.to_string(),
        report_revision: revision,
        accepted_ms,
        channel: s.channel,
        created_ms: accepted_ms,
    };
    Ok(Some((notice_key(project, &id), notice.encode_to_vec())))
}

fn all_of(store: &SqliteStore, glob: &str) -> Result<Vec<v1::ProjectNotice>, StoreError> {
    Ok(store
        .settings_with_prefix(glob)?
        .iter()
        .filter_map(|b| v1::ProjectNotice::decode(b.as_slice()).ok())
        .collect())
}

/// `project`'s notices only (the key already separates them; the id is checked again).
fn of_project(store: &SqliteStore, project: &str) -> Result<Vec<v1::ProjectNotice>, StoreError> {
    Ok(all_of(store, &notices_glob(project))?
        .into_iter()
        .filter(|n| n.project_id == project)
        .collect())
}

/// Removes what the limits and the expiry no longer keep: notices older than 30 days, then the oldest beyond 50 of `project`'s channel,
/// then the oldest beyond 10,000 on the hub. Never touches the board.
pub fn prune(store: &SqliteStore, project: &str, now_ms: u64) -> Result<(), StoreError> {
    let mut mine = of_project(store, project)?;
    mine.sort_by(|a, b| (b.created_ms, &b.notice_id).cmp(&(a.created_ms, &a.notice_id)));
    for (i, n) in mine.iter().enumerate() {
        if i >= PER_CHANNEL || now_ms.saturating_sub(n.created_ms) > KEEP_MS {
            store.remove_setting(&notice_key(&n.project_id, &n.notice_id))?;
        }
    }
    let mut all = all_of(store, NOTICE_PREFIX)?;
    if all.len() > ON_HUB {
        all.sort_by(|a, b| (a.created_ms, &a.notice_id).cmp(&(b.created_ms, &b.notice_id)));
        for n in &all[..all.len() - ON_HUB] {
            store.remove_setting(&notice_key(&n.project_id, &n.notice_id))?;
        }
    }
    Ok(())
}

/// One page of `project`'s notices, oldest first after the cursor, at most `limit` (1 to 50), and whether more follow. Expired notices
/// are not given.
pub fn page(
    store: &SqliteStore,
    project: &str,
    req: &v1::ProjectNoticesPage,
    now_ms: u64,
) -> Result<v1::ProjectNoticesPageResult, StoreError> {
    let limit = req.limit.clamp(1, PAGE_MAX) as usize;
    let mut mine: Vec<v1::ProjectNotice> = of_project(store, project)?
        .into_iter()
        .filter(|n| now_ms.saturating_sub(n.created_ms) <= KEEP_MS)
        .filter(|n| {
            (n.created_ms, n.notice_id.as_str())
                > (req.after_created_ms, req.after_notice_id.as_str())
        })
        .collect();
    mine.sort_by(|a, b| (a.created_ms, &a.notice_id).cmp(&(b.created_ms, &b.notice_id)));
    let more = mine.len() > limit;
    mine.truncate(limit);
    Ok(v1::ProjectNoticesPageResult {
        notices: mine,
        more,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn store(name: &str) -> (SqliteStore, std::path::PathBuf) {
        let path =
            std::env::temp_dir().join(format!("agora-notices-{name}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        (SqliteStore::open(&path).unwrap(), path)
    }

    #[test]
    fn a_project_never_sees_another_whose_id_starts_with_its_own() {
        let (s, path) = store("prefix");
        for project in ["a", "a/b"] {
            set_settings(
                &s,
                &v1::ProjectNoticesSettingsSet {
                    project_id: project.into(),
                    dispatcher: false,
                    channel: "#dev".into(),
                },
                project,
            )
            .unwrap()
            .unwrap();
            let (k, v) = pending(&s, project, "t", 1, 10).unwrap().unwrap();
            s.set_setting(&k, &v).unwrap();
        }
        let read = |p: &str| {
            page(
                &s,
                p,
                &v1::ProjectNoticesPage {
                    limit: 50,
                    ..Default::default()
                },
                20,
            )
            .unwrap()
            .notices
        };
        assert_eq!(
            read("a")
                .iter()
                .map(|n| n.project_id.as_str())
                .collect::<Vec<_>>(),
            ["a"]
        );
        prune(&s, "a", 10 + KEEP_MS + 1).unwrap();
        assert!(read("a").is_empty());
        assert_eq!(read("a/b").len(), 1, "pruning a never touches a/b");
        drop(s);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_channel_is_a_short_plain_name() {
        for ok in ["#dev", "#a", "#review_2-x", &format!("#{}", "a".repeat(32))] {
            assert!(valid_channel(ok), "{ok}");
        }
        for bad in [
            "dev",
            "#",
            "#Dev",
            "#dev ops",
            "#é",
            &format!("#{}", "a".repeat(33)),
        ] {
            assert!(!valid_channel(bad), "{bad}");
        }
    }

    #[test]
    fn notices_are_kept_once_bounded_paged_and_expired() {
        let (s, path) = store("keep");
        assert!(
            pending(&s, "p", "t1", 1, 10).unwrap().is_none(),
            "off by default"
        );
        let set = |channel: &str| v1::ProjectNoticesSettingsSet {
            project_id: "p".into(),
            dispatcher: false,
            channel: channel.into(),
        };
        assert_eq!(set_settings(&s, &set("dev"), "p").unwrap(), Err("channel"));
        set_settings(&s, &set("#dev"), "p").unwrap().unwrap();
        assert_eq!(settings(&s, "p").unwrap().channel, "#dev");
        // The same acceptance gives the same id.
        let (k1, _) = pending(&s, "p", "t1", 1, 10).unwrap().unwrap();
        assert_eq!(pending(&s, "p", "t1", 1, 99).unwrap().unwrap().0, k1);
        for i in 0..55u64 {
            let (k, v) = pending(&s, "p", &format!("t{i}"), 1, 1000 + i)
                .unwrap()
                .unwrap();
            s.set_setting(&k, &v).unwrap();
        }
        set_settings(
            &s,
            &v1::ProjectNoticesSettingsSet {
                project_id: "q".into(),
                dispatcher: false,
                channel: "#ops".into(),
            },
            "q",
        )
        .unwrap()
        .unwrap();
        let (k, v) = pending(&s, "q", "x", 1, 1000).unwrap().unwrap();
        s.set_setting(&k, &v).unwrap();
        prune(&s, "p", 2000).unwrap();
        let first = page(
            &s,
            "p",
            &v1::ProjectNoticesPage {
                project_id: "p".into(),
                limit: 20,
                ..Default::default()
            },
            2000,
        )
        .unwrap();
        assert_eq!(first.notices.len(), 20);
        assert!(first.more);
        assert_eq!(
            first.notices[0].task_id, "t5",
            "the five oldest went beyond 50"
        );
        let last = first.notices.last().unwrap();
        let rest = page(
            &s,
            "p",
            &v1::ProjectNoticesPage {
                project_id: "p".into(),
                after_created_ms: last.created_ms,
                after_notice_id: last.notice_id.clone(),
                limit: 50,
            },
            2000,
        )
        .unwrap();
        assert_eq!((rest.notices.len(), rest.more), (30, false));
        // Another project's notices are its own; a month later every notice of `p` is gone.
        assert_eq!(
            page(
                &s,
                "q",
                &v1::ProjectNoticesPage {
                    limit: 50,
                    ..Default::default()
                },
                2000
            )
            .unwrap()
            .notices
            .len(),
            1
        );
        prune(&s, "p", 2000 + KEEP_MS + 2000).unwrap();
        assert!(page(
            &s,
            "p",
            &v1::ProjectNoticesPage {
                limit: 50,
                ..Default::default()
            },
            2000
        )
        .unwrap()
        .notices
        .is_empty());
        drop(s);
        let _ = std::fs::remove_file(&path);
    }
}
