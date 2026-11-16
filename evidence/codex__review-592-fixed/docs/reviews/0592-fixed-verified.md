# #592: save-sibling mitigation verified

Reviewed head: `6c7703e1fde644939f4f235452dee7d41ebdbe69`. **Approve.** The writable save-sibling P2 is addressed.

The file-entry guard now checks already-existing names that sibling_rule makes writable. A regular matching sibling with multiple
links refuses the profile. Opening the parent directory for enumeration failing also refuses. Directory write entries keep the
previously documented no-recursive-scan scope. The CHANGELOG and threat model now name the save-sibling case.

## Independent checks

The [profile control](0592-sibling-profile-control.py) retains the original case and includes the newly added production helper
in extraction. It runs the actual profile/quoting/sibling functions on Linux, with real link metadata and path-walk shims.
Only the platform walk is substituted; the permission-generating code is unchanged.

- A direct hard link remains refused.
- A one-name base plus two-name state.json.tmp now refuses the profile: exit **0** for the expected-refusal control (previously 101).
- A normal one-name state.json.lock still permits the profile.
- A linked state.jsonx, which only resembles the save-family prefix, does not incorrectly reject the profile.

The last two are additional positive/name-boundary cases. These checks verify profile generation, not native Seatbelt execution.

## Native macOS evidence and attribution

The author ran the [original native proof](0592-native-sibling-proof.rs) on macOS 27.2 before and after the fix. The before run
printed native_exit=Some(0) and outside_unchanged=false; the after run passed because the profile refused. The report is the
Codex P2 fixed at 6c7703e comment on PR #592 and `comms:log/2026-11-19-2030-macos-592-siblings.md`.
This native execution is attributed to the author; the Linux reviewer did not run Seatbelt.

The same submitted native proof cross-compiles for aarch64-apple-darwin on the reviewed head. The persistent Mac unit test now
contains the sibling, ordinary lock and lookalike cases; the existing native atomic-save test remains a positive workflow control.

## Other validation

- Fresh Linux platform/i18n tests: **76 passed**.
- macOS and Linux all-target clippy with -D warnings pass; formatting, docs and whitespace checks pass.
- Shared enum/token, en/tr selectors and error-code hunks match merged #591 byte for byte. No schema change or new dependency.
- Required native platform CI is green on the reviewed head. Full directory-entry scanning is still outside this patch's scope.
- Production files are restored byte for byte; no account, agent tool, deployed node or security setting was touched.

Run in a disposable worktree of this head:

```sh
python3 docs/reviews/0592-sibling-profile-control.py
```

[Structured results](0592-fixed-validation.json) record outcomes and attribution. The native proof header gives its macOS command.
Remote evidence is retained after posting the verdict; the clean throwaway worktree/target is removed.
