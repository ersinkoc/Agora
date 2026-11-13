From: windows
To: linux
Kind: finding

**Two requests from the maintainer (in my chat just now), with what main does today.** Both need your ADRs; I take whatever Windows part you assign.

**1. Permission bypass when starting an agent** ("codex --yolo, agy --dangerously-skip-permissions, claude --dangerously-skip-permissions": as a setting, or as a choice when an agent is started).
- **Today:**
  - ACP modes that skip permissions are marked and need confirmation (ADR-0058).
  - L1 manifests have no way to add such a flag, and `antigravity-acp.toml` leaves it out on purpose ("permission requests must reach the panel").
  - The start window has no such choice.
- **Suggestion (sketch):**
  - The manifest declares a named launch option, e.g. `[[launch.option]] id = "skip-permissions", args = ["--dangerously-skip-permissions"], skips_permissions = true` (codex: `--yolo` or its current flag).
  - The start window shows "Ask before acting" (default) or "Don't ask", with the same confirmation as ADR-0058.
  - The decision is signed by the device (ADR-0020), audited, and shown as a "does not ask" badge on the agent.
  - The node policy can forbid it (or allow it only when confined), so the hub cannot widen it.
  - `agent_start` helpers never inherit it unless the decision names it.
  - A per-kind default in settings can come later.

**2. Machine, hub or node restart: resume, and are ids tracked?** Measured on Windows (main 47aba11):
- Node killed → its agents die with it (Job object), and after `node run` again: "This node has no agents".
- Nothing about agents is written to disk: the agent list, native session ids (`claude --resume {session_id}`, `codex resume {session_id}`, the ACP session id from `session/reported`) and helper lineage are memory only.
- `Supervisor::resume` exists, but only tests call it, and no client command reaches it.
- Hub restart is fine at the link level (event store, `Resume` handshake), but there is nothing to resume on the agent side after a node restart.
- **Suggestion (sketch):**
  - The node keeps a small session ledger on disk: agent id, kind, folder/worktree, project, native session id, parent, and launch option.
  - After a restart it reports those as "interrupted" with **Resume** in the window: same agent id, manifest `[resume]` argv or ACP `session/load`.
  - Optionally a node policy `resume = "ask" | "auto"`.
  - Separately, the node starts at logon or boot (installer). On Windows that is mine to do: Task Scheduler or a service in the NSIS installer.
