# 0045. The Windows node: how it gets terminals, a control channel and a process view
Status: Accepted
Date: 2026-11-09

## Context
The project builds and tests on `windows-latest` in CI, and releases a Windows binary, but a Windows machine cannot be a node:
`agora_platform::PTY_SUPPORTED` and `LOCAL_SOCKETS_SUPPORTED` are `cfg!(unix)`, so there is no pseudo-terminal, no local control
socket (the node's CLI, the mesh MCP bridge and the hub link all use it) and no process inspection (`process_cwd`, `process_tree`,
`process_stats`). `agora hub`, `agora tui` (as a client) and `agora update` already work there; running agents does not. The
vision lists "a Windows PC" among the machines a node runs on, and Phase 7 lists the Windows node. Doing it needs OS calls the
standard library does not expose, so it needs a dependency decision (AGENTS.md) and a boundary decision (all of it stays in
`crates/agora-platform/src/windows.rs`).

## Decision
1. **One new dependency: `windows-sys`** (the Microsoft-maintained raw bindings; no code of its own, no transitive dependencies,
   target-gated in `agora-platform/Cargo.toml` as `[target.'cfg(windows)'.dependencies]` so no other platform compiles it). Only
   the feature groups used are enabled. No other new crate.
2. **Terminals: ConPTY** (`CreatePseudoConsole` with two anonymous pipes, `CreateProcessW` with the pseudoconsole attribute,
   `ResizePseudoConsole`, `ClosePseudoConsole`). It implements the same `Pty` trait the Unix backend does, so `agora-node`,
   `agora-vt` and detection do not change. The child is placed in a **job object** with kill-on-close, so stopping an agent
   ends its whole tree (the Unix backend does the same with process groups).
3. **Local control channel: a named pipe** `\\.\pipe\agora-<user sid>-<name>` created with an owner-only security descriptor,
   behind the existing `local_listen` / `local_connect` functions and the `LocalStream` type, so the callers keep their code.
   `--socket PATH` on Windows names the pipe (the path's file name is used). Unix domain sockets on Windows are not used: the
   standard library does not support them there and the pipe gives the owner-only access check the socket file gives on Unix.
4. **Process view.** `process_children` and `process_tree` use the toolhelp snapshot; `process_stats` uses
   `GetProcessTimes` and `GetProcessMemoryInfo`; `process_name` uses `QueryFullProcessImageNameW`. **`process_cwd` is not
   provided** on Windows (it would mean reading another process's PEB, which is undocumented and breaks across versions): it
   returns `Unsupported`. Detection on Windows therefore relies on the screen rules and on what an agent reports itself (hooks,
   ACP, adapters), not on the cwd of a foreground process; checkout attribution of usage files that use `cwd` still works because it
   compares the agent's own worktree path with the path in the file.
5. **Order and scope.** W1 named-pipe control channel (a Windows node can run and be asked `agent list`); W2 ConPTY terminals and
   job objects; W3 process view; W4 CI: the supervisor, terminal and node integration tests that use `agora-fake-agent` run on
   `windows-latest` (they are `#![cfg(unix)]` today); W5 docs and the install story (`agora hub ssh install` to a Windows host is
   not part of this: the user installs the release binary by hand or with `agora update`). The constants become `true` per stage.
6. **Not in scope:** services (a Windows service or Task Scheduler entry), WSL (a WSL node is a Linux node and works today),
   Windows ACL work beyond the pipe's descriptor and `write_private` (still default permissions; a separate task with its own
   threat-model note), and signing the Windows binary.

## Consequences
- The node runs natively on Windows, with the same event, approval and mesh behaviour, because only `agora-platform` changes.
- A new dependency for one target; `cargo deny`/audit gates must accept `windows-sys` (it is already in the tree through tokio and
  ring on Windows).
- Each stage can be reviewed alone, and only W4 can prove them: the machine used for development is Linux, so CI on
  `windows-latest` is the only test environment. Expect several CI round trips per stage.
- Without `process_cwd`, a terminal agent that does not report its directory cannot be matched to a checkout by its foreground
  process on Windows; worktree-based attribution still works.
- Reversible: the constants and the target-gated dependency are the only switches.

## Alternatives considered
- **A loopback TCP port with a token file** as the control channel: no FFI, but any local user could connect to the port and the
  token file would be the only protection; rejected for a channel that can start agents.
- **`uds_windows` or another socket crate**: a second small dependency for a channel the OS already offers as named pipes.
- **Running the Windows node only through WSL:** works today and stays supported, but it does not see Windows projects, tools or
  agents installed on Windows.
