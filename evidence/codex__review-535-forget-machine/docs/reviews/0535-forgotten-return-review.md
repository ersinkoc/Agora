# PR #535: forget-machine reconnect review

Reviewed hub head: `5363f1165294bc44db7a08f24286bcc09dedfb95`.
Pending web slice checked: `bafedd89c93fdcabdf4b4d0a4d3a99eff005602c` (#537).
Decision: **changes requested** (one P2: refresh retained state for clients that joined while the node was forgotten).

## Reproduce the returning machine with no agents or folders

1. A node has announced its name, checkout, selected workspace and a running agent. The hub has acknowledged all five node events.
2. Its connection drops. The owner forgets it without refusing return.
3. A new window opens. Its filtered wire snapshot correctly contains none of the forgotten node's records.
4. The same node reconnects normally. The hub's resume asks from node sequence 6: all announcements were already acknowledged, so nothing is replayed. The hub emits **only `NodeLink(up)`** to the window.
5. The hub projection remembers the retained records, but the window never received them. Its actual reducer creates an unnamed online node with zero agents, checkouts and workspaces. A fresh snapshot at the same resulting position contains `laptop`, one running agent and both folder records.

This also affects a browser reloaded while the machine is hidden. A window that stayed open before forgetting can retain hidden data and mask the defect. The existing reconnect test inspects only a new server-side snapshot, so it misses the late subscriber.

The binary fixtures are generated from the actual Hub/NodeSession operations and consumed by the actual `fromSnapshot`/`applyEvent` web reducers. The consistency assertion fails, while a fresh-snapshot control passes. Temporarily substituting #537's reducer yields the same failure; it is not fixed by that slice's NodeForgotten handler. The real web live loop folds these events without requesting another snapshot.

Suggested fix: when a forgotten node returns, refresh affected clients using the existing fresh-snapshot path, or have the client explicitly resync when a returning node lacks retained state. Preserve the node's running agents and normal resume semantics; repeating NodeOnline to force data back would reset agents. No new message shape is required to use the existing snapshot path.

## Run the proof

From the review worktree, with normal web dependencies and generated types:

```sh
cp docs/reviews/0535-forgotten-return-wire-proof.rs crates/agora-hub/tests/review_535.rs
cargo test -p agora-hub --test review_535 -- --nocapture
cp docs/reviews/0535-forgotten-return.test.ts apps/web/src/net/review-535-return.test.ts
npm --prefix apps/web test -- src/net/review-535-return.test.ts
rm crates/agora-hub/tests/review_535.rs apps/web/src/net/review-535-return.test.ts
```

The Rust proof passes while recording `/tmp/agora-535-wire/` binary snapshots and the emitted return event. The web test then fails its consistency assertion (1 failed, 1 control passed).

## Other checks and limits

Owner-only checks pass for both member and viewer roles over real test sockets; the supplemental test fragment is `0535-owner-role-proof.rs`, appended temporarily to the PR's existing `tests/server.rs` to reuse its helpers. The PR's existing online/unknown/idempotence and certificate-revocation tests pass. All named native CI checks are green. Full tracked hub/web test counts and formatting/docs checks are in the JSON.

All temporary source edits/tests were removed, including the #537 reducer substitution. No product changes remain. Return fixtures use synthetic state and an in-process hub; web checks exercise the reducers, not a rendered browser. No real coding agent or deployed service was touched.
