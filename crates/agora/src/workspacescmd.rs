//! ADR-0064 decision 3: `agora workspaces` lists the workspaces this machine's node made for its agents (place, state, why it stays,
//! size, how long it has been free, branch), and `agora workspaces clean [--older-than 7d] [--dry-run]` removes the ones nothing would
//! be lost with and no agent uses. Both work on the node's own record (`agora_node::workspaces`) in its data folder, also while the node
//! runs: who is where comes from its session ledger and, when it runs, from the node itself (`agent.list`). A workspace that holds work
//! is never removed here: it is listed with its reason.
//!
//! Each listed line is fixed tokens for scripts: `PATH<TAB>STATE<TAB>REASON<TAB>SIZE<TAB>FREE-FOR<TAB>BRANCH`, `-` where none.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use agora_i18n::key;
use agora_node::control::Client;
use agora_node::workspaces::{Action, Clean, Listed, State, Users, Workspaces};
use agora_protocol::v1::{self, command_ok::Result as Ok_, command_request::Command as Cmd};

use crate::{home, out_line, socket_path, Opts, Say};

fn paths(o: &Opts) -> (PathBuf, PathBuf) {
    (
        o.data.clone().unwrap_or_else(|| home().join("data")),
        o.worktrees
            .clone()
            .unwrap_or_else(|| home().join("worktrees")),
    )
}

/// `7d`, `12h`, `30m`, or a bare number of days, as milliseconds.
pub(crate) fn parse_age(s: &str) -> Option<u64> {
    let s = s.trim();
    let (n, unit) = match s.char_indices().last()? {
        (i, 'd') => (&s[..i], 24 * 60 * 60 * 1000),
        (i, 'h') => (&s[..i], 60 * 60 * 1000),
        (i, 'm') => (&s[..i], 60 * 1000),
        _ => (s, 24 * 60 * 60 * 1000),
    };
    n.parse::<u64>().ok()?.checked_mul(unit)
}

/// A size as a short token: bytes in K, M or G (powers of 1024), `+` after a lower bound.
fn size_token(bytes: u64, truncated: bool) -> String {
    let (n, unit) = match bytes {
        b if b >= 1 << 30 => (b as f64 / (1u64 << 30) as f64, "G"),
        b if b >= 1 << 20 => (b as f64 / (1u64 << 20) as f64, "M"),
        b => (b as f64 / 1024.0, "K"),
    };
    format!("{n:.1}{unit}{}", if truncated { "+" } else { "" })
}

/// How long a workspace has been free, as `2d`, `5h` or `12m`.
fn age_token(ms: u64) -> String {
    let m = ms / 60_000;
    match m {
        m if m >= 48 * 60 => format!("{}d", m / (24 * 60)),
        m if m >= 60 => format!("{}h", m / 60),
        m => format!("{m}m"),
    }
}

fn line(w: &Listed, now_ms: u64) -> String {
    let reason = match w.state {
        State::LeftBehind(r) => r.code(),
        _ => "-",
    };
    format!(
        "{}\t{}\t{reason}\t{}\t{}\t{}",
        w.path.display(),
        w.state.code(),
        size_token(w.size_bytes, w.size_truncated),
        w.free_since_ms
            .map_or_else(|| "-".to_string(), |f| age_token(now_ms.saturating_sub(f))),
        w.branch
    )
}

/// What the node says runs where, when it runs. `Ok(None)`: no node listens; `Err`: one does and could not be asked.
fn node_agents(socket: &Path) -> Result<Option<v1::AgentListResult>, ()> {
    let mut c = match Client::connect(socket) {
        Ok(c) => c,
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Ok(None)
        }
        Err(_) => return Err(()),
    };
    match c.call(Cmd::AgentList(v1::AgentList::default())) {
        Ok(Ok(v1::CommandOk {
            result: Some(Ok_::AgentList(list)),
        })) => Ok(Some(list)),
        _ => Err(()),
    }
}

/// Who is where: the ledger's agents, and those the node says run (its own list is the newer). Anything that cannot be read makes
/// every workspace count as in use.
fn users(data: &Path, socket: &Path) -> Users {
    let Ok(entries) = agora_node::ledger::read_entries(data) else {
        return Users::unknown();
    };
    let mut u = Users::from_ledger(&entries);
    match node_agents(socket) {
        Ok(Some(list)) => {
            for a in list.agents.iter().filter(|a| a.running) {
                u.busy(Path::new(&a.worktree_id));
            }
            for i in &list.interrupted {
                u.busy(Path::new(&i.folder));
            }
        }
        Ok(None) => {}
        Err(()) => return Users::unknown(),
    }
    u
}

/// `agora workspaces`.
pub(crate) fn list(say: &Say, o: &Opts) -> ExitCode {
    let (data, worktrees) = paths(o);
    let w = Workspaces::at(&data, &worktrees);
    if agora_node::ledger::read_entries(&data).is_err() {
        say.err(key!("cli-workspaces-ledger-unreadable"), &[]);
        return ExitCode::from(1);
    }
    let now = agora_node::grants::now_ms();
    // As the node's pass sees it: a workspace in its grace period is `waiting` (`agora workspaces clean` removes it all the same).
    let listed = w.list(&users(&data, &socket_path(o)), now, true);
    if listed.is_empty() {
        say.out(key!("cli-workspaces-empty"), &[]);
        return ExitCode::SUCCESS;
    }
    for l in &listed {
        if !out_line(&line(l, now)) {
            return ExitCode::SUCCESS;
        }
    }
    let cleanable = listed
        .iter()
        .filter(|l| matches!(l.state, State::Cleanable | State::Waiting { .. }))
        .count();
    let total: u64 = listed.iter().map(|l| l.size_bytes).sum();
    say.out(
        key!("cli-workspaces-summary"),
        &[
            ("count", (listed.len() as i64).into()),
            ("size", size_token(total, false).into()),
            ("cleanable", (cleanable as i64).into()),
        ],
    );
    ExitCode::SUCCESS
}

/// `agora workspaces clean [--older-than AGE] [--dry-run]`.
pub(crate) fn clean(say: &Say, o: &Opts) -> ExitCode {
    let older_than_ms = match o.older_than.as_deref().map(parse_age) {
        Some(None) => {
            say.err(key!("cli-bad-arguments"), &[]);
            return ExitCode::from(2);
        }
        Some(Some(ms)) => Some(ms),
        None => None,
    };
    let (data, worktrees) = paths(o);
    if agora_node::ledger::read_entries(&data).is_err() {
        say.err(key!("cli-workspaces-ledger-unreadable"), &[]);
        return ExitCode::from(1);
    }
    let socket = socket_path(o);
    if node_agents(&socket).is_err() {
        // A node runs and cannot be asked who works where: nothing is removed.
        say.err(key!("cli-workspaces-node-unasked"), &[]);
        return ExitCode::from(1);
    }
    let w = Workspaces::at(&data, &worktrees);
    let how = Clean {
        by_node: false,
        older_than_ms,
        dry_run: o.dry_run,
    };
    let never = std::sync::atomic::AtomicBool::new(false);
    let outcomes = w.clean(
        &|| users(&data, &socket),
        agora_node::grants::now_ms(),
        how,
        &never,
    );
    let (mut removed, mut kept) = (0i64, 0i64);
    for out in &outcomes {
        let path = ("path", out.path.display().to_string().into());
        match &out.action {
            Action::Removed => {
                removed += 1;
                say.out(key!("cli-workspaces-removed"), &[path]);
            }
            Action::WouldRemove => {
                removed += 1;
                say.out(key!("cli-workspaces-would-remove"), &[path]);
            }
            Action::Forgotten => say.out(key!("cli-workspaces-forgotten"), &[path]),
            Action::Kept(State::LeftBehind(r)) => {
                kept += 1;
                say.out(
                    key!("cli-workspaces-kept"),
                    &[path, ("reason", r.code().into())],
                );
            }
            Action::Kept(_) | Action::BuildOutputRemoved(_) => {}
        }
    }
    say.out(
        if o.dry_run {
            key!("cli-workspaces-clean-dry-summary")
        } else {
            key!("cli-workspaces-clean-summary")
        },
        &[("removed", removed.into()), ("kept", kept.into())],
    );
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_read_days_hours_and_minutes() {
        assert_eq!(parse_age("7d"), Some(7 * 24 * 3_600_000));
        assert_eq!(parse_age("12h"), Some(12 * 3_600_000));
        assert_eq!(parse_age("30m"), Some(30 * 60_000));
        assert_eq!(parse_age("2"), Some(2 * 24 * 3_600_000));
        for bad in ["", "d", "-1d", "7w", "1.5d"] {
            assert_eq!(parse_age(bad), None, "{bad}");
        }
    }

    #[test]
    fn sizes_and_ages_are_short_tokens() {
        assert_eq!(size_token(512, false), "0.5K");
        assert_eq!(size_token(3 << 20, true), "3.0M+");
        assert_eq!(size_token(5 << 30, false), "5.0G");
        assert_eq!(age_token(59 * 60_000), "59m");
        assert_eq!(age_token(5 * 3_600_000), "5h");
        assert_eq!(age_token(3 * 24 * 3_600_000), "3d");
    }
}
