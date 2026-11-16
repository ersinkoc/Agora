# PR #629: legacy replay fix recheck

Reviewed head: `8ec3665edbb3529cc67681948e98d73fb95fdf8f`.
Verdict: **APPROVE**. The previously demonstrated historical client replay gap is fixed at the hub boundary, without rewriting stored events.

The moved exhaustive body classifier is shared by ingestion and rebuild. Rebuild records the end of the entire legacy log, rather than only the last bad event, as `distrusted_until`. A resume at/before a nonzero barrier takes the clean snapshot path; a checkpoint beyond the log also takes the existing snapshot path. A clean history has barrier zero and keeps ordinary replay. The guarded comparison is constant-time per subscription; it does not scan history on every resume. Legitimate hub-authored offline/link events remain trusted.

## Independent evidence

- **All 33 schema fixture bodies:** the actual live `NodeSession` result matches the history classifier (10 forbidden, 23 accepted for the fixture identities); all 33 hub-produced controls are trusted. This pins parity without copying the production match logic.
- **Real WebSockets:** seeded the original three forgeries plus two clean historical tail events. The last forgery is position 6 but the legacy end is 8. Seven checkpoints (0,3,4,6,7,8 and beyond the log end) all receive the same clean snapshot. A clean tail after the barrier replays normally; a resume at the current log end has an empty replay. All eight original stored records, including metadata/payload bytes, remain unchanged.
- **Real browser reducer:** replayed the fixture's old bytes to create an already-corrupt client at position 8 (agent hidden, victim count99, checkout assigned to n1). The actual reconnect snapshot heals it at position12: agent visible, count0, checkout assigned to n2, and the complete resulting view equals a fresh client's view. TypeScript is stripped and import locations adapted only; reducer logic is unchanged and protocol JS is generated from this checkout's schema.
- **Second disk-backed open:** after closing/reopening the same fixture store, legacy history raises its barrier to the new log end, while clean history keeps zero and ordinary replay. Stored bytes remain unchanged across the reopen.
- **Negative control:** temporarily removing only the subscription barrier condition makes the checkpoint proof fail immediately: a replayed event arrives instead of a snapshot (exit101). The runner restores every product/test source in `finally`; no control mutation is committed.

Existing hub suite: **279 passed, 5 ignored, 0 failed**. Three added native proofs and the actual browser proof pass. Formatting, docs checker and all-target hub Clippy with warnings denied pass on restored source. Required GitHub CI was green at the reviewed head. Local runtime evidence is Linux; no native Windows/macOS runtime is claimed. The author's 100k-event startup measurement is in the PR; it was not independently repeated here.

## Reproduction

```sh
cargo test -p agora-hub
REVIEW_629_FRAMES=/tmp/review-629-fixed-wire python3 docs/reviews/0629-fixed-run-proof.py
cd apps/web && npm ci && cd ../..
sh scripts/gen-ts.sh
node --disable-warning=ExperimentalWarning docs/reviews/0629-fixed-web-proof.mjs /tmp/review-629-fixed-wire
# Deliberate local negative control; expect exit101:
python3 docs/reviews/0629-fixed-run-proof.py --no-barrier-control
git diff --exit-code -- apps crates schema
```

The review uses only fixture data/fake authentication. Only evidence is committed; product source, existing tests and published fixtures are untouched. The main refresh contains already-landed unrelated work; this recheck concerns the new identity/replay fix, not an independent review of every intervening main change. The merger has since merged #629 under the maintainer's instruction; this verdict identifies the checked PR head and does not claim that Codex performed the merge.
