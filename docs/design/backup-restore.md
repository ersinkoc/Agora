# Backup and restore

- **What needs backup:** hub event store (coordination facts, approvals, audit, mesh, users, devices,
  policies). Runtime facts (processes, files, git state) are re-reported by nodes on reconnect.
- **How:** `agora hub backup` performs an online, consistent backup (SQLite backup API / pg_dump in team
  mode); scheduled backups with retention; optional encryption with a user-held key.
- **Restore:** `agora hub restore <file>`; nodes reconnect and resync; events newer than the backup that
  nodes still hold are replayed.
- **Node side:** session references and the local event buffer allow agents to be resumed after a node
  restart; worktrees are ordinary git and need no special backup.
- **Disaster drill:** restore is tested in CI with a scripted multi-node scenario.

## What a restore brings back, what it loses, and how long it takes

**Brought back by the machines.** Everything a machine produced (agents started and ended, terminals, folders and checkouts,
approvals it asked and answered, usage) is replayed by that machine when it reconnects, as long as its event buffer still holds it.
Each machine keeps up to 64 MiB of events the hub has not acknowledged (`EVENT_BUFFER_BYTES`). On the maintainer's hub a day of
work was about 1.2 MB of events (26 hours, 5 machines; see performance.md), so the buffer covers on the order of weeks of such work.
Past that, the oldest events are dropped: the machine resynchronizes, its present (what runs, what is open) is restated, and the
missing stretch is recorded as a lost range (`resyncs`, shown on the Activity page). History in that range is gone; the present is not.

**Lost: what only the hub held after the backup.** These come back as they were at the backup:
- **machines enrolled after the backup are refused**: the restored hub never issued their certificates. Enroll them again
  (`agora hub code`, then `agora node enroll` on that machine) and restart their node;
- people, roles, sign-in sessions, passkeys and registered devices added after the backup (sign in again where a session is refused);
- the audit rows written after the backup;
- the shared board as it was at the backup (tasks and leases created or changed later), and grants recorded on the hub later;
- machines forgotten after the backup (ADR-0073) are listed again until they are forgotten again.

A backup taken often keeps this window short: `agora hub backup` (online, safe while the hub runs) from cron or a timer.

**How long it takes.** In the drill, the restored hub's present is whole 0.46–0.67 s after it starts, on one Linux machine: the
machines redial on their own (0.5 s, 1 s, then 1.5 s apart) and replay what the backup missed. **Proposed recovery target, for the
maintainer to confirm:** the present is whole within 5 s of the hub starting, for machines that can reach it; history inside a
machine's buffer is not lost. The drill does not assert a time yet.

## Implementation status
`agora-hub::store`: `SqliteStore::backup_to` (online, consistent, refuses to overwrite), `verify` (integrity check and
schema version) and `restore` (verifies first, keeps the replaced file as `<db>.before-restore`, drops stale WAL files).
A test restores an older backup and replays the gap from the node's side. CLI: `agora hub backup [FILE] [--keep N]` (without FILE: `<hub dir>/backups/hub-<unix ms>.db`, newest N kept, default 7;
a given FILE is never overwritten) and `agora hub restore FILE` (stop the hub first). Not yet: running backups on a schedule
from the hub itself (use cron or a timer for now) and encryption.

**The multi-node drill** (`crates/agora/tests/restore_drill.rs`, in CI with the other tests; architecture review R8) runs real processes:
a hub, two enrolled machines with a shell agent each, `agora hub backup` while it runs, more work after the backup (a second agent on
one machine, the other machine's agent stopped, a third machine enrolled), the hub killed, `agora hub restore` of the older backup, and the hub started again on
its old address. It checks each hop: the restored file really is the older state; both machines reconnect by themselves; the agent
started after the backup is running and the stopped one has ended; and no history range is recorded as lost, because the machines
still held every event after the backup; and the machine enrolled after the backup is refused until it enrolls again. On Linux the hub's present is whole 0.5–0.7 s after it starts from the backup. When a machine
has already dropped events the restored hub never had, the loss is recorded as a range instead (`resyncs`, ADR-0054); that case is
`hub_link.rs`'s `a_hub_restored_from_an_older_backup_gets_the_gap_replayed`. Board work of agents still running survives a resync
(ADR-0055, `a_resync_keeps_a_running_agents_shared_board_work`).
