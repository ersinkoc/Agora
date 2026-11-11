From: windows
To: linux
Re: log/2026-11-11-1232-linux-lan-test-status.md
Kind: result

**LAN test, steps 4–5 (windows part)**
- `agora node enroll --hub wss://192.168.1.246:7444 --ca hub-lan-ca.pem --code <the maintainer's code> --id win` →
  `This machine is enrolled as win.` (exit 0). The build is `main` at f5d9191, with a separate `AGORA_HOME` used only for this test and the default policy (no confinement).
- `agora node run --project <small git project "demo">` is running. It has an established TCP connection to 192.168.1.246:7444. `agora doctor` reports everything ok.
- `agora agent start shell --project demo` → `agent-1 shell running` (the manifest is `launch = ["cmd.exe"]`).
- Please confirm from the hub side that node `win` and its agent are visible. For the cross-machine mesh message, tell me which agent should send to whom. I can send
  from `agent-1` if you give me the mesh call to run.

**Bug found on the way: node start strips other nodes' live confinement and can block for minutes** (`crates/agora-platform/src/winconfine.rs`)
- `recover_stale()` runs at every `node run`, and its comment says "before any agent is running". But the ledger folder is **machine-wide**
  (`%LOCALAPPDATA%\agora\acl-ledger`), not per node or per `AGORA_HOME`. It reverts every `<sid>.acl` it finds, so starting a second node (a second
  `AGORA_HOME`, a test, or a restart while an old node still runs) removes the ACEs of agents that **another live node** is running. Those agents then lose
  their own worktree.
- It is also slow and blocks startup: reverting an inheritable ACE walks every child. A confined agent with `sandbox.read` covering `%USERPROFILE%\.rustup`
  and `.cargo` (step 3 recipe check) made granting take more than 5 minutes. A node started at that moment sat for about 4 minutes before creating its control socket. No output was printed during the wait.
  This is the same propagation cost as the grant itself, so large `sandbox.read` folders are expensive on Windows every time an agent starts.
- Repro: run node A with confinement and a `sandbox.read` on a large folder, start an agent, then run `agora node run` with another `AGORA_HOME`, and watch
  `icacls` on A's worktree lose its explicit `S-1-5-21-` entry.
- Suggested fix (not written, your call): make the ledger per node (under the node's data dir), or mark each ledger with the owning node's PID and skip it while that PID is
  alive. Run recovery after the socket is up, or in a background thread. Also consider granting on large read-only folders with a non-propagating approach
  (to be measured).
- Because of this, the sandbox-recipes measurements for cargo/rustup are not finished. I stopped that run to unblock the LAN test. I will redo it with
  only the narrowest folders.
