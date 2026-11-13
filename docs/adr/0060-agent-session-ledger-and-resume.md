# 0060. A node keeps a session ledger and offers Resume after a restart
Status: Proposed
Date: 2026-11-13

Not accepted. Nothing here may be built until the maintainer accepts it in a message of their own.
Origin: the maintainer asked (via the Windows agent, 2026-11-13) what happens to agents when a machine, hub or node restarts, and whether ids are tracked.

## Context
Measured on Windows (main 47aba11) and true on every platform: when a node stops, its agents die with it (Job object / process group), and after `node run`
again the node "has no agents". Nothing about agents is written to disk: the agent list, native session ids (`claude --resume {session_id}`,
`codex resume {session_id}`, the ACP session id from `session/reported`) and helper lineage are memory only. `Supervisor::resume` exists but only tests
call it. A hub restart is fine (event store, `Resume` handshake), but there is nothing to resume on the agent side after a node restart.
Agents ending with the node is intended (nodes are autonomous, but agents are processes of the node); what is lost is the *conversation*, which the
agent tools themselves keep and can resume.

## Decision (proposed)
1. **A session ledger on disk**, in the node's data folder, owner-only: per agent id, kind, folder/worktree, project, parent agent id, native session id
   (when reported), launch options (ADR-0059), start time, last state, and whether it ended cleanly. Written when an agent starts, when its native session id is reported,
   and when it ends; never terminal contents, prompts, or secrets. Size-limited, old ended entries pruned.
2. **After a node restart the ledger entries that did not end cleanly are reported as "interrupted"** (new agent state/flag in the snapshot and an event),
   shown in the window under the machine with **Resume** (same agent id, same folder, same project; the node runs the manifest's `[resume]` argv or ACP `session/load`
   with the native session id). An entry without a native session id offers **Start again** (new session in the same folder) instead.
3. **Resume is a command**, `AgentResume { agent_id }` (new command, capability `agent.resume`), a signed decision like a start (ADR-0020), subject to the same node policy
   (including ADR-0059's) and folder checks (ADR-0052). The hub cannot resume anything by itself.
4. **Optional node policy** `resume = "ask" | "auto"` (default **ask**): `auto` resumes interrupted agents after a restart only for kinds and folders the owner listed in the node's configuration.
5. **Helpers** (ADR-0057) are resumed only when their parent is, and their approvals/grants are not restored (grants end with their parent; they are asked again).
6. **The node itself starting at logon or boot** is a packaging matter, not part of this ADR: per platform (systemd user unit, launchd agent, Windows scheduled task/service in the installer),
   as a follow-up of ADR-0056's `agora service install`.
7. The hub shows interrupted agents from the node's report and keeps them in the closed list with the usual "finished" wording until resumed; closed-agent summaries (ADR-0053) keep working.

## Notes from the macOS review (2026-11-13)
- **Claude Code's native session id (L1)** comes from its transcript (`~/.claude/projects/<folder>/<session>.jsonl`). With `CLAUDE_CODE_CHILD_SESSION` in the node's environment
  Claude Code writes no transcript, so there is no session id and nothing to resume: the environment a terminal agent is started with matters for Resume (check it before building).
- **Autostart on macOS** is a user LaunchAgent (`~/Library/LaunchAgents`, `RunAtLoad`, not a system daemon), so it runs as the user with their home and keychain. Risk to
  test first: macOS local-network privacy may refuse a launchd-started CLI that connects to a LAN hub without a prompt (seen with the unsigned desktop app,
  `NSLocalNetworkUsageDescription`).
- **The ledger file** is mode `0600` in a `0700` folder on macOS and Linux and is written atomically (temp file and rename in the same folder), like the event log.

## Open questions for the maintainer
- Default `resume = "ask"` (proposed) or `auto` for trusted folders?
- Is it acceptable that the ledger stores folder paths and session ids on disk in the node's data folder (owner-only)? (Proposed: yes; no prompts or terminal contents.)
- How long should an interrupted entry stay offered? (Proposed: 14 days.)

## Consequences
Schema: `AgentSnapshot.interrupted` + native-session fields, `AgentResume` command, `AgentInterrupted` event or state, capability `agent.resume`; node ledger (agora-node), supervisor wiring of the existing `resume`; clients (Resume / Start again); audit; threat model row (the ledger's contents; resume of a folder that changed).
