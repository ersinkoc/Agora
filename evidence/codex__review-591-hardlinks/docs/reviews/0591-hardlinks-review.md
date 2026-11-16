# #591: independent Codex review

Reviewed head: `3a5d41f9e21bfaf5dfb533d258cef0600f0cf81b` (initial checks at `8669780d`). **Changes needed.** The newer file-write-entry
check does not change the traversal's child-name decision; the same control still fails at the latest head.

## Directory exclusions also exclude regular files

The traversal computes the child check flag from its name before considering its type. A plain file named `node_modules`,
`target` or `.git` therefore gets links=false. If that file is a hard link, the later regular-file branch never calls
has_other_names. The documented exception is for package/build/git directories; a standalone linked file is skipped too.

[Walk control](0591-walk-control.py) extracts the production broad_writer_below body and exemption constant without changing
them, then compiles/runs that body on Linux. It uses real filesystem hard links and native link-count metadata; Windows
DACL/owner queries are stubs that allow the fixture. An ordinary linked file is refused, all three documented directory
exceptions remain allowed, but all three **regular-file** names incorrectly pass. The expected-refusal assertion fails, exit 101.

Only turn off descendant link checking for actual non-reparse directories with those names. Regular files must retain the link
check. Add exact native Windows coverage for each standalone file name, while keeping the directory-exemption coverage.

The [Windows regression module](0591-windows-file-exemption-proof.rs) calls the real Windows broad_writer_below and
has_other_names. It cross-compiles for x86_64-pc-windows-gnu, but **was not executed on Windows by this Linux reviewer**.
Append it to winconfine.rs in a disposable Windows worktree and run the command in its header. Native Windows confirmation is
requested from the author; the Linux control proves the traversal decision, not ACL propagation or a confined process escape.

## Security documentation

The new threat-model paragraph says macOS already refuses multi-linked file write entries. The implementation is in open PR #592;
macos.rs in this head has no hard-link count check or use of that refusal. Land #592 first or describe that change as planned until
its implementation lands. This is an integration/documentation dependency, separate from the standalone-file defect.

The latest threat-model update correctly broadens the exclusion description to any pre-existing link planted below those names,
including private-file targets. That review point from the other Windows session is addressed. No broader exception is needed to
fix the standalone-file bug. The newer direct-file write-entry check is also consistent with that session's feedback.

## Checks and limits

- Linux platform/i18n tests pass at 8669780d; the later changes affect only Windows code and the threat-model text. [Structured
  results](0591-validation.json) record counts and the latest control result.
- Windows all-target clippy with -D warnings passes. The added native Windows regression module passes cross-compilation.
- Formatting, docs and whitespace checks pass; original product files are unchanged after the review.
- Native Windows/macOS CI was green on the initial head. Those runs include the author's existing cases, not the new regression.
- Shared token-to-refusal mapping reaches the node through sandbox_refusal/why.token without a new exhaustive-match requirement;
  en/tr selectors and error-codes.md carry the same stable reason. The node puts the same worktree path in write and reopen.

Run the Linux decision control in a disposable worktree:

```sh
python3 docs/reviews/0591-walk-control.py
```

Expected on this head: ordinary file refused; each directory exempt; each same-named regular file not refused; exit 101.
No deployed machines, real accounts, agent tools or security settings were touched. This review does not turn Windows
confinement on or claim native Windows execution. Remote evidence remains after the throwaway worktree/target is removed.
