# The holder link (ADR-0079 decision 5)

Status: built for Unix in W2 step 1 (`crates/agora-node/src/holder_link.rs`, the server in `crates/agora/src/hold.rs`). Nothing in the node
uses it yet. Windows joins in a later step with the same frames and a named pipe.

The link is **private to Agora**: a node and its own terminal holder (`agora hold`) talk over it on one machine. It is not a protocol message
(`schema/` is for the wire between nodes, hubs and windows), and nothing outside this machine speaks it. It is versioned, small and has
golden fixtures (`crates/agora-node/tests/fixtures/holder-link/*.hex`, one frame's payload each; a fixture is added, never edited).

## Endpoint and who may connect
- A unix socket `<home>/run/hold.sock`, mode `0600`, in the `run` folder (`0700`). Windows: a named pipe with the owner-only descriptor.
- The holder reads who connected from the kernel (`SO_PEERCRED` on Linux, `getpeereid` and `LOCAL_PEERPID` on macOS): the peer must be
  the same user, its pid must be the pid in `<home>/run/node.lock` (ADR-0062 item 8), and its start time, read now, must be the lock's.
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
   with the PTY master attached (`SCM_RIGHTS`) for a running agent, and `AttachDone`. A running agent's buffer is given to the node and
   forgotten; an ended agent's status and buffer are sent without a terminal and **kept** until the node sends `Release` (it may die
   between the two). Terminals are never read by the holder while a node is attached, and it never writes to them.
4. `Start` returns `Started` with the new terminal attached. The launch is the node's: the holder adds and widens nothing. (The
   confinement a node builds is not carried yet; a later step adds it as a new tag.)
5. Loss of the link, or of the node's process (a pidfd on Linux, `kqueue` on macOS, else the start time), detaches: the holder reads each
   terminal into a 1 MiB buffer (oldest dropped first), so no agent blocks on a full terminal.
6. `Release` of an ended agent forgets it. A holder that held agents and has none left ends (decision 2).

A running agent gets `SIGHUP` when the holder dies (Linux `PR_SET_PDEATHSIG` on its first process; the kernel's hangup on the last
master) and nothing guarantees more: decision 8.

A terminal is read in exact sizes (the length, then the payload), so the descriptor sent with a frame's first byte is never taken for the
next frame's.
