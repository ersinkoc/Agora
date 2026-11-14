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

## Implementation status
`agora-hub::store`: `SqliteStore::backup_to` (online, consistent, refuses to overwrite), `verify` (integrity check and
schema version) and `restore` (verifies first, keeps the replaced file as `<db>.before-restore`, drops stale WAL files).
A test restores an older backup and replays the gap from the node's side. CLI: `agora hub backup [FILE] [--keep N]` (without FILE: `<hub dir>/backups/hub-<unix ms>.db`, newest N kept, default 7;
a given FILE is never overwritten) and `agora hub restore FILE` (stop the hub first). Not yet: running backups on a schedule
from the hub itself (use cron or a timer for now) and encryption.

**The multi-node drill** (`crates/agora/tests/restore_drill.rs`, in CI with the other tests; architecture review R8) runs real processes:
a hub, two enrolled machines with a shell agent each, `agora hub backup` while it runs, more work after the backup (a second agent on
one machine, the other machine's agent stopped), the hub killed, `agora hub restore` of the older backup, and the hub started again on
its old address. It checks each hop: the restored file really is the older state; both machines reconnect by themselves; the agent
started after the backup is running and the stopped one has ended; and no history range is recorded as lost, because the machines
still held every event after the backup. On Linux the hub's present is whole 0.5–0.7 s after it starts from the backup. When a machine
has already dropped events the restored hub never had, the loss is recorded as a range instead (`resyncs`, ADR-0054); that case is
`hub_link.rs`'s `a_hub_restored_from_an_older_backup_gets_the_gap_replayed`. Board work of agents still running survives a resync
(ADR-0055, `a_resync_keeps_a_running_agents_shared_board_work`).
