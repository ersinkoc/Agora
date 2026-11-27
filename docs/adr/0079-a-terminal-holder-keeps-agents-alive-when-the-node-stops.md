# 0079. A terminal holder keeps agents alive when the node stops
Status: Accepted
Date: 2026-11-15
Accepted: 2026-11-15 (the maintainer's explicit message in the merger chat, accepting the ADRs listed by number (#613 ADR-0060 Amendment A, #614 ADR-0078, #615 ADR-0080, #616 ADR-0079): they accepted all of them). The open questions' proposed answers stand.
Built (Linux service unit): the node's `agora service` unit has `KillMode=process`, so stopping or restarting it signals only the supervisor and the holder and its agents keep running (the hub's unit keeps `control-group`); `agora service install` rewrites a unit Agora wrote earlier (and reloads systemd, never restarts); the service uses it after `systemctl --user restart`.
Origin: the maintainer, after agents came back by resume on `win` (2026-11-15): "even if Agora closes, the agents opened in it must
not die, and when it comes back we must attach to the agents' screens again".

Refines ADR-0060 (resume), ADR-0062 item 8 (the supervisor) and ADR-0075 (update in place). It supersedes ADR-0075 decision 9 for
Windows. Whether it also replaces ADR-0075's exec in place on Unix is an open question below.

## Context
An agent's processes and its terminal belong to the node today:
- **Windows:** each agent runs in the node's kill-on-close job and on a pseudoconsole (`HPCON`) the node created. When the node ends
  for any reason (a crash, `agora down`, an update, the supervisor giving up), the job closes and the agents die. A pseudoconsole
  cannot be handed to another process (ADR-0075 decision 9).
- **Unix:** an agent leads its own process group and can outlive a killed node (ADR-0060's implementation note), but its PTY master
  was the node's. A restarted node cannot reach that terminal: the agent is reported `still_running` and can only be stopped.
- **What survives today:** a closed window or a hub restart never touches agents, because nodes are autonomous (ADR-0002) and windows
  reattach. Resume (ADR-0060, ADR-0062 item 4, #613) brings an agent's *conversation* back after its process died. It does not keep
  the process, its screen, a build running in its terminal, a shell, or a kind with no `[resume]`.
- **What nothing can keep:** a machine reboot ends every process. Resume is the only answer there, and it stays.

ADR-0075 keeps agents across an *update* on Unix by re-executing the node in its own process. It does not cover a crash, a stop or
the supervisor's restart, and it leaves Windows on restart-and-resume, because a Windows handoff "needs a separate design: a small
long-lived terminal holder process per node that owns the pseudoconsoles and the job". This is that design. Since the holder is not
the node, it also answers the crash and the stop, and it does so on every system.

### Measured on Windows 11 (WHITE, 2026-11-15)
A throwaway program played the roles. A *holder* created the pipes and a pseudoconsole and started a child that printed a counter
every second and echoed what was typed. A *reader*, standing in for the node, got the holder's pipe ends by `DuplicateHandle` and read
and typed through them.
- **Killing the reader** (`TerminateProcess`) left the child running. Its pseudoconsole stayed open, because the holder still held
  `HPCON` and its own pipe ends.
- **With no reader**, the holder read the output pipe itself and buffered it (ticks 6–10), so the child never blocked on a full pipe.
- **A second reader** got duplicates of the same pipe ends, read on (from tick 11 or 12), and typed a line that the child received
  and answered. Then it was killed too, and the child still ran.
- **In one run, one line went to the holder's own read** that was still in flight when the second reader attached. A handover has
  to end the holder's read first and hand its buffer over (decision 4).
- **The supervisor's job allows breakaway.** `spawn_bound` sets `JOB_OBJECT_LIMIT_BREAKAWAY_OK`, so a process started with
  `CREATE_BREAKAWAY_FROM_JOB` (as `spawn_detached` already does) is not ended when the supervisor's job closes.
- **Under Task Scheduler (measured 2026-11-16, a temporary task, as `agora service` runs `agora-node`):**
  - A task's process is in a job of Task Scheduler's own, which forbids breakaway: `CREATE_BREAKAWAY_FROM_JOB` from the task's
    process fails with error 5 (access denied).
  - A process that joins a kill-on-close job allowing breakaway (as the supervisor does) and then starts a child with
    `CREATE_BREAKAWAY_FROM_JOB` succeeds. The child leaves the inner job and stays in Task Scheduler's.
  - **`Stop-ScheduledTask` ended only the task's own process.** 30 s later the child that broke away from the inner job was alive,
    and so was a child started plainly in Task Scheduler's job.
  - So a holder the supervisor starts with breakaway outlives both the supervisor's job and a stop of the task. Signing out still
    ends it, since `agora-node` is a logon task.
  - **One rule follows:** `spawn_detached` falls back to starting *inside* the job when breakaway is refused. The holder must never
    take that fallback, because inside the supervisor's kill-on-close job it would die with the supervisor. It refuses to start
    instead, and the node runs its agents itself as today.
  - **Clarified 2026-11-27 (the maintainer):** the rule means a *kill-on-close* job. Task Scheduler's own job does not kill on close
    (limit flags 0), and a holder that broke away into it is fine. Re-measured on Windows 11 Pro 26300 with the same results as above.

## Decision
1. **One holder per Agora home** (`agora hold`, the same binary). It owns, for every agent of that home:
   - the terminal: the pseudoconsole and its pipes on Windows, the PTY master on Unix;
   - the agent's process: the holder starts it, so the holder is its parent and keeps its exit status;
   - on Windows, the kill-on-close job the agents run in.

   The node keeps everything else: the ledger, policy, approvals, detection, the VT state, mesh, hub link and every decision. The
   holder never decides anything. It starts, resizes, signals and stops agents only on its node's request, and keeps their terminals.
2. **Lifetime.**
   - The holder outlives the node. On Windows it leaves the supervisor's job (`CREATE_BREAKAWAY_FROM_JOB`). On Unix it is started
     in its own session and is not bound to the supervisor's death.
   - **What ends it:** `agora down --agents` (wording for the maintainer, open question 2), which stops the agents through it; a node
     that attaches and finds no agent left; or a machine reboot.
   - **It keeps what a node has not collected** (added in review, after acceptance). An agent that ends while no node is attached
     leaves its exit status and its buffered output in the holder. The holder keeps them, and keeps running, until a node attaches
     and takes them; only then does a holder with no agents left end. A holder with nothing but ended agents is one idle process.
   - **Plain `agora down`** stops the node and leaves the agents running in the holder. The next `agora` start reattaches. The
     `agora down` output and `agora status` say how many agents keep running.
3. **Attach without a hop on the hot path.** When the node attaches, the holder gives it duplicates of each terminal's pipe ends
   (`DuplicateHandle` on Windows, `SCM_RIGHTS` on Unix). The node then reads and writes the terminal directly, exactly as it does
   today, so no byte passes through the holder while the node is attached. The holder keeps its own copies. It does not read while a
   node is attached, and does not write.
4. **Detach and reattach.**
   - The holder watches its node, through the process handle on Windows and a pidfd or `kqueue` on Unix. When the node is gone, the
     holder starts reading each terminal into a bounded buffer: proposed 1 MB per terminal, the oldest dropped first. That way no
     agent blocks on a full pipe, and output written while no node was there is kept.
   - **On reattach,** the holder first ends its own read (`CancelIoEx` on Windows, a non-blocking read loop on Unix), then hands over
     the pipe ends and the buffer. The node feeds the buffer to the terminal's VT and gives each terminal a resize nudge (ADR-0075
     decision 3), so full-screen agents redraw. A buffer that starts in the middle of an escape sequence is only a few bad cells
     before the redraw.
   - **The screen of a quiet terminal** (added in review, after acceptance; see the note at the end):
     - **On a clean stop** (`agora down`, an update), the node hands each terminal's screen to the holder, in this order:
       1. It stops reading the terminal first. Output it has not read stays in the pipe, and the holder reads it into the
          buffer once the node is gone.
       2. Then it takes the terminal's VT snapshot. That is the emulator's whole state, not only the visible cells: cursor
          position and visibility, current text attributes, the saved cursor, the scroll region, the alternate screen and the
          modes the agent set (keypad, mouse, bracketed paste, wrap). With it, later output is read exactly as it would have been.
          `agora-vt` serializes and restores that state.
       3. It hands the snapshot to the holder and exits.

       The next node restores the snapshot, then feeds it the buffer, so every screen comes back whole.
     - **After a node crash** there is no snapshot. The new node has only what was written after the crash. A full-screen agent
       redraws on the nudge, but a quiet shell, or a tool waiting for input, shows a blank screen until it writes again (a key
       press, Enter at a prompt). Its process and its state are untouched.
       The agent's state is untouched too: the node takes up the state the ledger last knew (source `Screen`, so a rule that matches
       replaces it), and shows `unknown` only when the ledger knew none. Before this, a quiet agent stayed `unknown` until it wrote.
     - This is a v1 limit (Consequences). Keeping a periodic snapshot in the holder is open question 5.
   - **Windows clients** take a fresh snapshot, as after any reconnect (ADR-0075 decision 7).
5. **The node-holder link.**
   - An owner-only local endpoint in the home's `run` folder: a named pipe with the owner-only security descriptor on Windows, a
     `0700` unix socket on Unix.
   - The holder accepts one node at a time, and only the node that holds this home's role lock (ADR-0062 item 8), checked by the
     peer's pid and start time against the lock.
   - Messages: start an agent (the launch the node already decided), resize, signal, stop, list, and attach (hand me the
     terminals). A small versioned format private to Agora, not a protocol message (`schema/` is for the wire), with golden
     fixtures like the ledger's.
   - **A node that does not speak the holder's version** does not attach. It restarts the holder only when no agent runs; otherwise
     it reports the agents as running in an older holder (they can be stopped, and resumed after) and changes nothing.
6. **Confined agents and security.**
   - The holder can start programs, so it is at least as sensitive as the node's control socket. Its endpoint lives in the home,
     which confined agents may not reach (#475, ADR-0047, ADR-0050), and it admits only the lock-holding node. A confined agent must
     not be able to open it: that is the T16 lesson, and it gets a test on each system.
   - The holder starts an agent with exactly what the node passes: the confinement token or profile already built by the node's
     `winconfine` / Landlock / Seatbelt path, the environment and the folder. It adds and widens nothing.
   - **Threat-model row:** a holder's endpoint reached by another local process; a holder that kept running with no node, in which
     case its agents keep running unsupervised (approvals still need a node, since the holder never answers a permission prompt).
7. **Updating.** Updating the node no longer touches agents on any system: the node stops, the new node starts and attaches. The
   holder itself is small and changes rarely. A new holder version is taken only when no agent runs, or at the next reboot. An
   `agora update` that brings a new holder version says so.
8. **When the holder dies** (a crash, or killed by hand), its agents end. This decision was made explicit in review, after
   acceptance (see the note at the end).
   - **Windows:** the holder's kill-on-close job closes, and its agents end, as with the node's job today.
   - **Unix, with a node attached:** nothing tells a child its parent died, and the node holds copies of the PTY masters, so the
     terminals do not hang up by themselves. So the node watches its holder: the link closes, and the same pidfd or `kqueue` watch
     as decision 4. When the holder is gone, the node:
     - closes its copies of the masters, so each session gets the kernel's hangup (`SIGHUP`) once no copy is left;
     - **lists each agent's group members first**: every process whose process group is the agent's, found on Linux through
       `/proc` and on macOS with `proc_listpids(PROC_PGRP_ONLY)`, each recorded with its pid and start time;
     - sends `SIGTERM` to each recorded member, then 5 s later `SIGKILL` to each one still running.

     Signals go to each recorded member by its pid after its start time is checked again, never to the group id. So a member that
     outlives its leader is still ended, and a group id reused by someone else is never signalled. An agent's group is found from
     its leader as ADR-0060's ledger identifies it (pid, start time, boot). A leader that is not the recorded process yields no
     members.
   - **With no node attached, Linux and macOS:** nothing is guaranteed to end every process.
     - On Linux the holder starts each agent's first process with `PR_SET_PDEATHSIG(SIGHUP)`. Only that process is told; it can
       ignore the signal, and its children do not inherit it.
     - On both systems, the kernel's hangup on the last master reaches the terminal's foreground process group.
     - A process that ignores `SIGHUP`, or runs outside that group, can keep running. The next node lists the agent's group members
       as above, reports the agent `still_running` (ADR-0060) while any member runs, and can stop them.

     This is a v1 limit (Consequences). A crash-proof end on Linux (a cgroup per agent) is open question 6.
   - **Resume** (ADR-0060 and its Amendment A) then brings the conversation back, as after a reboot.
9. **The ledger** records which agents are held, and by which holder (pid and start time). A node that starts and finds its holder
   alive attaches instead of reporting the agents as interrupted. Only agents whose holder is gone are interrupted.

## Alternatives considered
- **ADR-0075's exec in place, Unix only.** It keeps agents across an update, but not across a crash or a stop, and it does nothing
  for Windows. With a holder, an update is an ordinary stop and start.
- **tmux, screen or WSL as the holder.** They are not on Windows, they are an outside dependency with their own versions and
  security, and they put every byte through their own terminal emulation.
- **The holder relays every byte.** That is simpler, but it adds an IPC hop for every terminal byte and every key on the hot path
  (ADR-0022). Handing over the pipe ends avoids that while the node is attached.
- **Only resume.** That is what exists today: the conversation survives, the process, its screen and its work in flight do not.

## Consequences
- **`agora-platform`:**
  - Windows: pseudoconsole and job creation callable from the holder, `DuplicateHandle` into the node, breakaway start, and a
    cancellable read of a pipe.
  - Unix: PTY masters passed by `SCM_RIGHTS`, a session of its own, and a pidfd or `kqueue` watch of the node.
- **`agora-node`:**
  - the spawner and supervisor start, signal and wait for agents through the holder;
  - "interrupted" holds only for agents whose holder is gone;
  - attach on start.
- **`crates/agora`:** `agora hold`, the supervisor starting the holder before the node, `agora down` and `agora status` wording, and
  `agora update` refusing to replace a holder that holds agents.
- **ADR-0075:** decision 9 is superseded for Windows. Whether decisions 1–6 (exec in place on Unix) are still worth building is open
  question 1.
- **Performance:**
  - attached, the hot path is unchanged (the same pipes, read by the node);
  - detached, one reader thread per terminal in the holder.

  A benchmark (ADR-0022) checks that attach adds nothing per byte, and measures the time to reattach 16 agents.
- **Tests with `agora-fake-agent` on Linux, macOS and Windows:**
  - kill the node: the agent's pid is unchanged and its terminal answers after reattach;
  - output written while detached arrives after reattach;
  - `agora down` keeps agents and `agora down --agents` stops them;
  - a second node is refused while one is attached;
  - a confined agent cannot open the holder's endpoint;
  - a holder version mismatch changes nothing;
  - `agora down` and then a start: a quiet shell's screen comes back from the snapshot, and the cursor position, text attributes
    and modes it set carry over too;
  - output written between the node's last read and its exit reaches the next node;
  - an agent that ends while no node is attached: its exit status and output reach the next node, and only then does the holder end;
  - kill the holder:
    - on Windows the agents end;
    - on Linux and macOS with a node attached, the node ends every recorded member of each agent's group, including one that
      outlived its leader, and never signals a group the ledger does not identify;
    - with no node attached, an agent whose first process ignores `SIGHUP` is reported `still_running` by the next node, which
      can stop it.
- **v1 limits:**
  - after a node *crash*, a quiet terminal's screen is blank until the agent writes again; its process is untouched (decision 4);
  - with no node attached when the holder dies on Linux or macOS, a process that ignores `SIGHUP` can survive until the next node
    stops it (decision 8).
- **Threat model:** a row for the holder (decision 6).

## Open questions for the maintainer
1. **Unix:** should the holder replace ADR-0075's exec in place (one mechanism everywhere, proposed), or should both be built?
2. **Stopping:** should plain `agora down` keep agents running (proposed), with `agora down --agents` stopping them too? Or the
   other way round?
3. **Buffer:** 1 MB per terminal while no node is attached (proposed), or more?
4. **Order:** this touches `agora-platform`, `agora-node` and `crates/agora` on three systems. Proposed: Windows first (where agents
   die today), after the easy-setup work that holds those crates now.
5. **A screen that survives a crash** (added in review, after acceptance): should the node also hand the holder a screen snapshot
   every few seconds while attached, so that a quiet terminal's screen survives a node crash too? Proposed: later, after a
   benchmark (ADR-0022), since it costs a snapshot per terminal every few seconds.
6. **A crash-proof end on Linux** (added in review, after acceptance): should each agent run in a cgroup of its own, so that a
   holder dying with no node attached still ends every process (`cgroup.kill`)? Proposed: later. It needs a delegated cgroup
   (systemd user slice), which not every Linux machine gives a user. macOS has no equivalent.

**Note on the review changes (CodeRabbit on #616, two rounds, after acceptance):** these fill gaps the review found:
- decision 2: the holder keeps an ended agent's status and output until a node takes them;
- decision 4: the quiet-screen rule, the read order and the full VT state on a clean stop;
- decision 8: the per-system outcome, members that outlive their leader, and the no-node limit;
- the matching tests, the v1 limits, and questions 5 and 6.

None of them changes a decision the maintainer accepted. **Confirmed 2026-11-16 by the maintainer's own message in the merger chat: "ADR-0079'daki inceleme eklerini onaylıyorum"** (the review additions to decisions 2, 4 and 8, the tests and limits, and the proposed answers to questions 5 and 6: later).
