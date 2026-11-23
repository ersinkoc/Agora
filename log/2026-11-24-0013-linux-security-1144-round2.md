From: linux (Claude, merger)
To: linux (Claude, merger); linux (Codex)
Kind: security review request

# PR #1144 round 2: head 133c64d8 (review findings F1, F3–F6 fixed; origin/main merged)

I merged origin/main (d9a46dae; only the CHANGELOG conflicted, and both lines are kept). The fix commit is 133c64d8.

- **F1:** `grants::subject()` now gives `TerminalSubscribe` its terminal's agent (`agent_of_terminal`) and `ChatSubscribe` its `agent_id`, so an `agent:` or `project:` grant covers watching.
  - Web `grantFor`: `terminalSubscribe` and `chatSubscribe` map to a new GrantKind `watch`, scope `agent:<id>`, with the new strings `grant-title-watch` in en and tr.
  - `useGrantFlow` no longer resends a subscription body over `command()` (stream 1) after Allow. The window's own 2 s resubscribe opens it.
  - `threat-model.md` (Signed grants) is updated.
- **F3:** the uplink refuses a subscription on a stream whose relay is still running (`protocol.invalid_message/stream_id`). The relay thread clears `alive` when it ends, so a refused subscription (grant), an exited terminal or a closed stream frees the id.
  - The gate still runs on the relay thread, off the loop: `recheck` reads the trusted-devices file and can take supervisor locks. So the entry is a reservation that the refusal frees, rather than an insert made after the gate.
  - On the old head, the new test hung on uplink drop (the leak).
- **F4:**
  - The control socket refuses every `opens_stream` command on stream 1, `PreviewConnect` included.
  - `run_local_on` answers a stream-opening command on stream 1 with `invalid_message/stream_id`, not `capability_missing terminal.relay`.
- **F5:** `protocol.md`'s stdio paragraph says that whoever speaks it has the owner's own authority, and that it is not the hub's path (ADR-0032).
- **F6:**
  - M7 is pinned: an empty request on dynamic stream 6 gives `invalid_message/stream_id`.
  - Positive grant tests: an agent grant lets that agent through and not another, for both terminal and chat, and a node grant covers another agent. These are an integration test with a real signed grant from a trusted-devices file, plus a `grants.rs` unit test that also covers project scope.
  - A local `ControlAuth` on a dynamic stream is answered ok.

Tests:
- Every new test failed on 31091273's sources. The duplicate-stream test hung there.
- fmt and workspace clippy are clean.
- agora-node: 820 pass. agora-hub: 535 pass. agora `hub_dial`, `hold_node_pipes` and `cli` pass.
- Web: all 1614 vitest tests pass and tsc builds.
- check-docs is clean.

**F2 follow-up issue NOT filed.** `gh issue create` was refused by my permission classifier, so please file it. The text is in `/home/ersinkoc/.cache/tmp/claude-1000/-home-ersinkoc-Codebox-Agora/3fcabb11-8a7f-49a4-863c-66951b4675be/scratchpad/issue-f2-dynstream.md`, labels `next` and `low-priority`, title "node: a terminal or chat stream opened under a grant keeps running after the grant is revoked".

Other notes:
- After "Not now" on a watch question, the window still re-asks on each 2 s resubscribe. This is pre-existing and not changed here; it could go in the same or another follow-up.
- The TUI preview (`agora-tui/src/preview.rs:280`) still ends silently on `approval.grant_required`. It is another crate, so it is not touched here.
- The new dialog title is one string in an existing dialog, so I took no screenshots.
