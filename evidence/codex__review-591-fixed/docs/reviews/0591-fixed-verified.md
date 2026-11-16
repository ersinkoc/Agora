# #591: directory-only exclusion fix verified

Reviewed head: `0999a1e0435e494ede9e36e7508705b820541838`. **Approve the Windows fix.** The standalone-file P2 is addressed.

The child exemption now requires a directory file type and rejects the symlink classification before matching the three names.
The same-named regular files therefore retain link checking. The author's persistent Windows unit test includes all three names
and uppercase NODE_MODULES. The direct-file write-entry check also remains in place.

## Verification and attribution

- The [unchanged production-walk control](0591-walk-control.py) now exits **0**. An ordinary hard-linked file is refused; all three
  real-directory exclusions still pass; regular hard-linked files named node_modules, target and .git are all refused. The prior
  head failed this same expected-refusal assertion with exit 101.
- This control runs the extracted, unchanged Windows traversal on Linux with DACL/owner-query stubs and native hard-link metadata.
  It verifies the type/name decision, not Windows ACL propagation or a confined process.
- The [Windows regression module](0591-windows-file-exemption-proof.rs), which calls the real Windows helpers, cross-compiles on
  this exact head. The author reports that this same submitted module was temporarily appended and **executed natively on
  Windows 11, passing**, then removed; see the Fixed at 0999a1e0 comment on PR #591 and
  `comms:log/2026-11-19-1730-windows-591-fixed-process-tree.md`. That native execution is the author's evidence, not a run by this
  Linux reviewer. The author also reports winconfine 11/11 and winsandbox 12/12.
- Fresh Linux platform/i18n tests: **74 passed**. Windows all-target clippy and the native regression's cross-compilation pass;
  Linux all-target clippy, formatting, docs and whitespace checks pass. All required native CI is green on the reviewed head.

The shared enum token, locale selectors and error-code value are unchanged by the fix. The broader excluded-subtree risk is now
documented, including arbitrary links planted by an unconfined actor. Windows confinement stays off by default.

## Integration note

The macOS sentence is explicitly tied to still-open #592. Keep that dependency visible: land #592 first, or describe its behavior
as planned until it lands. This Windows review does not certify the macOS implementation. No review blocker remains for the
Windows file-exemption correction itself.

## Reproduce

```sh
python3 docs/reviews/0591-walk-control.py
```

Run in a disposable worktree of the reviewed head. Expected output: normal_file_refused=true, three directory exempt=true lines,
three regular-file refused=true lines, exit 0. The Windows proof header gives its native command.

[Structured results](0591-fixed-validation.json) record the checks and attribution. No production changes are retained, no security
setting was changed, and no real account or agent was used. Remote evidence stays after the clean throwaway worktree/target is removed.
