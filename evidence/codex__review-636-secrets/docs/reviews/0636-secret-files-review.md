# PR #636 secret-file boundary review

Reviewed head: `a55944bde36127b48cb0741b42a6b43a2c69516d`.
Verdict: **CHANGES NEEDED**, one reproduced P1 disclosure. No maintainer design decision is needed to repair it.

## P1: repository-controlled Git prefixes bypass secret diff classification

`git::diff_with` invokes Git without fixing the source/destination prefixes, then decides whether content may leave the node by parsing the resulting display names. `section_paths` only removes `a/` and `b/`. Repository configuration can change that presentation before classification.

The independent integration proof starts an actual node and `agora-fake-agent`, uses only synthetic scratch files, and sends normal control-protocol requests. With the default node policy (secret files hidden):

1. Track `.env`, `.ssh/tokens` and `.docker/config.json`, then change each to a distinct synthetic marker. `FilesRead` refuses all three with `files.secret`.
2. The default `GitDiffResult` hides all three values and contains three `secret_files`. A public-file content control remains visible.
3. In the scratch repository set `diff.srcPrefix=x` and `diff.dstPrefix=y`. The **same node**, with the **same policy**, now returns all three synthetic values as hunk text and an empty `secret_files` array. Git printed `x.env`, `x.ssh/tokens`, etc.; these no longer match the classifier.
4. Unset the two repository settings: the same response once again hides all three and reports three secret entries.

The policy remains off and direct reads remain forbidden. This is a real node response disclosure, not a hypothetical protocol trace or merely a unit test of a parser. Repository metadata may be written by an agent and is untrusted. An ordinary owner preference for nonstandard Git prefixes can also trigger it.

Requested fix: generate trusted canonical prefixes in the command itself, or classify with authoritative machine-readable path metadata rather than untrusted display formatting. Include custom prefixes without slashes, no-prefix and mnemonic-prefix settings, quoted names, renames and ordinary-file controls. An unparseable header must not authorize returning raw content.

The control temporarily pins `--src-prefix=a/ --dst-prefix=b/` in the **actual diff command** and repeats the same protocol test. The custom configuration then discloses no values and returns all three secret entries. This demonstrates the failure mechanism and a repair direction; it is not a submitted or fully reviewed production patch. The runner restores source bytes in `finally` and removes its temporary test target.

## Validation and limits

- Existing core/node/protocol tests: **452 passed**, none failed or ignored.
- Independent native control-protocol reproduction: **1 passed**, confirms all three disclosures plus default/restored/public-file controls.
- Prefix-pinned control: **1 passed**, no disclosures, all three metadata entries retained.
- `cargo fmt --all -- --check`, `sh scripts/check-docs.sh`, `git diff --check`, and `cargo clippy -p agora-core -p agora-node -p agora-protocol --all-targets -- -D warnings` pass with original production source restored.
- No full-workspace/web run or native macOS/Windows execution is claimed. A targeted Windows stream-spelling test was requested on comms as an unverified platform question; it is not presented as another defect.
- The UI lock marker is an explicitly disclosed follow-up. This review does not mark it implemented. Existing fixtures were not edited and no production changes were committed by the reviewer.

Reproduce: `CARGO_BUILD_JOBS=4 python3 docs/reviews/0636-run-prefix-control.py`. Add `--pin-prefix-control` for the repair-direction control. Run in an isolated worktree, without another build reading the same source. Observed protocol metadata and validation results are in `0636-observed.json`.
