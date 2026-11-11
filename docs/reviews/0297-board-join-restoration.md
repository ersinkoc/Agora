# Restore saved boards before join import (#297)

Board restoration used to happen only on the first mesh tool call. If a node's join
response arrived first after reopening the hub, import created a fresh board and
persisted it over the saved task graph and live leases.

`Hub::board_joined` now uses the same lazy restoration as `Hub::board_call`, before
importing local work. Both callers hold the hub lock. An initialized board is never
reloaded, so a lagging best-effort storage write cannot roll back newer memory state.
Existing storage-error and invalid-snapshot handling are unchanged. No schema,
protocol, node transfer, dependency or membership policy changes are needed.

## Evidence

`crates/agora-hub/tests/board_join_restore.rs` exercises the public Hub API with an
in-memory event store. A saved prerequisite, dependent task and live README lease
are restored before a second node imports work. The test checks stable old task IDs,
distinct imported IDs, preserved dependencies, blocked dependent claims, refusal of
the conflicting lease with its original holder/expiry, and the persisted result.

The same test first initializes the board in a second scenario, creates and claims
new work, then deliberately leaves an older snapshot in storage. Joining must keep
the newer in-memory state. No actual accounts, agents, credentials or services run.

- Before the fix, only the imported task remains: one task instead of three.
- With the fix, both scenarios pass.
- Temporarily forcing storage reload makes the claimed prerequisite revert to open;
  the test fails. The mutation was restored before final validation.
- Full workspace tests (including documentation tests), Clippy with warnings denied,
  formatting, locale lint and documentation checks pass locally.

#291 separately fixes selected-folder membership and transition discovery. #293
separately bounds node-side transfers. This fix addresses saved board restoration
only, without claiming to solve every transfer lifecycle case. Independent review
and maintainer merge remain; the installed hub/node and active session are untouched.
