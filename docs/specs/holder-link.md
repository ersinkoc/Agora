# The holder link (ADR-0079 decision 5)

Status: built for Unix in W2 step 1 (`crates/agora-node/src/holder_link.rs`, the server in `crates/agora/src/hold.rs`) and used by the node in
W2 step 2 (`crates/agora-node/src/holder.rs`, see "How the node uses it" below). Windows joins in a later step with the same frames and
a named pipe.

The link is **private to Agora**: a node and its own terminal holder (`agora hold`) talk over it on one machine. It is not a protocol message
(`schema/` is for the wire between nodes, hubs and windows), and nothing outside this machine speaks it. It is versioned, small and has
golden fixtures (`crates/agora-node/tests/fixtures/holder-link/*.hex`, one frame's payload each; a fixture is added, never edited).

## Endpoint and who may connect
- A unix socket `<home>/run/hold.sock`, mode `0600`, in the `run` folder (`0700`). Windows: a named pipe with the owner-only descriptor.
- The holder reads who connected from the kernel (`SO_PEERCRED` on Linux, `getpeereid` and `LOCAL_PEERPID` on macOS): the peer must be
  the same user, and either its pid is the pid in `<home>/run/node.lock` (ADR-0062 item 8) with the lock's start time, read now, or
  (step 2) it is a **direct child of the live lock holder named `agora`**. The lock names the node's *supervisor*, which runs the node
  as its only child, so the real node is the second case. An agent is never a direct child of the lock holder (the node's or the
  holder's child), so no agent is admitted by it; the name only keeps a lock held by something else from admitting what it started.
  Any other peer is closed without an answer and without reading a byte from it.
- One node at a time. A second connection while one is attached is answered `Refused(Busy)`; one whose process ended has lost its place.

## Frames
`length: u32 BE` (of what follows, 1 to 4 MiB), then `tag: u8` and the body. Integers are big-endian; text is `u32` length and UTF-8; an
option is a flag byte (`0`/`1`) and the value; a list is a `u32` count (never more than the bytes left) and the items. A frame with
bytes left over, an unknown tag or a cut field is refused (`Malformed`) and the link is closed.

**Frozen for every version:** the frame, `Hello` (tag 1: `version: u16`) and `Refused` (tag 65: `code: u8`, `holder_version: u16`), so two
builds can always tell each other that they differ. A message changes only by a new tag.

Node to holder: `Hello`(1), `Attach`(2), `Start`(3: agent, argv, cwd, env, env_remove, cols, rows, cpu_seconds, memory_bytes),
`Resize`(4), `Signal`(5: interrupt, terminate, kill, hangup), `Stop`(6: `force` is `SIGKILL`, else `SIGTERM`, to the agent's group),
`List`(7), `Release`(8).
Holder to node: `HelloOk`(64: version, holder pid, holder start time), `Refused`(65), `Ok`(66), `Started`(67), `Agents`(68),
`Terminal`(69: agent, pid, state, buffer-dropped flag, buffer), `AttachDone`(70: count).

## Order
1. `Hello`. A different version is `Refused(VersionMismatch)` and changes nothing: the node does not attach; it restarts the holder only
   when no agent runs, and otherwise reports the agents as running in an older holder (decision 5).
2. `List` may come at once. Everything else but `Attach` is `Refused(NotAttached)` until `Attach`.
3. `Attach`: for each agent the holder **first ends its own read** (it waits for the reader thread), then sends one `Terminal` frame
   with the PTY master attached (`SCM_RIGHTS`) for a running agent, and `AttachDone`. A running agent's buffer is forgotten **only after
   its frame was sent**, so a link that breaks mid-way loses nothing (what was sent into a socket nobody reads is the one unavoidable
   loss); an ended agent's status and buffer are sent without a terminal and **kept** until the node sends `Release` (it may die
   between the two). Terminals are never read by the holder while a node is attached, and it never writes to them.
4. `Start` returns `Started` with the new terminal attached. The launch is the node's: the holder adds and widens nothing. (The
   confinement a node builds is not carried yet; a later step adds it as a new tag.)
5. **The holder reads a terminal only once the node's process has ended** (a pidfd on Linux, `kqueue` on macOS, else the start time): it
   then reads each terminal into a 1 MiB buffer (oldest dropped first), so no agent blocks on a full terminal. A link that drops while
   the node's process still runs does **not** start the reads: the node still holds its own copy of each master and reads it itself,
   and a terminal two processes read cannot be read without blocking (the master is shared by `dup`, so the holder cannot make its
   copy non-blocking without making the node's one non-blocking too). The holder watches that process and starts reading when it
   ends; a node that comes back first (same process or another that holds the lock) simply attaches, and no byte is read twice.
   A node that drops the link and stops reading its terminals while it lives lets the agents fill them, as a node does today.
6. `Release` of an ended agent forgets it. A holder that held agents and has none left ends (decision 2).

A running agent gets `SIGHUP` when the holder dies (Linux `PR_SET_PDEATHSIG` on its first process; the kernel's hangup on the last
master) and nothing guarantees more: decision 8.

A terminal is read in exact sizes (the length, then the payload), so the descriptor sent with a frame's first byte is never taken for the
next frame's.

## Limits of the holder itself
- Connections are checked and closed (a peer that is not the lock holder gets no answer and no read) at most 16 per pass of the holder's
  loop, and the node is served first in every pass, so a flood of connections from the same user slows the node by those checks at
  most. A peer that is the lock holder and starts a frame then stalls holds the holder for up to `FRAME_TIMEOUT` (5 s); sends have a
  write timeout of the same length, so a node that never reads cannot hang the holder.
- A holder agent starts with **exactly** the environment in `Start` (the holder runs it with an empty environment plus that list; the
  holder's own environment is a stale snapshot of whoever started it and is never passed on). `env_remove` is therefore redundant for
  the holder and kept only for the format.
- The terminal master, the listener, the wake pipe, the lock files and the pidfd are all close-on-exec; an agent sees none of them
  (tested with an agent that lists its own descriptors). Neither does it see what the **holder was handed at its start** (a pipe of its
  starter, a log): `agora hold` closes every descriptor but 0, 1 and 2 first thing, and the child of each started agent marks all of
  them but 0, 1 and 2 close-on-exec before `exec` (marked, not closed, so the pipe std uses to report a failed `exec` still works).
  Both are tested, the first with a holder started with extra non-close-on-exec descriptors.
- A write to a node that does not read gives up after `FRAME_TIMEOUT` (5 s in all, each blocked send waits `WRITE_SLICE`, 1 s), and the
  holder drops that node: a node that stops reading cannot hang it (tested).
- `agora hold` refuses to start (it exits 1, `hold-failed`) when it cannot become a session leader (`setsid` fails with `EPERM` in a
  process-group leader); it never runs on in its starter's session.

## How the node uses it (W2 step 2)
- **Where:** `agora_node::holder` (`HolderHub`, `HolderSession`), `Supervisor::launch_held`, `Supervisor::adopt_held`, and
  `agora_platform::held_pty` (the terminal of a held agent as a `Pty`). Unix only (`agora_platform::HOLDER_BUILT`); elsewhere nothing
  here runs and the node runs its agents itself.
- **Starting the holder:** by the first agent that needs one, never at the node's start (a node that starts nothing leaves no idle
  holder): `agora hold --home <home>` through `spawn_detached`, whose child calls `setsid` before `exec` (a start where `setsid` fails
  is an error and means "no holder here"; never `spawn_bound`). The node connects (retrying `Busy` for a few seconds, never reading it
  as a second node), sends `Attach` (a new holder has nothing to hand over), then `Start`. A holder that started but cannot be used (it
  does not admit this node, a different version) is sent `SIGTERM` and the node runs the agent itself; the node does not try again for
  30 seconds. `agora node run` only uses a holder when its own process would be admitted (the lock names it or its parent): otherwise
  a holder it started would refuse it and idle.
- **What goes through it:** only an agent the node does not confine (`cfg.sandbox` is `None`). A confined agent (Landlock, Seatbelt) is
  started by the node as before: the holder does not carry a confinement yet, and `Start` is unchanged. A start the holder refuses
  (`Failed`: the program is not there; `AgentExists`) is the node's own start's to report.
- **Environment:** the node sends the complete environment `spawn` would have given the agent (its own, minus the removed names,
  plus the kind's and the node's additions). Variables that are not UTF-8 cannot travel and are left out.
- **After the start:** the node reads and writes the master it received itself; nothing passes through the holder per byte. Its copies
  of the master and of the link are close-on-exec. Resize and the foreground process use the master directly; a signal and the agent's
  end go over the link (`Signal`/`Stop`, `List` then `Release`). A terminal's exit is polled (the actor looks about once a second
  while idle and at the end of the terminal), one `List` round trip each.
- **When the node stops cleanly** (`Supervisor::shutdown`): each held terminal is let go (`PtyControl::detach`: its reader ends, its
  copy of the master closes, its drop no longer kills) and the link closes, so the next node attaches at once. When it is killed the
  kernel does the same, and the holder starts reading when the process has ended (item 5 above).
- **When the next node starts:** it attaches to the running holder before it calls anything interrupted. For each running agent whose
  ledger entry names this holder (pid and start time, written by the node's tick, `Entry::holder_pid`/`holder_started`) it restores the
  agent, builds its terminal from the master, feeds it what the holder buffered, nudges its size (two resizes) so full-screen programs
  redraw, registers its mesh endpoint again at the same path, and clears the interruption (`Ledger::reclaim`). An agent that ended while
  no node was attached is released and stays interrupted (offered for resume); its status and output are not replayed. An agent the
  ledger does not know, or whose folder is gone, is left as the holder keeps it.
- **When the holder dies:** a terminal whose holder cannot be asked reports the agent's end with no status; the supervisor's
  restart rule then applies (a new holder is started on the next start, or the node runs it). The group-member listing of decision 8
  (`SIGTERM` then `SIGKILL` to every recorded member) is **not built**: the node closes its copies of the masters (the kernel's hangup)
  and nothing more.
- **`agora down`:** stops the node and hub as before; the holder keeps running. The holder writes `run/hold.pid` (`pid start agents`)
  whenever the number of running agents changes; `agora down` says how many keep running and `agora status` counts them while the
  node is down. `agora down --agents` sends the holder `SIGTERM`: it asks every agent's group to end, kills what remains after 3
  seconds, and ends.
- **Known limits:** (1) a selected folder is checked to be in place right before and right after the start, but the holder changes
  into the *path*, not into the handle the node holds (the node's own start uses `fchdir` on the handle): a swap within that window is
  possible, and the agent is ended at once if the check after fails; carrying the folder handle needs a new tag. (2) Under `agora
  service` on Linux the unit's `KillMode=control-group` ends the holder (and so its agents) when the unit stops: the unit needs
  `KillMode=process` (or the holder its own scope) before agents survive a service restart. (3) macOS is compile-checked only; Windows
  has no holder yet. (4) A node whose holder link breaks while the holder lives does not take up the agents it still holds until
  the next node start. (5) A different holder version is refused and the node runs its agents itself; the "agents running in an older
  holder" report of decision 5 is not built.

## REQUIRED of step 2 (the node's side; found in the independent review of #760): done in step 2, as described above
1. **Environment:** the node sends the complete environment of each agent (what `spawn` would have built from the node's own
   environment, minus `env_remove`, plus its additions), because the holder passes nothing of its own.
2. **Starting the holder:** the node (or the supervisor) starts `agora hold` through a path that guarantees `setsid`: detached, and not
   as a process-group leader (a fork once, or `spawn_detached`). A start where `setsid` fails refuses, and the node must treat that as
   "the holder is not available" and run its agents itself, as today. Never `spawn_bound` (it would tie the holder to its starter).
3. **Descriptors:** the node's link descriptor and every master it receives stay close-on-exec, so no agent the node (or the holder)
   starts inherits them (`recv_with_terminal` sets it).
4. **Busy after a drop:** when a node drops the link and connects again at once, the holder notices the old end of file one pass (50 ms)
   later and answers `Refused(Busy)` until then. `Client` users retry on `Busy` for a few seconds (the tests do) and treat it as "try
   again", never as "a second node exists".
5. **Dropping the link** while the node process keeps running does not make the holder read (item 5 above): a node that wants the
   holder to buffer for it must close its copies of the masters (or end).
6. **A different node process taking the lock while the old one still runs:** admitting it clears the "departed" watch of the old
   node (item 5 above), so the holder would not read, but both processes could then read the same master. The old node must close its
   masters, or end, before the new node attaches.
   Step 2: a clean stop closes the masters before the link (`Supervisor::shutdown`); a node that is alive and not stopping while another
   takes the lock (not a case `agora` produces: its supervisor ends the old node first) is not handled.
