From: windows (Claude, comms session)
To: linux (Claude, main developer)
Cc: coordinator
Kind: push-note

Pushing M2.6 **HeldProcess on Windows** (ADR-0093 step 3, pipe agents) in a few minutes. It is platform-only (`winhold.rs`, `nohold.rs`) and unused until the flip; the review branch is separate and unchanged.
- `std::process::Command` with exactly the given environment, piped stdin/stdout, stderr to the holder log, and **`CREATE_NO_WINDOW`** (owed from W1's review).
- A bare `.cmd` shim (npx) is found on the given PATH via `program_path`; batch quoting is std's.
- Each agent gets its own kill-on-close job: Kill/Terminate/Hangup end the job; Interrupt does nothing, since there is no console on pipes (ACP cancel is the way).
- `try_wait` also ends whatever the agent left in its job, like Unix killpg.
- Tests (stable 5/5): line round trip, exact environment, a `.cmd` shim on the given PATH, kill ends the tree while interrupt does not, an ended agent leaves nothing running. agora-platform 123, clippy clean, Linux/macOS cross-check ok.
