# 0093. Structured (ACP) agents survive a node restart
Status: Accepted
Date: 2026-11-20
Accepted: 2026-11-20 (the maintainer's explicit message in the merger chat: "ADR-0092 ve ADR-0093'ü kabul ediyorum"). The open questions' proposed answers stand.

Refines ADR-0060 (resume), ADR-0079 (the terminal holder) and ADR-0090 (a complete structured conversation). Touches ADR-0075 and ADR-0089.

Origin: measured on `blackview` on 2026-11-20. A node restart for 0.3.26 kept the terminal agent (Claude Code, same holder process) and
lost the two structured WrongStack agents. The window said the node "has no agents" for them, and automatic resume did not bring them back
because their kind was not in the node's `resume.kinds`.

## Context
- ADR-0079's holder owns terminals (a PTY on Unix, a pseudoconsole on Windows). A structured agent has no terminal: the node runs an adapter
  or an ACP agent as a child process and talks to it over stdio (a pipe pair). When the node ends, the pipes close and the agent ends.
  The holder never saw these agents, so "agents survive" is true only for terminals. This was not stated anywhere and was found by loss.
- Resume (ADR-0060) restores the conversation with `session/load` when the agent supports it, and only if the owner listed the kind.
  A turn that was running is gone, and so is the in-memory chat history until ADR-0089's persistent history is built.

## Decision
1. **The holder also keeps structured agents.** The same holder process starts the ACP or adapter child with pipes instead of a PTY and
   keeps both ends alive across a node restart. A restarted node asks the holder for its agents and receives, per agent, the bytes the
   agent wrote while no node was reading (a bounded buffer, oldest dropped, an explicit "dropped N bytes" marker), then the live stream.
2. **The protocol messages are ACP's own JSON-RPC lines**, so the holder does not parse them. Framing is a length prefix per line. The holder
   has no knowledge of ACP, approvals or chat.
3. **A turn in progress survives.** A permission request that was waiting when the node ended is still waiting; the restarted node
   rebuilds the pending approval from the replayed line, so the person is asked again (never auto-answered, ADR-0020).
4. **Without the holder** (Windows today, a holder that failed to start) a structured agent is resumed (ADR-0060), as now, and
   the window says "continued" rather than "kept", so the person knows the difference.
5. **The default for `resume.kinds`** gains every structured kind the manifest marks resumable, so the maintainer's file does not
   need a hand edit. The owner can still list or exclude kinds (policy stays the node's, the hub cannot widen it).
6. **Confinement is unchanged.** A confined agent (ADR-0084) still runs inside the node's sandbox; it is held only where the holder can
   enter the same sandbox, otherwise it is resumed. Stated per agent in the window.

## Build order
1. The holder starts and keeps a pipe-based child (Unix) and the node reattaches; test with `agora-fake-agent` through a node kill.
2. The replay buffer and the "dropped bytes" marker; pending approvals rebuilt from the replay.
3. Windows (named pipes in the holder, ADR-0079 Windows rung).
4. The window wording ("kept" vs "continued") and the doctor line.

## Open questions
- Replay buffer size. Proposed answer: 1 MiB per agent, oldest dropped.
- Should the holder keep a structured agent after `agora down` without `--agents`? Proposed answer: same rule as terminals.

## Consequences
Schema: none expected (agent snapshot already has `still_running`); a holder link message for pipe agents (ADR-0079's link spec gets a
section, append-only). Tests: node kill with a structured agent mid-turn, with a pending permission, with a full buffer. Docs: `docs/specs/holder-link.md`.

## Build status
- Steps 1 and 2 are built for Unix (the change that adds "Pipe agents" to `docs/specs/holder-link.md`): the holder starts and keeps pipe agents, the node
  reattaches, the replay is bounded to 1 MiB per agent with a "dropped N bytes" count, and the pending permission request is rebuilt from the replayed
  line and asked again, never answered. Link version 2 carries it, negotiated by `Hello` so that a holder or a node built before this ADR keeps working
  with the other. No schema change was needed.
- Open question answered as proposed: 1 MiB per agent, oldest dropped (whole lines).
- Not built: step 3 (Windows), step 4 (the window's "kept" and "continued" words and the doctor line; the snapshot has no field that says "survived a
  node restart" without a schema change, so that step needs one), and decision 5 (the default of `resume.kinds`).
- Doctor line, node-local (no schema change): a holder that speaks only link version 1 (built before 0.3.27) cannot keep structured agents, so a node restart ends
  them and resume brings them back. The node records the version it agreed in `<home>/run/hold.link`; `agora doctor` (a warning, same exit status) and
  `agora status` read it and say so, and the node logs `holder.link_v1` once. See `docs/specs/holder-link.md`. Still not built: the window's "kept" and
  "continued" words (needs a field in the agent snapshot, so a schema change and its own ADR step), Windows, and decision 5.
- Decision 3 amended by ADR-0104 decision 1 (#1080): on take-up the newest `session/options`, `session/config` and `session/commands` of the agent's
  session in the `seen` lines also replace the kept session file's copy, because they replace the last report and so reading one twice changes
  nothing. Chat, usage and state are still not taken from `seen`. See "How the node uses it" in `docs/specs/holder-link.md`.
