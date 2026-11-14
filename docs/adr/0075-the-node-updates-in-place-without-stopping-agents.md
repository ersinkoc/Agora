# 0075. The node updates in place without stopping its agents
Status: Proposed
Date: 2026-11-15

Refines the live handoff of ADR-0019 (Accepted), which is not built yet. It changes only the mechanism on Unix; the rest of 0019 stands.

## Context
ADR-0019 decided that updating a node must not kill running agents. On Unix, the old node passes the PTY master file descriptors to a
*new process* over `SCM_RIGHTS`, with serialized runtime state; on Windows it duplicates the ConPTY handles; a failure falls back to
"restart with native agent resume". The fallback is now built: the ledger (ADR-0060) and automatic resume after a restart
(ADR-0062 item 4, #581). The handoff itself is not built.

Resume after a restart is not the same as never stopping. Every update, and so every `agora` upgrade on a machine, still loses:
- the screen and scrollback of every terminal;
- work in flight: a tool call the agent was running, a command typing out in a shell, a build in a terminal;
- every agent that cannot continue its own session: plain shells, kinds with no `[resume]`, and a Codex agent whose session file was
  not matched;
- up to 5 s × N/8 of waiting while agents come back 8 at a time.

The maintainer (2026-11-13/14): "my interface must never freeze them or lose them." Updating is the most common reason a node stops.

Passing handles to a second process meets three problems in today's code:
1. **Agents are the node's children.** The supervisor waits for them (exit status, restart policy), signals their process groups, and
   the ledger records their pid and start time (ADR-0060). A second process is not their parent: it cannot wait for them. Linux
   could fix that with `PR_SET_CHILD_SUBREAPER` only for orphans, and macOS has no equivalent.
2. **The node is bound to its supervisor.** That uses `PR_SET_PDEATHSIG` on Linux, and on macOS a `kqueue` thread inside the node
   that watches the supervisor's pid (ADR-0062 item 8). The role lock holds the node's `pid:start-time`. A new process breaks all three, and needs a lock handover
   while two nodes are alive.
3. **Two processes at once.** Between start and handover, two nodes would read and write the same ledger, sockets and data folder.

## Decision
1. **Unix: the node re-executes itself (`execve`) in its own process.** It keeps the same pid and the same start time, and runs the
   new binary on disk. Agents stay its children. At no moment do two nodes run. This replaces the `SCM_RIGHTS`-to-a-new-process step
   of ADR-0019 on Unix. What keeps the node bound to its supervisor:
   - **Linux:** `PR_SET_PDEATHSIG` survives `execve` for a binary that is not set-user-ID; nothing to do.
   - **macOS:** the watch is a `kqueue` thread inside the node, armed by `end_with_supervisor` from `AGORA_SUPERVISOR_PID`, which
     the node removes from its environment at start. `execve` ends that thread. So the re-exec passes `AGORA_SUPERVISOR_PID` back in
     the new image's environment, only when the node was started bound. The new image re-arms the watch first thing, as at any start.
     If the supervisor ended in between, the new image ends at once, as a node started after its supervisor ended does. Without this,
     a killed supervisor would leave the re-executed node running (review of #582, windows).
   - **The lock:** its `pid:start-time` stays valid. Its `version` field names the old build until the new image rewrites it under
     the lock's guard, which it does right after reading the handoff file.
2. **What crosses the exec.** Open descriptors cross; `FD_CLOEXEC` is cleared right before `execve`, and only on the ones named in
   the handoff file:
   - PTY masters;
   - the control socket listener;
   - the per-agent mesh listeners;
   - adapter agents' stdio pipes.

   A **handoff file** (`<data>/run/handoff-<pid>`) carries what the new binary cannot rebuild from disk:
   - for each agent: id, kind, pid, process group, its descriptors by number, terminal size, restart counters and launch facts as
     the ledger has them;
   - pending approvals;
   - every replay guard held in memory. Nonces and grants already used stay used: the handoff never reopens a window a restart
     would not.

   The file is owner-only and opened like the ledger (no link, the owner's file, written atomically). It is read once and deleted.
   It holds no terminal contents and no secrets beyond what the ledger holds. Its format is **node-private and versioned**, not a
   protocol message (`schema/` is for the wire). A binary that does not know the file's version does not read it.
3. **The screen is not carried in v1.** After the exec each terminal gets a resize nudge (one row smaller and back, `SIGWINCH`), so a
   full-screen agent (Claude Code, Codex, WrongStack, opencode) redraws at once. Scrollback from before the update leaves the window
   in v1. Carrying the VT grid is a later step, with its own budget check (ADR-0022).
4. **Order of a handoff.** It is triggered by `agora update` after the installed binary is replaced, or by ADR-0019's self-update
   after its signature check.
   1. Check the new binary first, in a child process: it must start and report a handoff version it accepts. A broken download never
      replaces a running node.
   2. Drain, bounded (proposed 10 s). No new launches or control commands are accepted. The error code `node.updating` is added to
      the error catalog, append-only; the window shows "Updating…". The node waits until no launch is half done and no adapter
      request is in flight. Terminal bytes keep flowing.
   3. Flush the event buffer and the ledger. Write the handoff file. Clear `FD_CLOEXEC` on the named descriptors. Call `execve`
      with the node's own argv plus `--handoff <file>`.
5. **Failure is never worse than today's restart.**
   - If `execve` fails, it returns. The old node lifts the drain and keeps running as it was.
   - If the new binary fails on the handoff file (a panic, an unknown version), it exits. Its supervisor starts it again
     (ADR-0062 item 8), and it starts as an ordinary restart: agents whose terminals closed come back through automatic resume.
   - If the drain does not finish in time, the update is refused with a reason naming the busy agent. The person can try again, or
     choose `agora update --restart`, the restart-and-resume path.
6. **Mesh bridges reconnect.** The `agora mesh stdio` bridges in each agent lose their connection at the exec, and the listeners
   stay open. A bridge reconnects to its same socket path, with backoff, without failing its MCP client. A bridge started by an
   older version does not reconnect; its agent's mesh tools fail until that agent restarts. So reconnecting ships one release
   before the handoff is relied on.
7. **The hub link drops and comes back.** The node is autonomous (ADR-0002) and its buffered events are sent on reconnect. Windows
   take a fresh terminal snapshot, as after any reconnect.
8. **Who can trigger it.**
   - The machine's owner, through the local control socket (`agora update`).
   - The hub only through ADR-0019's flow: a node binary signed with the pinned release key, and the user's approval verified on
     the node (ADR-0020).
   - The node runs only the path it was started from, resolved at start. It never runs a path named in a request.
9. **Windows keeps restart-and-resume for now.** A pseudoconsole handle (`HPCON`) belongs to its process and cannot be handed on;
   `ClosePseudoConsole` ends its conhost. Agents run in the node's kill-on-close job. There is no `execve`. A Windows handoff needs a
   separate design: a small long-lived **terminal holder** process per node that owns the pseudoconsoles and the job, which the node
   reconnects to. It is not chosen here. It adds a process, and an IPC hop for every terminal byte, on the hot path (ADR-0022), so
   it would also need a benchmark. Unix does not need it.

## Consequences
- On Linux and macOS, an update keeps every agent running: same processes, same terminals, same pids. That includes shells and kinds
  that cannot resume, and work in flight. Clients see a short reconnect and a redraw.
- Windows stays on restart-and-resume until a holder design is accepted.
- One more format to maintain: the handoff file, versioned, with golden fixtures like the ledger's. An older binary never reads a
  newer file.
- Pending approvals and in-memory replay guards must be serializable. Today a restart drops them; the handoff carries them.
- No hot-path change. The drain pauses only control commands, never terminal input or output.
- Tests on Linux and macOS with `agora-fake-agent`:
  - a handoff to the same binary keeps the node's pid, each agent's pid and a live PTY round trip;
  - on macOS, killing the supervisor after a handoff ends the re-executed node (the watch is re-armed); on Linux the same test checks
    `PR_SET_PDEATHSIG`;
  - the lock's `version` names the new build after the handoff;
  - the ledger is unchanged;
  - a mesh bridge reconnects;
  - a handoff file the new binary refuses falls back to restart-and-resume;
  - a failing `execve` leaves the old node serving.
- `docs/security/threat-model.md` gets an entry for the handoff file and the trigger.

## Open questions for the maintainer
1. Exec in place instead of `SCM_RIGHTS` to a new process on Unix (decision 1). This supersedes that one step of ADR-0019.
2. Losing scrollback in v1 (decision 3): acceptable, or must the VT grid be carried from the start?
3. Drain: 10 s, and refuse rather than silently fall back to a restart (decision 4)?
4. Windows: stay on restart-and-resume, or start the terminal-holder design now (decision 9)?
