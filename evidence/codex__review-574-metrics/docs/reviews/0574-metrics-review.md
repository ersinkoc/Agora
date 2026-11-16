# #574: independent metrics review

Reviewed head: `5cffd85f8e0a452ba853af1fd2eb6a80fcbaaa43`, stacked over #562 at `a87f339`.
**Changes needed:** the terminal-byte counter includes chat transcript bytes. The relay hot-path addition also needs a measured
before/after benchmark result, as required by AGENTS.md and ADR-0022.

## Reproduced counter defect

`Router::open_relay` explicitly uses `terminal: None` for chat. The production command path in server.rs passes a terminal id
for TerminalSubscribe and None for ChatSubscribe. Both streams reach `relay_frame`, whose new successful-send branch increments
terminal_bytes for every kind and every relay.

[Runnable proof](0574-chat-bytes-proof.rs) uses the real Router, an encoded ChatSnapshot (kind 10, **4,104 bytes**) on a chat relay,
and **4 bytes** of raw terminal output (kind 0) on a terminal relay. Both reach their client queues. The expected terminal total
is 4; the observed total is **4,108**. The regression test fails with exit 101 for that assertion. No product source is changed.
Thus an idle-terminal fleet with chat traffic reports terminal bandwidth, and the new terminal `rate()` is inaccurate.

Count successful frames only when the relay is a terminal relay. Add exact accounting tests for terminal output/snapshot/resize/end,
chat snapshots/entries, rejected sends and frames after closure. The current `>= 4` assertion misses contamination and over-counting.
The dropped-relay counter also sees chat relays; either restrict it to terminals or document its broader scope consistently.

## Hot-path verification

The successful relay path adds one relaxed atomic increment while holding the relay mutex. The PR's explanation estimates the
number of increments; its inherited ingest timings do not measure relay_frame. The repository benchmark crate does not depend
on agora-hub, so the green bench job does not demonstrate this path's cost. Please attach a reproducible before/after measurement
for successful terminal fanout, with payload sizes and client count identified, before approval.

## Other checks

- `cargo test -p agora-hub`: **257 passed, 3 ignored**; the new regression proof exposes what the existing tests miss.
- Approval lookup is qualified by the producer before timing, so identical local approval ids on different machines do not mix.
  The start time comes from that machine's request event; the resolution's time comes from the same producer. Duplicates do not
  re-enter publish; unknown or already resolved approvals do not add another observation.
- Histogram bounds are fixed arrays; the approval snapshot is copied under the hub mutex and formatted after release. This adds
  no per-event allocation. The published schema is unchanged; requested_at_ms belongs only to the hub projection.
- All native CI is green on the reviewed head. This does not substitute for the missing relay-path measurement.

To reproduce in a disposable worktree at the reviewed head:

```sh
cp docs/reviews/0574-chat-bytes-proof.rs crates/agora-hub/tests/review_574_chat_bytes.rs
cargo test -p agora-hub --test review_574_chat_bytes -- --nocapture
rm crates/agora-hub/tests/review_574_chat_bytes.rs
```

[Structured result](0574-validation.json) records the expected failing regression and observed byte counts. Tests use synthetic
in-memory routing, no live agents/accounts or deployed hubs. The author's next head needs a short re-check; #562 must still land
before this stacked PR. Review evidence stays remote; the throwaway worktree/target is removed after posting the verdict.
