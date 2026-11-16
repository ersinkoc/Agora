# #592: independent Codex review

Reviewed current head: `10e52f24b9a80cbecce92c2d86865b2e9c548a8d`. Its macos.rs is byte-identical to the initially checked
`8363d5f278e17479f5324282fee36a82d9dedc0f`; the later commit merges main/#591. **Changes needed: the file-entry mitigation
does not cover the atomic-save sibling paths that the same profile makes writable.**

## Remaining writable-file case

The new guard examines write_real only. A listed state.json with one name passes, then sibling_rule authorizes
state.json.* for reads and writes. A pre-existing state.json.tmp hard-linked to an unlisted private file therefore remains
writable. No parent directory is in Sandbox.write in this case, so this is distinct from the documented folder-entry exception.
The sibling allowance is pre-existing; this is a remaining case in the hard-link mitigation, not a claim that the PR introduced it.

[Profile control](0592-sibling-profile-control.py) extracts and runs the **unchanged** profile, literal, quote and sibling_rule
functions on Linux with real files/link counts. Darwin path walking is replaced with static-fixture metadata/path shims; no
symlinks or concurrent swaps are involved. The direct-file hard-link positive control is refused. With the base file back to one
name and its pre-existing sibling at two names, profile generation succeeds and its actual regex matches the linked sibling.
The expected-refusal control exits 101. This proves the policy-generation gap, not a native Seatbelt write.

[Native macOS test](0592-native-sibling-proof.rs) is supplied separately and cross-compiles for aarch64-apple-darwin. It verifies
the ordinary file profile first, creates the sibling hard link, then either accepts a typed refusal or runs the real profile
through sandbox-exec and checks that the unlisted target remains unchanged. **It was not executed on macOS by this reviewer.**
Please run it natively before treating the file-entry hard-link case as covered.

Check pre-existing siblings that the generated rule makes writable, or narrow that permission so an existing multi-linked sibling
cannot be written. Preserve ordinary atomic saves (the existing a_listed_file_lets_its_siblings_be_saved test is the positive
workflow control). The documentation should distinguish this remaining file-family case from intentionally unscanned directories.

## Sound parts and checks

- The direct-file guard reads link count from the walk's result; ordinary descriptor resolution uses that descriptor's metadata.
  The native test added by the author covers one-name files, direct hard links and the intentional folder exception.
- The stable reason, enum, en/tr text and error-code value match #591 byte for byte. No published message/schema is changed.
- Linux platform/i18n tests: **74 passed** on the initial head. Later changes do not alter macos.rs. Native macOS tests are the
  author's/CI's evidence; Linux does not execute them.
- macOS all-target clippy and the added native regression's cross-compilation pass. Formatting, Linux clippy, docs and whitespace
  checks pass. Original required native CI was green, but did not cover this new sibling case.

Run the Linux policy control in a disposable worktree:

```sh
python3 docs/reviews/0592-sibling-profile-control.py
```

Expected on this source: direct-file control passes, base link count 1, sibling link count 2, generated sibling regex allows it,
exit 101. The native proof header gives the macOS command. [Structured evidence](0592-validation.json) records source identity
and attribution. No real account, deployed node, agent tool or security setting was used or changed; production files are restored.
