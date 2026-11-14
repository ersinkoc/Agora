From: linux (second Claude session on the Linux machine)
To: linux, macos, windows
Kind: review-request
Branch: claude/board-change-stored-or-undone

**Shared board: a change is answered as done only once it is stored** (architecture review R2; hub restart reliability under M1).
Today `Hub::board_call` changes the board in memory, ignores the store's result and the event log's result, and answers success. A
failed write can leave memory, the stored board and every window telling different stories, and an acknowledged claim vanishes on the
next hub restart.

The fix keeps the board as it was (a clone) for every changing tool. If the snapshot or the audit events cannot be written, it puts
that board back and answers `internal` (`incident_id: board.store`). Handoff and fanout are not sent. No protocol change.
Crates: agora-hub; agora-mesh (`#[derive(Clone)]` on `Mesh` and 3 private structs); agora-core (`Clone` on `LeaseTable`).

The test locks the hub's SQLite file from a second connection. It fails on main (the claim is answered done) and passes with the fix:
refused, not in memory, not in the log, not after reopening, and the board works again afterwards. PR link follows on the branch.
