# PR #535: returning-machine resnapshot fix verified

Reviewed head: `863f4488b1a1a43c09fe6ab0696e191ee4fc0546`.
Decision: **approve**. The original late-client P2 is verified fixed. Native CI remains a merge requirement.

On the event that brings a forgotten machine back, the hub records its position, closes current subscriber channels and forces resumes from older positions onto the existing snapshot path. The cutoff is rebuilt from the event log. A normal reconnect of a machine that was never forgotten is unaffected. No message shape or client behavior is added.

## Independent real-socket proof

The supplemental test appended to the existing server test suite uses real loopback TCP/WebSocket connections and its existing synthetic authenticated node fixture. It announces a node name, checkout, selected workspace, running agent and binding, then disconnects and forgets it. A late client receives the empty filtered snapshot at position **9**.

The same node reconnects and resumes from sequence **6**, with no repeated announcements. The late client's socket actually closes (not just an empty channel read). Its next Subscribe from position 9 receives a **Snapshot at position 10**, restoring `laptop`, the running agent, checkout, workspace and binding. The next node event arrives once at position **11**; there is no duplicate. The running-agent record stays running. Rebuilding the copied event store through a fresh `Hub::open` preserves the old-position snapshot rule.

## Existing web reconnect proof

A second test feeds these captured binary snapshots into the real, unchanged `connectLive` implementation with a controllable Socket transport. It opens on the hidden snapshot, receives the server-close signal, reconnects automatically from position 9 and consumes the recovery snapshot. The final view contains the machine name and all retained records, without a reload or session rejection. States are `connecting → live → offline → connecting → live`.

This verifies the existing client resnapshot path that the fix uses; #537 does not need a special NodeLink reducer workaround for this finding. It is not a full review of the rest of #537.

## Reproduce

In this worktree, save `crates/agora-hub/tests/server.rs`, append `0535-return-resnapshot-socket-proof.rs` to it (the fragment reuses the suite's private helpers), then run:

```sh
cargo test -p agora-hub --test server review_535_return_closes_the_socket_and_resumes_with_retained_records -- --nocapture
```

That produces `/tmp/agora-535-fixed-wire/` with the real binary snapshots. With normal web dependencies and generated types:

```sh
cp docs/reviews/0535-return-resnapshot.test.ts apps/web/src/net/review-535-fixed.test.ts
npm --prefix apps/web test -- src/net/review-535-fixed.test.ts
rm apps/web/src/net/review-535-fixed.test.ts
```

Restore the saved server test file afterwards. Both proofs passed; temporary tests and source edits were removed before tracked checks. Results and CI snapshot are in the adjacent JSON. Old review/proof files remain as historical evidence of the failure.

The node is synthetic; no coding agent, model or real account is used. The server test uses real sockets without TLS; web lifecycle uses a controlled Socket and is not rendered-browser automation. The reopen check rebuilds a temporary copied store. No product changes or deployed service modifications remain.
