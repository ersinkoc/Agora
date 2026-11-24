# 0060. A node keeps a session ledger and offers Resume after a restart
Status: Accepted
Date: 2026-11-13

Accepted: 2026-11-13 (maintainer's explicit approval in chat: "0059 ve 0060 kabul"). Decisions the maintainer gave (first to the Windows agent, then confirmed here): `resume = "ask"` is the default (a Resume button, nothing automatic); the other open questions (paths and session ids in the owner-only ledger, 14 days): the proposed answers stand.
Origin: the maintainer asked (via the Windows agent, 2026-11-13) what happens to agents when a machine, hub or node restarts, and whether ids are tracked.

## Context
Measured on Windows (main 47aba11) and true on every platform: when a node stops, its agents die with it (Job object / process group), and after `node run`
again the node "has no agents". Nothing about agents is written to disk: the agent list, native session ids (`claude --resume {session_id}`,
`codex resume {session_id}`, the ACP session id from `session/reported`) and helper lineage are memory only. `Supervisor::resume` exists but only tests
call it. A hub restart is fine (event store, `Resume` handshake), but there is nothing to resume on the agent side after a node restart.
Agents ending with the node is intended (nodes are autonomous, but agents are processes of the node); what is lost is the *conversation*, which the
agent tools themselves keep and can resume.

## Decision
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

## Open questions (answers as decided above)
- Default `resume = "ask"` (proposed) or `auto` for trusted folders?
- Is it acceptable that the ledger stores folder paths and session ids on disk in the node's data folder (owner-only)? (Proposed: yes; no prompts or terminal contents.)
- How long should an interrupted entry stay offered? (Proposed: 14 days.)

## Consequences
Schema: `AgentSnapshot.interrupted` + native-session fields, `AgentResume` command, `AgentInterrupted` event or state, capability `agent.resume`; node ledger (agora-node), supervisor wiring of the existing `resume`; clients (Resume / Start again); audit; threat model row (the ledger's contents; resume of a folder that changed).

## Note from the implementation (2026-11-13): an agent that outlives its node
Found on a real Mac: after the node is killed (`kill -9`) a terminal agent can keep running (it leads its own process group; `codex`
stayed with PPID 1), and Resume then started a second process on the same conversation. The ledger therefore also records each
agent's pid, process group and process start time (read in `agora-platform`: `/proc/<pid>/stat` on Linux, `proc_pidinfo` on macOS;
not needed on Windows, where the agents' job object ends them with the node). At the next start an interrupted entry whose pid still
runs with the same start time is reported `still_running`: Resume is refused (`agent.still_running`) until the owner stops it
(`agent.stop` ends exactly that process, its group when it leads one) or leaves it. A pid whose start time differs belongs to another
process and is never touched; neither is one whose boot (Linux `boot_id`) or program name (`comm`) differs from the recorded one, so
a ledger written by someone else cannot point a stop at another program. A node dropped in-process (tests, an embedding) ends its
agents itself. `agora node` had no signal handler when this was decided; since ADR-0104 O2 it catches `SIGTERM` (Unix) to record and write what it
held, and still ends no agent: under systemd's default `KillMode=control-group` its agents die with it anyway; a plain `kill` of the
node process leaves terminal agents running, and they show as still running at the next start.

## Amendment A (Accepted, 2026-11-15): an agent that was working carries on after an automatic resume
Accepted: 2026-11-15 (the maintainer's explicit message in the merger chat, accepting the ADRs listed by number (#613 ADR-0060 Amendment A, #614 ADR-0078, #615 ADR-0080, #616 ADR-0079): they accepted all of them). The open questions' proposed answers stand.
Origin: the maintainer, about restarts: "these agents should go on with what they were doing, without a break, as if they had never
dropped". Measured on `win` (0.3.13 release, `resume = "auto"`): after the node was killed, a Claude Code agent came back by itself in
its own session and remembered what it had been told, then **sat idle**. A task cut off in the middle does not go on until someone
types "continue". The ledger already keeps each agent's `last_state` (decision 1), so the node knows which agents were in the middle
of something.

**Change:**
1. When the node brings an interrupted agent back **by itself** (`resume = "auto"`, or `node run --resume auto`, ADR-0062 item 4) and
   the ledger's `last_state` for it was **working**, the agent is given one message that the session was cut off by a restart and that
   it should first check the state of what it was doing, then carry on where it stopped.
2. **Never otherwise.** It is not sent when the last state was *needs you*: text typed into a terminal that shows a permission prompt
   could answer it. It is not sent for idle, finished or unknown. It is not sent when a person pressed Resume, because they are there
   and can say what they want. A helper (ADR-0057) follows decision 5 and gets it only on the same terms as any other agent.
3. **The node's own policy decides; the hub cannot.** A policy line `resume.continue = on | off` in the node's policy file, default
   **on** wherever automatic resume is on. No command or protocol message carries it or its text, so a hub can neither turn it on nor
   change what is said (ADR-0020: the hub cannot widen node policy).
4. **The words are data, not code.** A kind says how it is continued in its manifest, under `[resume]`:
   - `continue_argv`, the resume command with a first message, for tools that take one (`claude --resume {session_id} "{continue}"`,
     `codex resume {session_id} "{continue}"`);
   - or, for an ACP kind, a `session/prompt` right after `session/load`.

   `{continue}` is a sentence in the manifest's `[resume] continue`. A kind without either is resumed as today and gets no message.
   No such text lives in node Rust.
5. **Once per interruption.** The ledger records when the message was sent, and it is never sent twice for the same interruption.
   The existing rule that an agent brought back by itself in the last 10 minutes is not brought back again also keeps a crash loop from
   repeating it.
6. **Nothing is skipped.** The message is ordinary input to the agent. Every approval and the node's policy apply as they did before the restart. Launch
   options (ADR-0059) are not carried by a resume: the node does not pass a "Don't ask" option again (ADR-0062 decision 4,
   `ledger.rs`). This does not by itself make the resumed agent ask: a structured agent can restore its own permission-skipping
   mode, and the node does not yet check that mode on a resume (ADR-0090 known gap, backlog P2-16). The message shows in the agent's own terminal and conversation like any prompt,
   and the node's log says it was sent.

**Consequences:** a manifest field (`[resume] continue` and `continue_argv`, `docs/specs/agent-manifest.md`), a ledger field (sent at),
a policy line, the node's automatic-resume path, tests with `agora-fake-agent` (working → message; needs-you, idle, a person's Resume
and `off` → none; at most once), and a threat-model row: *a message no person sent carries on a step that was half done* (a
migration, a push). The row's mitigations are only working agents, the text asks to check the state first, approvals still apply,
an off switch per node, and once per interruption. No schema change.
