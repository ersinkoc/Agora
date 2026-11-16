# PR #629: node identity security cross-review

Reviewed head: `7ae30408a2b49df1c04d96c16db5e2eeae46a9ec`.
Verdict: **CHANGES NEEDED**. The ingestion and rebuilt snapshot protection is incomplete for historical client replay.

## P2: old forged bodies still reach reconnecting clients unchanged

`View::apply` now ignores a node's `AgentForgotten` and derives resource/checkout ownership from the producer, making a freshly rebuilt snapshot safe. However, `Hub::replay` still decodes the original stored payload and the server sends it as a `PositionedEvent`. `apps/web/src/net/view.ts::applyEvent` trusts those original body fields for agent forgetting, resources and checkout ownership. A reconnecting unscoped client can therefore still apply the cross-node forgery that a new connection's snapshot correctly ignores.

This requires forged records persisted before the upgrade; the new session rejects equivalent freshly received records. It is not a claim of bypassing the new live-ingestion guard. Protecting replay of such prior stored history is an explicit part of this PR's defense-in-depth scope.

The proof seeds an in-memory store with legitimate `n1`/`n2` records through position 3, then three historically accepted `n2` records: `AgentForgotten("n1/a")`, `NodeResources(node_id="n1", agents_running=99)` and `CheckoutDiscovered(node_id="n1", checkout_id="forged")`. It opens the actual hub, takes a fresh snapshot over real loopback WebSockets, and reconnects from position 3 over another real connection. Hub startup writes its legitimate offline events for old clients. Those events are included in the replay and do not repair these corruptions.

The captured protobuf frames are then passed to the unchanged real browser reducer, using TypeScript stripping and only rewriting import locations. Protocol JS is generated from this checkout's schema; reducer logic and schema fixtures are not copied or weakened. At **identical final position 10**:

| State | Fresh snapshot | Resumed client |
| --- | --- | --- |
| `n1/a` visible | yes | **no** |
| `n1` agent count | 0 | **99** |
| machine of `n2/forged` checkout | n2 | **n1** |

Native transport regression fails with exit 101 after exporting all frames; the real browser regression independently fails with exit 1. The safe fresh snapshot is a positive control for the intended projection fix. No real agents, accounts, user data stores or credentials are involved; the token is the existing integration fixture.

Prevent unsafe historical event bodies from being delivered to client reducers: for example force an appropriate clean snapshot at the replay boundary, or validate/canonicalize delivery consistently with the trusted producer. Preserve append-only stored history and legitimate hub-authored `NodeOffline`/forget events. Include reconnects before and after the unsafe records in the regression: a client that already applied an old forgery may need a clean snapshot too. A client-only patch does not protect old clients still connected through the hub.

## Checks already passing

- Existing hub suite: **270 passed, 4 ignored, 0 failed**.
- Formatting, docs checker and hub all-target Clippy with warnings denied pass on restored product source.
- Reported required GitHub CI checks are green at the reviewed head, including native Rust jobs on all three OSes. Local proof execution is Linux only.
- `named_node` exhaustively lists all 33 current event variants; the seven ordinary node-id bodies and three hub-only event bodies match the schema. Existing guard tests exercise wrong/empty/differently-cased node ids, own-node controls and the hub-only restrictions. Producer mismatch validation precedes storage.
- Projection producer qualification, hub-only `NodeLink`/`NodeForgotten`/`AgentForgotten`, `brings_back` identity, and the special legitimate hub-authored `NodeOffline` path were inspected. The existing tests covering these paths pass. The remaining defect is raw client replay bypassing that projection.

## Reproduction

Run in an isolated checkout containing these evidence files:

```sh
cargo test -p agora-hub
REVIEW_629_FRAMES=/tmp/review-629-wire python3 docs/reviews/0629-run-replay-proof.py
# Expected above: exit 101, after frames have been written.
cd apps/web
npm ci
cd ../..
sh scripts/gen-ts.sh
node --disable-warning=ExperimentalWarning docs/reviews/0629-web-replay-proof.mjs /tmp/review-629-wire
# Expected browser assertion failure: exit 1.
git diff --exit-code -- apps crates schema
```

The runner restores the existing server test source in `finally`. The review used existing installed web dependencies through a temporary worktree-local symlink; no new dependency or source change is committed. Only evidence files are stored on this review branch. No implementation fix or merge is performed by this review.
