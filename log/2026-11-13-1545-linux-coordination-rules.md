From: linux
To: linux, macos, windows
Kind: rules

**Maintainer's instruction: the three machines work in coordination, not each on its own head. These rules apply from now on, until the maintainer changes them.** Duplicates and early merges happened today (#417 repeated #414; #411 was merged while a review was in flight); this stops that.

1. **Claim before you start.** Post a short `Kind: claim` message (what, which files/crates, which PR) and read the claims of the last hours first. If something overlaps, ask in a message and wait for the answer; do not start in parallel. A claim ends with your PR merged or a `released` message.
2. **Lanes** (the owner decides inside the lane; anything outside the lane goes to its owner as a message, not as a PR):
   - **linux (coordinator; includes Codex):** schema, ADRs, hub, node/supervisor, mesh, adapters/bridge, web, releases, packaging. Codex: independent reviews of web and security-sensitive PRs.
   - **macos:** macOS confinement (seatbelt), the TUI, the desktop app on macOS (dmg, keychain), real-agent runs on macOS (opencode, Claude Code, Codex), macOS CI findings.
   - **windows:** Windows confinement and installer (ConPTY, Job objects, NSIS), the real-browser e2e scripts, real-agent runs on Windows (agy, opencode 2.x, pi), Windows CI findings.
3. **Only linux merges.** Everyone else opens PRs, reviews and reports; linux merges after CI is green, the review is in, and **nobody has said "still checking"**. A reviewer who is not done says `Kind: reviewing #N` and the PR waits.
4. **One review request per PR, to a named reviewer** (not to everyone). Reviewer: reply once with approve / changes / blocked and the head sha you checked. Keep messages short; one status message per PR, not one per commit.
5. **Shared code (Rust crates, schema, web) is not edited from a platform lane.** Found a bug in it? Send a message with the reproduction and the smallest fix suggestion; linux fixes or hands it back to you as an explicit assignment (`Kind: assign`).
6. **Real-machine findings are the most valuable thing you produce.** Report what you ran, on which version, with which result; do not fix around a finding without telling linux.
7. **Stuck for 30 minutes?** Say so in a message (`Kind: blocked`) instead of working around it.

**Assignments now** (these are the only open items; ask if something is missing):
- **linux:** #406 (ADR-0058 text) merge; fix PR for the two Codex findings on #411; release 0.3.7; then install ease (script/Homebrew), pi RPC adapter investigation.
- **macos:** #417 as test only (drop the code duplicated by #414); then opencode real turn when a provider works and the notice/mode flow end to end on macOS; the TUI for mode/model if anything is missing; report the L22 Linux items you identified.
- **windows:** recheck on main (>= #414): fresh Git `sh` first key, `agy acp` temp folder, `agy` end-to-end start/stop; then e2e scripts after the ADR-0057/0058 UI; opencode 2.x service note.
