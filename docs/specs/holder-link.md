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
4. `Start` returns `Started` with the new terminal attached. The launch is the node's: the holder adds and widens nothing. An agent id
   that is empty or longer than `MAX_AGENT_ID` (256 bytes) is `Refused(BadRequest)` and nothing starts (#929), so every reply that names
   an agent fits a frame (below, "Limits of the holder itself"). A holder that already keeps `MAX_HELD_AGENTS` agents (15,251, terminal
   and pipe agents together) answers a `Start` or `StartPipe` for a new id `Refused(Failed)` and starts nothing (#983). (The
   confinement a node builds is not carried yet; a later step adds it as a new tag.) `Refused` is frozen and carries no reason, so a
   start refused with `Failed` is logged on the holder's stderr (`run/hold.log` when a node started it; an operational line, not shown
   to a person): the agent id, the step that failed (full; the terminal or the program; the copy of the terminal for the node), the
   error's kind and number, and whether the program (and the folder, when one was asked for) exists at that moment. The program's
   arguments and the environment are never logged.
5. **The holder reads a terminal only once the node's process has ended** (a pidfd on Linux, `kqueue` on macOS, else the start time): it
   then reads each terminal into a 1 MiB buffer (oldest dropped first), so no agent blocks on a full terminal. A link that drops while
   the node's process still runs does **not** start the reads: the node still holds its own copy of each master and reads it itself,
   and a terminal two processes read cannot be read without blocking (the master is shared by `dup`, so the holder cannot make its
   copy non-blocking without making the node's one non-blocking too). The holder watches that process and starts reading when it
   ends; a node that comes back first (same process or another that holds the lock) simply attaches, and no byte is read twice.
   A node that drops the link and stops reading its terminals while it lives lets the agents fill them, as a node does today.
   One exception, for an agent being **ended** (a `Stop` was sent) that is still not reaped 200 ms later: no node read its terminal
   (a start whose answer was lost, or a node that stopped reading), and on macOS a process that exits with unread terminal output stays
   in the exiting state until that output is read. The holder then reads what waits in that terminal (non-blocking, into the agent's
   bounded buffer) so the agent can exit. It never does this while a node whose link dropped still runs (that node may read it), nor
   for an agent that was not asked to end, nor within the grace: a node that reads its terminal lets an ending agent go by itself.
   The wait was seen on a real Mac (#776). From the XNU source, and measured on a Mac (2026-11-26: kept when read 0.5 s after, gone at 0.7 s), it is bounded at the **last** close of the
   terminal's program side (no other process of the agent keeps it open): output still unread 0.6 s after that close is discarded
   (`ptsclose` drains with a 60-tick timeout, then flushes), where Linux keeps it readable on the master (tested). So on macOS, what an
   agent wrote last before it ended is kept only when someone reads the terminal within that time: its node, or the holder, which
   starts reading a tick (50 ms) after it sees the node's process end.
6. `Release` of an ended agent forgets it. A holder that held agents and has none left ends (decision 2).

A running agent gets `SIGHUP` when the holder dies (Linux `PR_SET_PDEATHSIG` on its first process; the kernel's hangup on the last
master) and nothing guarantees more: decision 8.

A terminal is read in exact sizes (the length, then the payload), so the descriptor sent with a frame's first byte is never taken for the
next frame's.

## Limits of the holder itself
- Connections are checked and closed (a peer that is not the lock holder gets no answer and no read) at most 16 per pass of the holder's
  loop, and the node is served first in every pass, so a flood of connections from the same user slows the node by those checks at
  most. A peer that is the lock holder and starts a frame then stalls holds the holder for up to `FRAME_TIMEOUT` (5 s): once a frame
  has begun each receive waits at most `WRITE_SLICE` (1 s) or what is left of the 5 s, and a frame whose last bytes come later is
  refused (tested, also when the receive that completes the frame returns after the deadline: the read's clock is given, #1089); sends have a write timeout of the same length, so a node that never reads cannot hang the holder.
- **Agent ids** are at most `MAX_AGENT_ID` (256 bytes; the node's own are `agent-<n>`, at most 26), refused on `Start` and `StartPipe`
  (#929). Before, an id of a few MiB still fitted its `Start`, but every later `Attach` reply (the id and up to 1 MiB of buffer) was
  larger than `MAX_FRAME`: the holder could not send it and dropped the node on every try. With the bound a `Terminal` frame (one id and
  the whole 1 MiB buffer) and a `PipeData` frame (one id and everything kept) always fit (checked at compile time and tested), and one
  agent costs at most `MAX_LIST_ENTRY` (275 bytes) in `List`.
- **The number of agents** is at most `MAX_HELD_AGENTS`, as many as one `List` reply (one frame naming every agent) can carry:
  `(MAX_FRAME - 5) / MAX_LIST_ENTRY`, 15,251 (#983; checked at compile time and tested). Every agent the holder keeps counts: terminal
  and pipe agents, running ones and ended ones not yet released (`Release` frees the place). A `Start` or `StartPipe` for a new id
  past it is `Refused(Failed)` before anything starts (an id it already keeps is still `AgentExists`), so `List` is never larger than
  a frame and is not paged. Before, nothing bounded the count: a holder holding more could not send `List` (`write_frame` refuses a
  frame larger than `MAX_FRAME`) and dropped the node on every try. The bound is far past what a machine runs (every terminal agent
  is a PTY, every pipe agent two threads of the holder).
- A holder agent starts with **exactly** the environment in `Start` (the holder runs it with an empty environment plus that list; the
  holder's own environment is a stale snapshot of whoever started it and is never passed on). `env_remove` is therefore redundant for
  the holder and kept only for the format.
- The terminal master, the listener, the wake pipe, the lock files and the pidfd are all close-on-exec; an agent sees none of them
  (tested with an agent that lists its own descriptors). Neither does it see what the **holder was handed at its start** (a pipe of its
  starter, a log): `agora hold` closes every descriptor but 0, 1 and 2 first thing, and the child of each started agent marks all of
  them but 0, 1 and 2 close-on-exec before `exec` (marked, not closed, so the pipe std uses to report a failed `exec` still works).
  Both are tested, the first with a holder started with extra non-close-on-exec descriptors.
- A write to a node that does not read gives up after `FRAME_TIMEOUT` (5 s in all, each blocked send waits `WRITE_SLICE`, 1 s, or what
  is left of the 5 s when that is less, and is given at most 2 KiB, one kernel buffer even at the smallest `SO_SNDBUF`, so it
  waits only once). `write_frame` sets the stream's write timeout to `WRITE_SLICE` itself, so the bound holds whatever the caller set (none, or a longer one; tested). In the frame's last two `WRITE_SLICE`s a send is first waited for with `LocalStream::wait_writable` (`poll(2)` on Unix, rounded up to a millisecond, at most a slice and what is left), so the frame ends at `FRAME_TIMEOUT` and not when a late socket timer fires after it (a 1 s send timeout fired 24-27 ms late on Linux; the late-timer case is reasoned, not reproduced by a test); earlier sends are not polled, since `poll` reports room only once three quarters of the buffer are free and that halves a large frame's rate. On Windows the pipe is not polled (the call returns at once); the write timeout bounds each send instead: a pending pipe write is cancelled at it, and while a timeout is set a send writes at most 16 KiB, because only a write no larger than the pipe's buffer is sure to have sent nothing when cancelled (measured, #1089; a larger one can be partly taken and still report 0). The holder link runs on Unix only until ADR-0079 W2. A receive on a stream whose peer has already shut the connection (macOS refuses a new read timeout with `EINVAL`) goes on without the timeout, decided in `LocalStream::set_read_timeout_or_closed`, which takes `EINVAL` for that case only when `poll(2)` shows the stream hung up; every other failure of that call, and `EINVAL` on a stream still open, fails the read. The holder drops that node: a node that stops reading, or takes a frame bit by bit, cannot hang it (tested).
- `agora hold` refuses to start (it exits 1, `hold-failed`) when it cannot become a session leader (`setsid` fails with `EPERM` in a
  process-group leader); it never runs on in its starter's session.

## How the node uses it (W2 step 2)
- **Where:** `agora_node::holder` (`HolderHub`, `HolderSession`), `Supervisor::launch_held`, `Supervisor::adopt_held`, and
  `agora_platform::held_pty` (the terminal of a held agent as a `Pty`). Unix only (`agora_platform::HOLDER_BUILT`); elsewhere nothing
  here runs and the node runs its agents itself.
- **Waiting:** a start that needs the holder holds the supervisor while it connects (`Busy` for at most 1 s) and while a holder it
  started comes up (at most 3 s); when it cannot be had the node does not try again for 30 seconds. The supervision tick reads the
  holder's identity without waiting for a start.
- **Starting the holder:** by the first agent that needs one, never at the node's start (a node that starts nothing leaves no idle
  holder): `agora hold --home <home>` through `spawn_detached`, whose child calls `setsid` before `exec` (a start where `setsid` fails
  is an error and means "no holder here"; never `spawn_bound`). The node connects (retrying `Busy` for a few seconds, never reading it
  as a second node), sends `Attach` (a new holder has nothing to hand over), then `Start`. A holder that started but cannot be used (it
  does not admit this node, a different version) is sent `SIGTERM` and the node runs the agent itself; the node does not try again for
  30 seconds. `agora node run` only uses a holder when its own process would be admitted (the lock names it or its parent): otherwise
  a holder it started would refuse it and idle.
- **What goes through it:** only an agent the node does not confine (`cfg.sandbox` is `None`). A confined agent (Landlock, Seatbelt) is
  started by the node as before: the holder does not carry a confinement yet, and `Start` is unchanged. A start the holder refuses
  (`Failed`: the program is not there, or the holder is full; `AgentExists`) is the node's own start's to report: on `Failed` the node
  runs the agent itself, so an agent past `MAX_HELD_AGENTS` still starts, it just does not outlive the node.
- **Agent ids:** the node sends no id the holder would refuse (`holder_link::agent_id_fits`): an agent whose id is longer than
  `MAX_AGENT_ID` (none the node makes; a resumed agent keeps its id from the ledger, which takes up to 4 KiB) is run by the node itself, terminal or adapter, never refused.
- **Environment:** the node sends the complete environment `spawn` would have given the agent (its own, minus the removed names,
  plus the kind's and the node's additions). The link carries text: when a variable is not UTF-8 the agent is **not** started through
  the holder (the node runs it itself), never with a smaller environment than the node's own start gives it.
- **An ambiguous start:** when the link fails after a `Start` was sent, the holder may have started the agent. The node asks the holder
  (`List`) before anything else: an agent it has under that id is ended (stop, wait for the exit, `Release`) and only then is another
  start made, so two copies never run. If that cannot be settled and the holder's process is alive, the start fails; if the holder's
  process is gone the node starts again. When the link failed with anything but a reply that timed out (end of stream, a broken
  pipe, a reset, or `EPROTOTYPE`, which macOS can give for a send to a socket whose peer is closing), the holder is given up to a second
  for its process to end, and the node goes on only once it has: a holder ends right after it answered the `Release` of its last agent, so
  the restart of an agent that crashed, whose end the node collected first, can meet a holder that is closing but has not exited yet.
  Once it has, the node starts a new holder and the restart succeeds (#951: before, that first restart failed and one crash cost two
  restarts from the give-up budget). The same applies to a pipe agent (below).
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
  redraw, registers its mesh endpoint again at the same path, and clears the interruption (`Ledger::reclaim`). The mesh endpoint is made
  through the launch hook, so the hook is set before the agents are taken up (#906: it was set after, and a kept agent's socket was never
  served again, so its open `agora mesh stdio` bridge got `mesh.unavailable` for good). The new broker's ids (messages, tasks, leases,
  handoffs) start above every id an earlier run handed out (reserved on disk first, `MeshServer::reserve_ids`, see mesh-tools.md "Message ids"), so the
  `inbox` cursor the kept agent carried over still shows what arrives for it (#965: they started over at `m-1` and every new message,
  presence notice and message from another machine was hidden behind it). After the node's `NodeOnline` (which tells the hub
  that everything the node ran before is gone) and its `CheckoutDiscovered`, each agent taken up is said up again as a resync
  announcement says it: `AgentStarted`, `AgentStateChanged` (the state the ledger knew), `AgentSessionReported` when the ledger has a
  session, and `TerminalOpened` (`EventSink::agent_taken_up`, re-stated, so the ledger does not learn it again). Without them the hub, the
  window and the terminal client showed the kept agent finished (#907). `AgentStarted` carries the launch options the agent was started
  with and whether it acts without asking (ADR-0059), from the ledger (`Entry::launch_options`, `skips_permissions`, recorded from its
  first `AgentStarted`; `Supervisor::set_kept_launch`): it is the same process. An option this node's manifest marks as skipping, or no
  longer lists, counts as skipping too. The options' arguments are not known again, so a restart after a crash runs without them, as a
  resume does, and the restarted agent is shown without them. An agent that ended while no node was attached is released and
  stays interrupted (offered for resume); its status and output are not replayed.
  **Nothing is left attached and unread:** `Attach` made the holder stop reading each terminal and forget its buffer, and the node drops
  a master it does not take, so a running agent left like that would block on a full terminal for good. Every held agent the node does
  not take up (the ledger does not know it or names another holder, its folder or project is gone, or taking it up failed) is ended
  through the holder (stop, wait for the exit, `Release`). If one cannot be settled the holder is not used for new agents for 30
  seconds, and the next attempt ends it first. The same happens when a new session finds agents its own broken link left behind.
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
  service` on Linux the node's unit has `KillMode=process`: stopping or restarting it signals only the supervisor, which passes
  `SIGTERM` on to the node (as `agora down` does); the holder and the agents are in sessions of their own and keep running, and the
  new node attaches to the holder. systemd logs "left-over process" for them: expected. The hub's unit keeps `control-group`. A unit
  written by an earlier version is rewritten by `agora service install` (a unit Agora wrote and whose text would now differ; one
  without Agora's first line is never replaced), which reloads systemd but never restarts the service: the running node keeps
  `control-group` for the holder it has until `systemctl --user restart agora-node`, and the restart itself ends that old holder once. Logging out without lingering still ends the user manager
  and with it everything left in its cgroup. macOS needs nothing (`AbandonProcessGroup` false, agents in their own sessions). (3) macOS is compile-checked only; Windows
  has no holder yet. (4) A node whose holder link breaks while the holder lives counts the agents ended (it can no longer reach
  them) and ends them in the holder when it next connects. (5) A different holder version is refused and the node runs its agents itself; the "agents running in an older
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

## Pipe agents (ADR-0093, link version 2)

Appended, as the spec is: nothing above changes. Status: built for Unix in ADR-0093 build-order steps 1 and 2 (`crates/agora/src/hold.rs`,
`crates/agora-platform/src/held.rs` and `unixhold.rs`, `crates/agora-node/src/holder_link.rs`, `holder.rs`, `supervisor.rs`,
`crates/agora-adapters/src/host.rs`). Windows joins in step 3 with the same frames over the named pipe.

A **pipe agent** is a structured agent: an adapter or an ACP bridge, a program with a pair of pipes (stdin and stdout) and no terminal. The holder
starts it, keeps both pipe ends across a node restart, reads its output into a bounded buffer whether or not a node is there, and lets the
next node take the output over. **The holder never parses what the agent writes**: a line is bytes up to a newline, and what is in it (ACP's or
the adapter's JSON-RPC) is the node's to understand. The holder knows nothing of ACP, approvals or chat.

### Version negotiation
The frame, `Hello` and `Refused` are frozen, so version 2 is reached without a new frame: a node says `Hello(2)`. A holder that speaks 2
answers `HelloOk(2)`; a holder built before ADR-0093 answers `Refused(VersionMismatch, 1)`, and the node says `Hello(1)` on a new connection
(`Client::connect`; `Client::version` says what was agreed). **Version 2 is version 1 plus the tags below** and nothing in version 1 changes:

- A holder speaks 1 and 2 (`version_spoken`). To a node that said `Hello(1)` it never sends a new tag, and it answers a new request with
  `Refused(BadRequest)`. This is what lets a node built before this version keep working with this holder.
- A node never sends a new tag to a holder that answered 1: an older holder closes the link on a tag it does not know, which would cost the node
  its terminals (decision 5 above: `Request::decode` failing is "not served"). `HolderSession::pipes` says whether the holder keeps pipe agents; when
  it does not, the node runs a structured agent itself, as before.
- A different version from both is still `Refused(VersionMismatch, 1)` (the holder's own base version) and changes nothing.

### New frames
Node to holder: `StartPipe`(9: agent, argv, cwd, env), `AttachPipes`(10), `PipeRead`(11: agent), `PipeWrite`(12: agent, line).
Holder to node: `PipeStarted`(71: agent, pid), `PipeData`(72: agent, pid, state, `dropped: u64`, `eof: flag`, `seen: lines`, `fresh: lines`).
A line list is a `u32` count (never more than the bytes left) and, per line, `u32` length and bytes. A line is sent without its newline.
`Ok`, `Refused`, `Agents` and `AttachDone` are as before. Golden fixtures for the new frames are in `tests/fixtures/holder-link/` (`start-pipe`,
`attach-pipes`, `pipe-read`, `pipe-write`, `pipe-started`, `pipe-data`, `pipe-data-exited`, `hello-v2`, `hello-ok-v2`).

The framing of the link itself is the length prefix per frame; **a line is one length-prefixed item inside a frame**, so a line is never split by the
frame and the node never has to find a newline in the link's bytes. Limits as for every frame (1 to 4 MiB): a frame that is empty, larger than
`MAX_FRAME`, has an unknown tag, a cut or trailing body, a count that lies or a flag that is not 0 or 1 is `Malformed` and the link is closed
(tested with the real holder, `crates/agora/tests/hold_pipes.rs`, and for every cut of a frame in `holder_link_pipes_format.rs`). A line a node
wants to write that cannot fit one frame fails that write on the node's side, before anything is sent: the link stays whole.

### Order and meaning
1. `Hello(2)`, then `Attach` and `AttachPipes`, as for terminals (everything but `List` waits for `Attach`; `AttachPipes` is `NotAttached` before it).
2. `StartPipe` runs `argv` in `cwd` with **exactly** `env` (as for terminals: the holder passes nothing of its own), stdin and stdout piped, stderr the
   holder's own (its log), in a session of its own (so a process group of its own, as a terminal agent). Every descriptor beyond 0, 1 and 2 is
   close-on-exec. The launch is the node's: it is sent the complete environment `Adapter::spawn` would have given (`Adapter::held_env`); a variable that
   is not UTF-8 means the node runs the adapter itself. `Failed`: the program is not there (the node's own start says why); `AgentExists`: the id is in use;
   `BadRequest`: the id is empty or longer than `MAX_AGENT_ID`, as for `Start`.
   `argv` and `env` are the node's launch, never ACP: the holder cannot tell an adapter from any other program.
3. **Output.** A thread per agent reads stdout into a buffer of at most 1 MiB (`HELD_BUFFER`) of whole lines, oldest dropped first. A line longer than
   1 MiB (`HELD_LINE_MAX`) is dropped and counted. Empty lines are nothing. A last line without a newline is a line when the output ends.
4. **Dropped bytes.** Bytes pushed out of the buffer *before any node was given them* are counted, and `PipeData.dropped` says how many since the last
   hand-over, once. This is the explicit "dropped N bytes" marker. Kept bytes plus dropped bytes are every byte the agent wrote (tested exactly, with 3 MB
   written to a holder nobody read). The node tells the person (`adapter.output_dropped`, a notice with the count in `bytes`, after the replay).
5. **Taking over.** `PipeRead` answers the lines no node has had (`fresh`) and marks them given. The holder **keeps lines it has given** until 1 MiB of newer
   ones pushes them out. `AttachPipes` answers, per pipe agent, **all** it holds: the lines an earlier node was given as `seen` and the others as
   `fresh`; then `AttachDone`. A line is `fresh` once: what a node was given is never `fresh` again. Delivery is at most once, as for terminals ("what was
   sent into a socket nobody reads is the one unavoidable loss"): a reply that is written to a node that dies before using it is lost. `eof` is true when the
   agent's output has ended; with the agent's state `Exited` and no line left, nothing more will come.
6. **Input.** `PipeWrite` queues one line (the holder adds the newline) for a thread of its own, up to 256 lines, so an agent that does not read never holds
   the holder. A full queue, a closed input or an agent whose end the holder collected is `Refused(Failed)`; an unknown agent `Refused(UnknownAgent)`. `Resize` of a pipe agent is `BadRequest`.
7. **Ending.** `Signal`, `Stop`, `List` and `Release` work on pipe agents as on terminals (a pipe agent is in `List`). `Release` of an ended agent forgets it;
   a running one is `BadRequest`. A holder with no agent left ends (decision 2). `SIGTERM` to the holder (`agora down --agents`) ends the pipe agents too.
   If the holder dies, the agent's stdin reaches end of file (nothing guarantees more; there is no `PDEATHSIG` on pipe agents).

### How the node uses it
- **Where:** `HolderSession::start_pipe`, `pipe_io`; `Supervisor::launch_adapter` (through `launch_held_adapter`), `adopt_held_pipe`; `Adapter::attach_held`,
  `Adapter::detach`; `agora_node::daemon::take_up_held`. Unix only; elsewhere nothing here runs.
- **What goes through it:** an adapter the node does not confine (`sandbox` is `None`), also when the node restarts it after a crash or resumes it. A
  confined adapter is the node's own, as the confined terminal agents are.
- **Reading and writing:** the adapter host's reader reads a stream that polls the holder (`PipeRead`, every 2 ms after a line, backing off to 40 ms when
  idle: a line waits at most about 40 ms); its writer writes lines to the holder (`PipeWrite`). The host, its JSON-RPC and everything above it are unchanged.
- **When the node stops cleanly** (`Supervisor::shutdown`): a held adapter is let go (`Adapter::detach`: reading and writing stop, the process is not ended);
  any other adapter ends, as before. **When it is killed** the link closes and the holder keeps buffering.
- **When the next node starts:** `take_up_held` attaches, then attaches the pipe agents. For each running pipe agent whose ledger entry names this holder
  (pid and start time) and is interrupted, it restores the agent (`restore`), reads the adapter's session id and capabilities the old node wrote to the ledger
  (`Entry::adapter_session`, `adapter_capabilities`; the node cannot ask the adapter again), starts a new adapter host over the holder's pipes (**no
  `initialize` and no `session/start`: the adapter still has its session**), makes the mesh endpoint again at the same path, and clears the interruption.
  It is said up again to the hub after `NodeOnline` as a terminal agent is (above), with its launch options, without `TerminalOpened`, and then with what
  it last reported of its session: `AgentSessionOptions`, `AgentSessionConfig`, `AgentCommands` (#932). The adapter does not report them again by itself,
  so the node keeps the last ones it recorded for each adapter agent in `<data>/sessions/kept-<id>.pb` (beside the ledger, owner-only; a report larger
  than the hub keeps, 64 KiB encoded, is not kept). The file goes when the agent ends or is interrupted (also when only the file has it), when an agent
  that runs restarts after a crash (a resume, which follows an end or an interruption, keeps what its new process reported before its start was recorded),
  and at every start for each agent not taken up, with any unfinished write. A failed removal keeps the report unavailable for the
  lifetime of the node and is retried on the next kept-session change or a clean flush. A new node rebuilds those marks when its
  startup sweep cannot remove a report for an agent it did not take up. The file has only what the old node recorded, and a
  report it had read (the holder counts it given) but not yet recorded when it stopped or was killed is not in it (#1080). So on
  take-up the newest `session/options`, `session/config` and `session/commands` of the agent's session (`Entry::adapter_session`, or a
  report that names no session) among the `seen` lines replace the file's copy of that kind; a kind with no report there keeps the
  file's (ADR-0104 decision 1). Only the adapter's own reports set these and each replaces the last, so the newest one the holder still
  has is at least as new as the file's. The node puts the reports back on the agent (bounded and judged again as a report is) and
  re-states them from the supervisor's copy (`Supervisor::session_events`), which also records them in the file again, so the
  window's badge matches `skips_permissions_now`. A resync announcement re-states them after `AgentStarted` too. A mode or config
  option in use that makes the agent act without asking (switched to after the start, by the person or by the agent) is so shown and checked
  (`skips_permissions_now`) until the agent reports new ones. Its `AgentStarted` says `same_process` (ADR-0104 decision 2), so the hub and the
  window keep its tokens, model, context level and own cost (#930); usage the old node was still gathering (up to 5 s) is lost only when it was
  killed: a clean stop logs it first (below).
  Its earlier conversation is shown again (ADR-0104 C1, #931): before the node reads anything new from the holder, it fills the agent's chat
  ring from the end of the transcript the old node wrote (`<data>/transcripts/<id>.jsonl`, ADR-0089), numbers on after the file's highest
  id, rebuilds the tool calls and the plan that change in place, shows a permission entry still pending there as cancelled (the request is
  asked again under a new id, below), and drops a prompt still queued there (its queue ended with the old node; one the old node sent in
  its last unwritten second is said dropped too). The transcript is
  written in batches at most a second apart (while the node holds a secret value, the agent's newest words in a turn wait for what
  follows or the turn's end), so a kill loses at most those (a clean stop writes them all first, its end masked as an edge, below):
  they were shown live, are not in the file,
  and their ids may be given again by the next node (each window holds the next node's snapshot by then). Chat lines a killed node had read
  from the holder (the holder counts them given) and had not yet recorded are lost too, as before.
- **A clean stop (ADR-0104 O2, Unix):** `agora node` catches `SIGTERM`, `SIGINT` and `SIGHUP` (`agora down`, a service stop or restart, the
  restart after an update, Ctrl-C), so a planned stop is no longer a kill. From then on control commands (local and the hub's) are refused
  with `node.offline`, the node resumes no waiting agent and starts no helper (#1119), and the supervision loop takes nothing more; each
  adapter the holder runs is let go (`Supervisor::let_go_held`: its reader stops asking, hands over the lines it already had and ends).
  That needs the supervisor's lock, which a start or a resume holds while it launches an adapter (up to 30 s): it is waited for at most
  1 s (`LOCK_WAIT`); past it, the transcript's last batch, the usage and the ledger are written at once, and the lock is tried again until
  2 s into the stop. Still busy then, nothing is let go: the held adapters see the link close, as at a kill (#1119). A call still waiting
  on a let-go adapter for its answer (a prompt, a queued prompt being sent, a person's decision being delivered) whose line was written
  was given: it is answered `Detached`, and it is recorded as sent or decided, never taken back, dropped or recorded as not delivered. One
  whose line was not written yet when the reader ended is answered `Exited` (not delivered) and its line is never written; one being
  written is answered when that write returns (#1119). The commands already running and the loop's last round are
  waited for (at most 1 s, `LOOP_WAIT`), and what the adapters had read and the node had not recorded is recorded as the loop would
  (states, options, config, commands, usage and chat; a waiting request is not, the next node asks it again from `seen`). Then the transcript's last batch, all the usage still being gathered and the ledger with the
  kept session files are written, and the process ends as the signal would have ended it (so its supervisor sees the same end). No agent
  is ended: the holder's run on, and the node's own are left as a killed node leaves them (ADR-0060); no other thread is waited for. The
  readers are waited for at most 2 s (`DRAIN_WAIT`; a read of a holder that does not answer waits up to the link's frame time, 5 s) and
  the whole stop at most 5 s (`NODE_STOP_DEADLINE`), after which the process ends anyway; both are under `agora down`'s 10 s wait and
  systemd's `TimeoutStopSec=25`. A request before the node is up (still starting), and a second request, end the process at once, as
  before; a signal the node was started ignoring (`nohup`, a background job) stays ignored. A request less than 1 s after the first is
  that first one reaching the node again, not a second (#1119): Ctrl-C on a foreground `agora supervise node` signals the node and its
  supervisor (one process group), and the supervisor passes its own on as `SIGTERM`. `agora down` and the systemd unit
  (`KillMode=process`) signal the supervisor only. launchd, from its documentation (`launchd.plist(5)`; not measured on macOS), stops the
  job with `SIGTERM` and `SIGKILL` after `ExitTimeOut` (the page does not say whether to the job's process, the supervisor, or to its
  whole process group), and with `AbandonProcessGroup` false kills what is left of the job's process group once the job died, which is
  after its node. Either way the node gets one request. An adapter that really ended just before the stop, whose end the loop had not taken, is let go like the others and its end is
  not recorded (the stop cannot act on it): the next node finds it ended in the holder, does not take it up, the holder lets it go, and
  the ledger offers it for Resume (#1119). Windows ends the node with
  `TerminateProcess` and has no clean stop.
  An agent the node does not take up (unknown to the ledger, another holder, ended, its folder gone, no session recorded) is ended through the holder.
- **Rebuilding what waited on the person (decision 3):** the `seen` lines are read once (`agora_adapters::host::replay`, which also gives the newest
  reports above; `pending_approvals` is its requests), which keeps an `approval/request` unless a later
  `state/update` shows the agent went on (a state other than `blocked`); the `fresh` lines are read as if they had just arrived. A kept request becomes an
  `ApprovalRequest` event exactly as a new one, so the person is **asked again**: the node never answers it, and no line of the replay is a decision. A
  request the old node had already shown and the person had already answered shows the agent going on and is not asked again; one that cannot be told apart
  (a holder that kept less than the turn) can be shown after it was answered, and the answer then reaches an agent that no longer waits (the adapter answers
  with an error), never the other way round.
- **Without a holder:** unchanged. A structured agent is the node's own, ends with it, and is resumed (ADR-0060).

### Limits, plainly
- A request the old node had read, before it was killed and with fewer than 1 MiB of output after it, is rebuilt. A request more than 1 MiB of output back is not
  (the agent waits and the person can cancel the turn).
- Lines in flight to a node that dies are lost (at most once). What a killed node had *not yet turned into an event* is lost with it, except what
  is taken from `seen` (a waiting request, and the newest options, config and commands). A clean stop records it first (above), except what a
  reader still waiting on a holder that does not answer is handed after the stop stopped waiting for it: the holder counts those lines given,
  so they are lost as with a kill.
- Mode, model, configuration and command lists come from the kept file and the newest report in `seen` (ADR-0104). A report the old node read and did
  not record, when 1 MiB of newer output then pushed it out of the holder, is not known until the adapter reports again. Chat, usage and state are
  never taken from `seen` (they append, so reading them twice would show or count them twice): those a killed node read and did not record are lost.
- "Kept" against "continued" in the window (ADR-0093 decision 4) is **not built**: the snapshot has no field that says it without a schema change (`still_running`
  says the process runs, not that it survived a node). Left for build-order step 4.
- **Delivered is forgotten as new.** A line is marked given when the holder takes it for a reply, before the reply is written. A reply that then
  fails (the link broke) loses those lines as `fresh`; they stay available as `seen`, from which only `approval/request` lines and the newest
  options, config and commands are taken. Anything else in them (chat, state, usage) is lost to the next node.
- **Evicted but delivered is not counted.** `dropped` counts only bytes pushed out of the buffer before any node was given them. An
  `approval/request` that a node had been given and that 1 MiB of newer output then pushed out is neither rebuilt nor counted. (Counting it would
  need a new field in `PipeData`, a new tag; not done.)
- **The bound is on encoded size** (each line costs its bytes plus 4), so everything kept, as `seen` and `fresh` together, fits one `PipeData` frame
  whatever the lengths of the lines (a flood of one-byte lines is tested).
- **An ended agent's group:** when a pipe agent has ended, the holder sends `SIGKILL` to what is left of the agent's process group **before it
  collects the agent's end** (`waitid` with `WNOWAIT`: until then the ended agent keeps its pid, so the group's number is certainly still the agent's),
  and only then collects it, so a crashed adapter cannot leave processes running that hold its output open. Only an end counts (the
  child exited, or a signal killed it, by `siginfo_t.si_code`): a stopped agent (`SIGSTOP`, a debugger) is not ended. `waitid` with only
  `WEXITED` reports no other state on Linux or macOS, so this is a defence, not a fix of an observed case. On Linux it also ends what is left of the agent's **session** (what the agent moved into process groups of their own: job control,
  `setpgid`); elsewhere only the group. A process that left the session (`setsid`) is not found and can keep the output open: the node then
  counts the agent ended after a short grace without output.
- **A node that went back to link version 1** still sees pipe agents in `List` (they are agents of the holder) but does not end them: a version 1 node ends only
  what `Attach` handed it, and `Attach` never lists pipe agents (a `Stop` it sent by name would work, but it never knows the names); they run until the
  holder gets `SIGTERM` (`agora down --agents`).
  A node of this build never does this to itself.
- Confinement (ADR-0093 decision 6) is as for terminals: a confined adapter is the node's own.
- Windows has no holder (step 3).

### Telling the person about an older holder (ADR-0093 "Build status")
A holder started before link version 2 keeps running until every agent it keeps has ended, so a node of this build can negotiate down to
version 1 and then runs structured agents itself: a node restart ends them (automatic resume brings them back). `agora doctor` and
`agora status` are not the node, so the holder does not answer them (admission above) and they never connect to it. Instead the node writes
what its own `Hello` agreed to `<home>/run/hold.link` (`version pid start`, owner-only, replaced atomically; `agora_node::holder_note`).
A reader names a holder as older only when the note says version 1 **and** that pid still has that start time; the number of agents it keeps
comes from the holder's own `run/hold.pid`. `agora doctor` then prints a warning (exit status unchanged) and `agora status` a line under the
node; there is nothing to print without a holder, with a version 2 holder, or on Windows. The node logs `holder.link_v1: ...` once, the first
time it starts a structured agent itself for this reason. Nothing here stops, restarts or signals the holder or reads an agent.

## Terminals inside the frame (Windows, M2.6)

Windows has no `SCM_RIGHTS`, so a held terminal cannot travel beside a frame. There it travels **inside** it as a reference to handles the
holder duplicated into the node (ADR-0079 decision 3: `DuplicateHandle`). `agora_platform::TERMINAL_IN_BAND` says which way a build goes:
true on Windows, false on Unix. The link version does not change: no older Windows holder exists, and a Unix holder never sends these frames.

### Frames
Holder to node: `StartedRef`(73: agent, pid, `terminal: bytes`) and `TerminalRef`(74: agent, pid, state, buffer-dropped flag, buffer,
`terminal: bytes`). They are `Started` and `Terminal` with the terminal reference as one more field.
- On Windows a frame that hands a terminal over is always one of these. `Started`(67) is never sent there, and `Terminal`(69) only for an
  ended agent, whose frame carries no terminal (as on Unix).
- A node whose terminals travel beside the frame (Unix) refuses 73 and 74 as `Malformed`, as it would an unknown tag, and closes the link.
- `terminal` is opaque to the link: 1 to `TERMINAL_REF_MAX` (64) bytes, refused as `Malformed` before anything reads it when empty or
  longer. Its layout is the platform's own. On Windows it is a version byte (1) and the two handle values in the node (the input's write
  end, then the output's read end), each a `u64` big-endian: 17 bytes.
- Golden fixtures: `started-ref`, `terminal-ref`.

### Handles on every path
- **The holder** duplicates both ends into the node through the handle it opened and checked by start time when it admitted that node
  (decision 5; `TerminalHandle::export_to`). If the frame does not go out (a send fails, the link drops first), it closes both duplicates
  in the node again (`DUPLICATE_CLOSE_SOURCE`), so nothing of an undelivered frame stays there. Once the frame went out, they are the node's.
- **The node** imports a reference only after checking it: the exact length and version, two nonzero and different values, both of them
  pipes (`TerminalHandle::import`). A reference that fails is `Malformed` and closes the link. Nothing of it is closed, since its values
  may name handles the node holds for something else; a wrong reference is a holder fault, and the holder was admitted by pid and start
  time.
- **What can still be lost:** a frame that went out but that the node never reads, from a node that keeps running (it closed the link
  first). Then the two handles stay in the node until it exits: two per terminal and per attach, never more. A node that ends takes its
  handles with it.
- **Resize** never reaches the pseudoconsole through the node's copy (an `HPCON` stays in the holder). The node sends `Resize`(4) instead.
