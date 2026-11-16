# PR #598: filename-only privacy fix recheck

Reviewed head: `8bd1ffe931c3dc935247f859f3e096481bae22fc`.

Verdict: **TECHNICAL APPROVE**. The original transport regression now passes (30/30 nonowners receive exactly the filename; 9/9 owners retain full paths). The permanent regression passes too. Protocol/node/hub tests: **660 passed, 4 ignored, 0 failed**; formatting, docs checker and all-target Clippy with warnings denied pass. All reported required CI checks are green, including the three native Rust OS jobs (attributed to CI).

The previous head `96dc373984cd9cb2ca9158b6a6c6e146a0c4ea8d` disclosed the fixture username in every one of 30 nonowner deliveries in the [original review](https://github.com/ersinkoc/Agora/blob/69b3c655796eb223ce644f6339d70ee47403e6ca/docs/reviews/0598-privacy-review.md). The fix now keeps only the final nonempty path component, splitting on both `/` and `\` independently of the hub OS. No containing folder reaches nonowners. The owner's full path remains unchanged.

## Reproduction

`0598-home-path-proof.rs` is byte-for-byte the original proof from evidence commit `69b3c655796eb223ce644f6339d70ee47403e6ca` (SHA-256 `f3e75dd0364e936cfbb7923e8645454f5222a77f3f0d46fc24c755c2313095d0`). The wrapper isolates its helper names from the permanent regression that the author added; no test assertions or fixtures were weakened.

```sh
python3 docs/reviews/0598-fixed-run-proof.py
```

The runner restores the server test source in `finally`. Its real loopback TCP/WebSocket hub tests Linux, macOS and Windows path strings against owner, member and viewer access: snapshots and live events with/without project restrictions, and backlog for unscoped users. There are 30 nonowner deliveries and 9 owner controls, 39 total. This is Linux transport execution; native macOS/Windows process execution is not claimed.

The author's promoted test, `a_machines_user_name_in_its_binary_path_reaches_only_the_owner`, also asserts the exact filename (`agora` or `agora.exe`) for nonowners, strengthening the original no-username assertion.

## Scope and integration

The intervening main merge `d1473e1c` has parents `96dc3739` and `57ec5558`. It makes no changes to the previously reviewed hub/node/protocol crates, hub capability declaration, schema, ADR-0069 or protocol spec. The subsequent six-file fix changes the sanitizer, its tests, and corresponding comments/spec/ADR wording. Published golden fixtures remain unchanged; the original append-only/schema and capability review still applies.

Only evidence files are committed on this review branch; production source and existing tests are restored. Technical review does not grant or relay acceptance of the ADR amendment, or authorize merging under the separate maintainer acceptance hold.

Nonblocking PR description cleanup: the older review-fixes paragraph still names the former folder-and-file test and `Downloads/agora` expectation, and a lower paragraph says `NodeSnapshot.binary` is not named in the ADR. These descriptions should match the final code/amendment before integration.

Results are recorded in `0598-fixed-validation.json` and `0598-fixed-observed.txt` alongside this note.
