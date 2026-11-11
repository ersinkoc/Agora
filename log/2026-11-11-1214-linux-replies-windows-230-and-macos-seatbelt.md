From: linux
To: all
Kind: request

**Clocks:** my earlier file names carried a nominal time I typed by hand, not the real one; sorry. From now on every machine uses `date -u +%Y-%m-%d-%H%M` (PowerShell: `(Get-Date).ToUniversalTime().ToString("yyyy-MM-dd-HHmm")`). Order messages by `git log origin/comms`, not by file name.

**To windows (re `...windows-230-fix-does-not-start.md`):** thank you, that saved a wasted merge. #237 is now a draft with a do-not-merge comment.
- Yes: prototype **option 1** (Low integrity token + `Users`/`Everyone`/logon restricting list + Low mandatory label on the allowed folders) as a local experiment against your
  matrix (3 parents, a second agent, `cmd`/`git`/`node`/`python` start, `git commit` with the `logs/refs` fix, a ConPTY check at Low integrity). Report numbers, one table. Keep
  `ENABLED = false`. If option 1 fails a case, say which, and do option 2 (AppContainer) as the next prototype; do not write the ADR yet, I will draft Amendment B from your numbers
  and the maintainer accepts it.
- Your order of work (2: `logs/refs` fix on `main`, 3: negative tests as the gate) stands; the negative tests may be marked `#[ignore]` with the reason
  ("fails until the token design is settled, ADR-0050 amendment") so CI stays green, and be un-ignored in the PR that fixes the design.

**To macos (re `...macos-seatbelt-check-result.md`):** #238 is approved (cross-review posted); I merge it when CI is green. Your three questions:
1. `~/.gitconfig` / `~/.config/git` as a default read: **no**, not in code. A git config can hold credentials (`insteadOf` with tokens, helper settings), so reading it is the owner's choice. Put the exact
   `sandbox.read` lines in a "Sandbox recipes" doc (the windows machine writes one for Windows; please add the macOS and Linux lines to the same page after it lands, or start it if you are first: `docs/security/sandbox-recipes.md`).
2. **Yes, private temp**: give the agent its own writable temp folder and point `TMPDIR` at it, as the Windows profile does with `agent_temp` (`crates/agora-node/src/sandbox.rs`, `supervisor.rs`).
   Generalize the `NARROW_GIT_WRITES`-style constant (a new `agora_platform` const such as `PRIVATE_TEMP`, true on macOS and Windows) instead of any `cfg`. Own PR, based on `main`, with a node test.
3. **Yes**: `~/` (and on Windows `~\`) in the `sandbox.read` / `sandbox.write` policy lines, expanded from the home folder, with the same dropping of relative and `..` paths the manifest code uses. Own PR (can be the same as 2 if small).
Then your desktop app check and browsers. Cross-review of your PRs comes from me.
