# PR #598: technical cross-review

Reviewed head: `96dc373984cd9cb2ca9158b6a6c6e146a0c4ea8d`.
Verdict: **CHANGES NEEDED**, confirming the [existing Windows review finding](https://github.com/ersinkoc/Agora/pull/598#issuecomment-6894149348), not a new independent finding.

## P2: home-directory username still reaches members and viewers

`scope::short_binary` always keeps the last two nonempty path components. When the executable sits directly in its user's home, the retained folder is the username:

| Reported by node | Delivered to nonowner |
| --- | --- |
| `/home/alice/agora` | `alice/agora` |
| `/Users/alice/agora` | `alice/agora` |
| `C:\Users\alice\agora.exe` | `alice\agora.exe` |

This is a normal off-PATH installation location. The hub is genuinely shortening the path; its shortening rule cannot satisfy the stated username minimization goal for these locations. The shortened string is already present on the wire, so client formatting cannot repair the disclosure.

The attached proof starts the real hub over loopback TCP/WebSockets, uses the existing fake node and authentication helpers, and decodes actual protocol frames. It does not copy the path filter. Across all three fixture paths it checks:

- Owner: original full path in snapshot, live event, and replay (9 passing controls).
- Member and viewer with all-project access: snapshot, live event, and replay.
- Member and viewer with one granted project: filtered snapshot and live event. These clients receive snapshots instead of replay by design.

All 30 nonowner deliveries retain `alice`. The regression assertion fails after all 39 observations, exit 101. This runs on a Linux hub using Windows/macOS/Linux path strings; it does not claim native Windows or macOS process execution. No real agents, credentials, or personal paths are used.

Run in an isolated checkout of the reviewed head containing these evidence files:

```sh
python3 docs/reviews/0598-run-home-proof.py
```

The runner restores the server test source in `finally`. The expected result on the reviewed head is failure. Prefer a filename-only form for nonowners, or a node-generated safe form that knows its actual home directory. Keep owner full-path controls and promote the transport regression when fixing. Align the ADR/spec text with the final privacy rule.

## Other checks

- `cargo test -p agora-protocol -p agora-node -p agora-hub`: **659 passed, 4 ignored, 0 failed**, 44 result groups including doc tests.
- Published golden fixture files are untouched. Old fixtures decode with empty additive fields; the new-field compatibility tests pass. Schema fields use unused tags, with no existing field edits.
- Node `program` is derived with the node's native path semantics; hub/node capability declarations both include `kinds.program`.
- Existing node off-PATH tests cover matching directories, equivalent paths and the fixed empty-PATH-entry case.
- Formatting, the docs checker and Clippy with warnings denied for all three reviewed crates/all targets pass.
- All reported required GitHub CI checks pass at the reviewed head, including the three native Rust OS jobs. Those checks do not cover this direct-home transport case.

Only review evidence is committed here. Production source and existing tests are unchanged. The ADR-0069 amendment's maintainer acceptance/integration hold is independent of this technical verdict; this review does not grant or relay acceptance.
