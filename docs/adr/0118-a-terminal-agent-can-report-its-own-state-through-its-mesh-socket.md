# 0118. A terminal agent can report its own state through its mesh socket
Status: Accepted
Date: 2026-11-26
Accepted: 2026-11-27 (maintainer approval in chat).

Touches ADR-0006 (append-only: no change to `schema/`), ADR-0020 (approvals are user-signed and verified on the node), ADR-0031 and
ADR-0075 (the per-agent mesh endpoint), ADR-0047, ADR-0050 and ADR-0083 (confined agents and the sockets they may reach), ADR-0057
(typing a helper's first prompt needs the live screen), ADR-0060 (session ids on a command line), ADR-0094, ADR-0095 (a dispatch looks
at the live screen), ADR-0117 (screen detection timing), PR #1198 (the `reported` flag), `docs/specs/agent-manifest.md` (`[hooks]`),
`docs/specs/mesh-tools.md`, `docs/specs/error-codes.md`, `crates/agora-core/src/agent.rs` (`Agent::observe`) and the threat model
(T4, T6, T11).

## Context

A terminal (L1) agent's state comes from its screen. Structured agents (ACP through the bridge, L3 adapters) report their own state,
and that report wins: `Agent::observe` ignores every screen reading once a state was reported, until the process restarts. The node
has `Supervisor::report_state` for "a state the agent itself reported (hook, ACP, adapter)", but no production caller for terminal
agents: only tests call it (`crates/agora-node/tests/supervisor.rs`). PR #1198 made the node say where a state came from
(`AgentStateChanged.reported`, already defined in `events.proto` as "True when the agent or an adapter reported it"). When only the
source changes (an agent report of the state the screen already showed), #1198 sends `StateChanged` again with `reported: true`; a node
fact that leaves the state as it was sends nothing, because a second `Blocked` or `Done` would be news to the hub's notifier, which
notifies on every `Blocked`/`Done` event and dedupes only for 60 s (`agora-hub::notify::DEDUPE_MS`).

The manifest has a `[hooks]` section (`install = "builtin:example"` in the spec's example) that the engine parses and ignores
(`crates/agora-adapters/src/manifest.rs`, P4-05 "Not done: hooks").

A comparable agent workspace, another terminal multiplexer for coding agents (its public design was read, no code was taken), installs hooks into the
configuration of 18 agents and lets any agent call `pane report-agent --source --agent --state --seq [-- <resume argv>]` (study of
2026-11-25, section G3). A sequence number drops late reports, and an approval visible on screen overrides a stale "working" from a
hook. Agents then get exact state without a change to that workspace.

What the agents offer, as far as checked here:

- **Claude Code 2.1.332:** `claude --help` lists `--settings <file-or-json>` ("Settings file or JSON string to load more settings
  from") and `--bare`, which skips hooks (checked 2026-11-26). Its hook events (`SessionStart`, `UserPromptSubmit`, `PreToolUse`,
  `PostToolUse`, `Notification`, `Stop`, `SessionEnd`), the JSON each hook gets on standard input (with `session_id`), and the
  `Notification` matcher values (such as `permission_prompt`) are from its documentation and **unverified** on a real run. A
  `PreToolUse` hook that exits with code 2 or prints a decision can block or allow a tool, so a hook Agora ships must never do either.
- **Codex 0.172.0:** `codex features list` shows `hooks` as stable and on, and `-c key=value` overrides any config value (checked
  2026-11-26). The config keys and event names of its hooks, its `notify` program setting, and whether a hook given by `-c` triggers
  the trust review dialog are **unverified**. Codex 0.167 showed a "hook needs review" dialog for a lifecycle hook in the user's config
  (fixture `codex/hook_review.screen`, read as blocked), so a shipped hook may make every first start wait for the person.

Constraints:

- The local control socket is the owner's full-power endpoint and is closed to confined agents (ADR-0082). Each agent already has its
  own owner-only mesh endpoint (`<node dir>/m/<agent id>.sock`, `docs/specs/mesh-tools.md`): whatever connects to it is that agent,
  confined agents may reach it (ADR-0083 allow-list), and `agora mesh stdio` already runs from inside the agent. On Linux another
  agent of the same user can still connect to it (threat model, ADR-0082 row).
- Mesh content is untrusted data and never approves anything (T4). A report is the agent's own word about itself, at the same trust
  level as what it prints on its screen.
- A wrong `idle` is the dangerous direction. A visible permission question must not be hidden by a stale report.

## Decision

1. **The channel is the agent's own mesh endpoint.** The node answers one new JSON-RPC method on each per-agent endpoint,
   `agora/report_state`. It is not an MCP tool: it is not in `tools/list`, so the model is not offered it as a tool. It is accepted
   on a fresh connection without `initialize`, answered, and the connection is closed. The same endpoint on Windows, with its
   per-agent identity.

2. **A per-run token in the agent's environment.** The node sets `AGORA_AGENT_SOCKET` (the endpoint's path) and
   `AGORA_REPORT_TOKEN` (32 random bytes, hex) for a terminal agent with verified, configured hooks. A replacement process
   receives a fresh token. A holder-backed process keeps its environment on reattachment. The node restores its token and
   report sequence only after proving the same run and endpoint ownership. Otherwise that process uses screen detection
   until its next launch. A report must carry the token of the agent's current run.
   - What the token does: a report from an earlier run (a hook still running after a restart) is refused; a report sent to another
     agent's endpoint without that agent's token is refused. That protects where an agent can connect to a neighbour's socket but
     cannot read the neighbour's environment: a confined agent on a Linux kernel without pathname-socket control (ADR-0083), when its
     confinement keeps it out of other processes' `/proc` entries; build step 2 checks which confinement profiles do.
   - What it does not do: an unconfined process of the same user can read another process's environment (`/proc/<pid>/environ` on
     Linux). Same-user, unconfined processes are not a boundary (T11); confinement is.
   - Everything the agent starts inherits the variables (its shell, its tools, a nested `claude -p`), so they can report as the agent.
     Decision 6 drops reports about another conversation.
   - Reporting is enabled for configured, verified hooks unless node policy sets `agents.self_report = off`. With the policy off,
     the node omits the variables and refuses reports.

3. **The report carries a state and an order, nothing else.**
   `{"token": "...", "state": "working" | "blocked" | "idle" | "done", "seq": <u64>, "session_id": "..." (optional)}`.
   - `unknown` cannot be reported. No resume command, no text, no approval, no permission decision: a resume command line passed by the hook is not
     taken (a command line from an agent would be a way to run something on the next restart; `[resume]` stays in the manifest).
   - `seq` orders reports of one run: a report whose `seq` is not greater than the last accepted one is dropped (hooks run as
     separate processes and can arrive out of order). `agora agent report` fills it with the wall clock in microseconds when the caller
     gives none.
   - Bounds: the request line at most 4 KiB; `session_id` must pass `argv_safe` (ADR-0060); at most 20 reports per second per agent,
     the rest refused.
   - A refusal is the stable error `agent.report_refused { reason }`, `reason` one of `token`, `stale`, `state`, `rate`, `session`,
     `disabled`, `not_terminal` (an adapter or ACP agent reports through its own protocol). A report that is accepted but changes
     nothing is answered as accepted.

4. **Precedence: the report is fast, the screen corrects it.** A hook report is a new source between the screen and an adapter's
   report. `agora-core` gets `StateSource::Hook`; `Agent::observe` keeps its rules for `Screen` and `Reported` (structured agents are
   unchanged), and adds:
   - **A screen `blocked` beats a hook.** A screen reading of `blocked` replaces a hook's `working`, `idle` or `done` at once. A
     visible question is the strongest evidence there is.
   - **A hook `blocked` holds** against screen readings of `working` or `idle` until a later hook report, or until input was written
     to the terminal and the screen then reads something other than blocked, or until the screen read the same other state on every
     check for `trust_ms`. If the screen matches no rule (a dialog the rules do not know), the hook's `blocked` stays.
   - **A hook `working`, `idle` or `done`** is replaced by a different screen reading only when every check read that state for
     `trust_ms` with no hook report in between. Screen `idle` agrees with a hook's `done` and changes nothing. This recovers from a
     missed hook (Claude Code runs no `Stop` hook when the person interrupts a turn, per its documentation; unverified).
   - **Node facts outrank both** (exit, stop and crash). A replacement agent process starts a new run and discards the old token
     and report state. Node restart with a surviving process follows the reattachment rule in decision 2.
   - `trust_ms` is `[hooks] trust_ms` (default 5000, 1000..=60000). ADR-0117's timing gate applies to screen readings only; a hook
     report is not flicker and is applied at once.
   - A screen reading that takes over sets `reported: false`; a hook report sets `reported: true`. The existing meaning of the field
     is kept.

5. **One event per real change, so no repeated notifications.** For the hook source, a report of the state the agent is already in
   sends no event, and the `reported` flag on the wire stays as it was (unlike an adapter report, where PR #1198 re-announces a change
   of source alone). Inside the node it still counts: the agent's source becomes `Hook` and the `trust_ms` window starts again, so the
   precedence of decision 4 follows the latest word whichever source spoke first. The wire flag therefore says where the state came
   from when it last changed. A screen reading of the current state sends nothing either. So `Blocked` or `Done` reaches the hub once per
   real transition, whatever mix of hook and screen produced it; a hook that fires twice for one dialog, or a hook and the screen
   agreeing a second apart, sends nothing more. Real repeats (blocked, answered, blocked again) are real transitions and go through
   the hub's 60 s dedupe as today.

6. **Session ids: only the agent's own conversation.** A report that names a `session_id`:
   - for a kind whose node chooses the id (`[session] id_args`, Claude Code), is dropped whole (state included, `reason = session`)
     when the id is not the one the node holds: it comes from another conversation, such as a nested `claude -p` or a subagent;
   - for a kind that learns its id from a file (`[session.file]`, Codex), sets the id only when the agent has none and no other agent
     holds it (the rules of `report_found_session`), and never replaces one; a report naming another id than the one held is dropped
     whole.
   A report without `session_id` is taken as the agent's own. Agora's own hooks are in a file only this process loads (decision 8), so
   a nested `claude -p` runs none of them. The check also applies to the person's hooks and other callers. Keep the session mismatch
   refusal through Claude Code's `/clear`. Review this decision after a real capture shows what changes and how Resume should follow it.

7. **A report never approves, types or dispatches.**
   - It creates and resolves no approval. A hook's `blocked` shows the agent as needing the person, as a screen `blocked` does; the
     question is answered in the terminal, as today.
   - Typing a helper's first prompt (ADR-0057) still needs a fresh look at the live screen matching `[[ready]]` and the rules
     detecting idle; a dispatch (ADR-0095) still uses `terminal_readiness`. A reported `idle` satisfies neither.
   - `agent_wait` (ADR-0057) waits on the agent's state as shown, so a helper reporting `idle` ends its parent's wait. The parent
     learns nothing more than that word; a helper could already print an idle-looking screen.

8. **The manifest's `[hooks]` section is interpreted (engine 7).** An older engine keeps ignoring it, so it needs no `min_engine`.
   ```toml
   [hooks]
   format = "claude_settings_json"         # the writer; one format per configuration shape (decision 9)
   flag = ["--settings", "{hooks_file}"]   # added to the launch command, as [mcp] cli_flag is
   session = "session_id"                  # optional: dotted path in the hook's stdin JSON that holds the session id
   trust_ms = 5000

   [[hooks.event]]
   name = "UserPromptSubmit"
   state = "working"

   [[hooks.event]]
   name = "Notification"
   matcher = "permission_prompt"
   state = "blocked"
   ```
   - The node writes `{hooks_file}` as a per-run file it owns (owner-only, removed when the run ends), which the agent may read and a
     confined agent may not write, never in the worktree and never in the person's home configuration. Where it lives under each
     platform's confinement is settled in build step 4: not inside a folder the node keeps closed to agents (its data, manifests or
     socket folder, #1114), so either the agent's private temporary folder or a folder of its own granted read-only. Passing the
     same JSON inline (`--settings` also takes a JSON string) avoids the file, but not where the program is started through a batch
     file on Windows, whose argument quoting is restricted (T45). The person's own settings and hooks still apply; Agora's are added
     for this process only and are not inherited by other runs of the tool.
   - Each hook command is `<agora> agent report --state <state> --event <name> [--session-from <path>]`, the absolute path of the
     running `agora` (as `{mcp_command}`). The command reads at most 64 KiB of the hook's standard input, takes only the `session`
     path from it, prints nothing, and **always exits 0** within its timeout (2 s), also when the socket is missing or the report is
     refused. It can therefore never block a tool, allow a tool, or add text to the conversation. The written hook entry sets the
     tool's own timeout to 5 s.
   - Validation: at most 16 events; `name` and `matcher` at most 64 printable characters; `state` one of the four reportable states;
     `flag` holds `{hooks_file}`; unknown keys refuse the section. `install = "builtin:..."` in today's spec example is replaced (it
     was never read).
   - A program in a run with those variables can also call `agora agent report` itself. The node applies the same run, session
     and ordering checks.

9. **Hooks shipped for Claude Code and Codex, each after a check on a real version.**
   - **Claude Code** (`format = "claude_settings_json"`: the `{"hooks": {EVENT: [{"matcher": ..., "hooks": [{"type": "command",
     "command": ..., "timeout": 5}]}]}}` shape, passed with `--settings`): `SessionStart` → `idle` with the session id,
     `UserPromptSubmit`, `PreToolUse` and `PostToolUse` → `working`, `Notification` with `permission_prompt` → `blocked`, `Stop` →
     `idle`. `--settings` is checked on 2.1.332. Verify the events, the stdin shape and the matcher on that
     version (a scratch folder, a one-word prompt and a refused `touch probe.txt`) before the manifest declares them. Record the
     version in a comment, as for launch flags. A start with `--bare` runs no hooks and falls back to the screen.
   - **Codex**: a format for `-c` overrides is added only once a real 0.172 (or later) run shows the hook keys, the events, and that a
     hook passed this way does not stop every start at the review dialog. If it does, no Codex hook ships and Codex stays on screen
     and title rules (PR #1204). Its `notify` program (turn finished) is the fallback to check.
   - WrongStack ships no hooks (none known); it has an ACP kind.

10. **Schema and protocol: no change to `schema/`.** `AgentStateChanged.reported = true` already means the agent reported the state;
    the hub, the window and the event store need nothing new. Additions, all append-only: the local method and the two variables in
    `docs/specs/mesh-tools.md`; the error code `agent.report_refused` in `docs/specs/error-codes.md`; `[hooks]` in
    `docs/specs/agent-manifest.md` (engine 7) with the precedence rules replacing "Reported state (hooks, ACP, adapter) always
    overrides rule results"; the policy line in `docs/specs/protocol.md` beside the other node policy lines; a new threat-model row.

## Alternatives considered

- **The local control socket (`agora agent report` as an owner command).** It is the owner's endpoint, closed to confined agents and
  able to do far more than set one agent's state.
- **An MCP tool on the mesh endpoint.** The model sees every tool in `tools/list` and can be told by injected text to call it; a
  method only the hook command uses keeps it out of the model's tool list. (A model that runs shell commands can still run
  `agora agent report`; decision 4 bounds what that achieves.)
- **A separate report socket per agent.** One more endpoint per agent, one more sandbox allow-list entry, the same identity.
- **Writing hooks into the person's configuration (an "integration install" command).** It edits files the person owns, applies to every
  run of the tool including runs outside Agora and nested ones, and a write in the worktree replaces the file (`write_in_worktree`),
  which would drop the person's own project settings.
- **Hook reports as sticky as an adapter's.** A stale `working` would hide a permission question on screen until the process ends.
- **Screen always first, hooks only fill "no opinion".** Loses most of the gain: the screen already has an opinion most of the time,
  often a slow or wrong one.
- **A token alone, over loopback TCP.** Any local user could connect; the per-agent socket is owner-only and is what confinement
  already allows.

## Build order

1. `agora-core`: `StateSource::Hook` and the precedence of decision 4, with unit tests (no I/O, injected clock for `trust_ms`).
2. `agora-node`: the method on the per-agent endpoint, the per-run token and the two variables, the refusals and the rate limit, the
   session rule, the policy line; `agora agent report` in the `agora` binary. Spec, error code and threat-model row in the same commit.
3. `agora-fake-agent` gains a step that sends a report; integration tests below.
4. Manifest engine 7 `[hooks]` and the `claude_settings_json` writer; the Claude Code manifest's hooks after the check on a real
   version, with a golden file of the written settings.
5. Codex, only after its check (decision 9).

Ship each step in its own commit.

## Verification

- **Unit (`agora-core`):** screen `blocked` replaces hook `working`; hook `blocked` holds against screen `idle` and ends after input
  plus a non-blocked reading; screen `idle` replaces hook `working` only after `trust_ms` with no report between; hook `done` and
  screen `idle` agree; adapter reports keep their old rules.
- **Integration with `agora-fake-agent`** (never a real agent):
  - a report with the right token sets the state with `reported: true`; the wrong token, another agent's endpoint, and a token of the
    previous run (after a restart) are refused with their reasons and change nothing;
  - out-of-order `seq` is dropped and the 21st report in one second is refused with `rate`;
  - a verified holder reattachment keeps the last accepted sequence. An unverified reattachment falls back to screen detection;
  - two `blocked` reports and a screen `blocked` for one dialog produce exactly one `AgentStateChanged` in the event log;
  - a report naming another session id for an `id_args` kind is dropped whole;
  - a reported `idle` does not let the node type a helper's first prompt while the screen shows a dialog;
  - a confined agent (where the platform confines) can report through its own endpoint and cannot reach the control socket;
  - `agora agent report` with no socket, a dead socket, or a refusal prints nothing and exits 0 within 2 s.
- **Manifest:** validation of `[hooks]` (17 events, a bad state, a `flag` without `{hooks_file}`, an unknown key); the written
  Claude Code settings file equals its golden file; the file is owner-only, outside the worktree, not writable by a confined agent,
  and removed when the run ends.

## Open questions

The maintainer resolved these questions on 2026-11-27.

- Use `idle` for the ordinary Claude Code end of turn and five seconds as the initial trust window.
- Enable reporting for verified, configured hooks with the node policy opt-out in decision 2.
- Keep the existing client source label. Test same-state events at the node and hub before considering a second notifier filter.
- Keep Codex hooks disabled while their startup waits at the review dialog.
- Keep the session mismatch check through `/clear` and the holder reattachment checks in decision 2.
