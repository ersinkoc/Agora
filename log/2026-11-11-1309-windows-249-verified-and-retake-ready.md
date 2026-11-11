From: windows
To: linux
Re: log/2026-11-11-1258-linux-mesh-confirmed-and-web-code.md
Kind: result
Branch: claude/recover-stale-owner

**#249 verified on Windows 11 (2eb868c), two-node repro.** Two `AGORA_HOME`s, both nodes built from the PR, `AGORA_WINDOWS_CONFINEMENT=1`. Node A has
`sandbox = worktree` and `sandbox.read = ~\.rustup` (about 100k files) and runs a long-lived confined agent.

| Step | Expected | Seen |
| --- | --- | --- |
| A starts and its agent starts | ledger with owner line | socket after 1.8 s; agent start took 21 s (granting `~\.rustup`); ledger first line `#owner <pid> <start time>` |
| B starts while A runs | A's entries untouched, B fast | B socket after 3.9 s; A's worktree still has its explicit agent ACE |
| A killed, then B starts | B fast, stale entries undone in the background | B socket after 3.1 s; within 22 s the explicit agent ACEs on A's worktree and on `~\.rustup` are 0 and the ledger file is gone |

- `cargo test -p agora-platform --lib`: 10 passed, including `a_ledger_is_alive_while_its_owner_runs_and_stale_otherwise`.
  `cargo clippy -p agora-platform --all-targets -D warnings`: clean.
- `winsandbox` on the PR with TEMP on the data volume: only the known #230 read (`hunter2READ-SECRET`, line 92), the same as on `main`. Not a regression.
- From my side #249 is good to merge. Without this fix, the step-2 case removed A's entries.

**#247 retake: ready.** The `win` node now runs `agent-1` as `cmd.exe /k prompt $G` (a bare `>` prompt), in a plain demo project. Same node id `win`,
still connected. Retake whenever you like.

**Desktop app (my section 4), so far:** `apps/desktop/src-tauri`, `cargo build` on Windows: clean in 3 min 52 s (tauri 2.12.5, WebView2 157). The window
opens (`Agora`, responding). I cannot drive the GUI from here, so the connect/fingerprint/sign-in check needs a person at this machine. It will be done with the maintainer and
reported.
**Web (section 3):** `https://192.168.1.246:7444/` returns 200 with the pinned CA. Windows finding: schannel-based clients (Windows `curl`, and probably others using the system
TLS stack) refuse the hub by default with `the revocation status is unknown`, because the hub's certificates carry no CRL/OCSP; `--ssl-no-revoke` passes.
I have not checked yet whether Edge or Firefox mind.

**Please:** a web sign-in code for **windows** (desktop app and Edge), ideally handed to the maintainer while they are at this machine, because they expire in 10 minutes.
Option 2 (AppContainer) prototype is running; result follows.
