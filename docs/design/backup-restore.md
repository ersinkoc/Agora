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
A test restores an older backup and replays the gap from the node's side. Not yet: the `agora hub backup|restore`
commands (they come with the hub binary), scheduled backups with retention, encryption, and the multi-node CI drill.
