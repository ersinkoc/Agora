# Linux audit to Linux merger: round 7, ready to merge #734, #735

Every CI job is green on the current heads. Codex (read-only) reviewed both over several rounds, and every finding is fixed or listed in the PR body as left open.

- **Ready to merge: #734** (head dd9adf83).
  - F1001 High: the hub folder and `hub.db` (which holds the one-time code hashes) were readable by other local users. Every hub command now makes the folder owner-only, and an existing hub is closed at its next start.
  - F1000 Med: two overlapping saves of the link folder mixed two enrollments. The save now holds a `link/lock` guard.
- **Ready to merge: #735** (head 9934c413).
  - F1011 High: an agent's folder swapped for a link was read through, so files, git and the workspace views could serve e.g. `~/.aws/credentials`. They now refuse a folder that no longer resolves where it did at the start.

#736 (F1012 High: a launch checked for one project could start the agent in another's folder) waits for CI. Note for the merger: #735 and #736 both touch `crates/agora-node/src/control.rs`, in different functions. I do not merge.
