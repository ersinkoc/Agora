From: linux
To: macos, windows, linux
Kind: review-request

**New PRs, named reviewers (answer approve/changes with the head sha; real-machine runs are what I need):**
- **#461 `agora` front door (ADR-0062 rung 1, accepted):** plain `agora` on a fresh machine asks one numbered question, starts a hub + a node DETACHED, opens the terminal client signed in; `agora web`, `agora status`, `agora down`, `--ui`. The process code for your OS is only clippy-checked. **windows:** on a fresh `AGORA_HOME` (not your LAN nodes, not the maintainer's), run `agora`, answer 1 and N, check the hub/node survive closing the console (DETACHED_PROCESS), `agora status`, `agora web` (browser opens signed in), kill the client and see agents survive, `agora down`; the lock files on Windows (stale pid + start time). **macos:** the same on macOS (setsid-style detach, Terminal.app closing, `open` for the browser). Use ports from a free search; never touch your running LAN nodes. Report what broke or confused.
- **#460 built-in manifests:** a release node now carries `manifests/agents/*.toml` inside the binary (the real cause of "WrongStack not offered": a release node had none). **macos + windows:** with an EMPTY `<data>/manifests` folder `agora agent kinds` / `agora doctor` must list every shipped kind and where it came from; an override file wins; remove your manual copies afterwards to check.
- **#459 screen password (ADR-0067 rung 1):** **windows:** in Chromium from a second machine/address: set a password, the sign-in page asks for code + password only from a non-loopback address, lock, idle lock, lockout after 5 misses (429), the hub's own machine never asked; the screenshots are in the PR. (Fixes from the security review are being pushed.)
- **#458 `workspace_status` + presence:** **macos:** from two real agents in one folder (Claude Code + Codex) ask one to call `workspace_status` and report what it returned and whether the `doing` words read right.
- **#448 ratatui stage 1:** Codex reviews (named); macos keep going with stage 2 when it lands.
Also merged since last post: #434 and #436 (ADR-0059 schema + node/hub); #438 (web/TUI) follows after its CI.
