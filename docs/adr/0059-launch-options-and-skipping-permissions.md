# 0059. Launch options, including "do not ask" modes, chosen when an agent starts
Status: Accepted
Date: 2026-11-13

Accepted: 2026-11-13 (maintainer's explicit approval in chat: "0059 ve 0060 kabul"). Decisions the maintainer gave (first to the Windows agent, then confirmed here): the default node policy is `skip_permissions = "allow"` (not `forbid`); "Don't ask" stays never preselected, with the confirmation, the signature, the audit and the badge, and helpers never inherit it; the owner can still set `forbid` or `confined_only` per machine. The other open questions: the proposed answers stand.
Origin: the maintainer asked (via the Windows agent, 2026-11-13) to choose at start, or as a setting, whether an agent runs with its
permission-skipping flag (`codex --yolo`, `agy --dangerously-skip-permissions`, `claude --dangerously-skip-permissions`).

## Context
Today the node can only start a terminal agent with its manifest's fixed `launch` argv; a structured (ACP) agent's
permission-skipping *modes* are marked and need a confirmation in the panel (ADR-0058), but nothing lets the person who starts an agent
choose a launch flag. `antigravity-acp.toml` leaves `--dangerously-skip-permissions` out on purpose so permission requests reach the panel.
A person who trusts a folder (a throwaway worktree, a VM) will want agents there to run without stopping at every request; a person who does
not must be sure no agent can be started that way by accident, by the hub, or by another agent.

## Decision
1. **A manifest declares named launch options** (additive to the manifest format; `min_engine` raised):
   `[[launch.option]] id = "skip-permissions"  label = "option-skip-permissions"  args = ["--dangerously-skip-permissions"]  skips_permissions = true`.
   The label is a Fluent message id. An option may also be harmless (`id = "resume-last"`, `args = ["--continue"]`). Shipped manifests declare
   the real flag of the version they were checked against (Claude Code, Codex, `agy`); a flag that was not checked on a real version is not
   declared. Structured (ACP) kinds declare options as extra argv of the ACP server command in the same way.
2. **The start window** offers each declared option; **"Ask before acting" is always the default** and "Don't ask" is never preselected. Choosing an
   option with `skips_permissions` shows the same confirmation as ADR-0058 ("Let the agent act without asking?", Keep asking as the primary button).
3. **The choice is a signed decision.** `AgentStart` carries the chosen option ids (new repeated field `launch_options`); the node verifies the device
   signature (ADR-0020) that covers them, so the hub cannot add an option. Every start with a `skips_permissions` option is audited with the option
   and shown as a "does not ask" badge on the agent in every client for its whole life.
4. **Node policy decides, per machine.** `skip_permissions = "forbid" | "allow" | "confined_only"` (default **allow**, as the maintainer decided; the owner sets it in the node's own
   configuration, never over the hub). The node refuses a start that violates it with a stable error code (`agent.option_forbidden`), and
   `confined_only` requires the agent's confinement (ADR-0047/0050) to be active, so a "don't ask" agent still cannot write outside its folder.
5. **Helpers never inherit it.** An agent started by `agent_start` (ADR-0057) gets no `skips_permissions` option unless the signed approval names it
   explicitly; a grant never covers it.
6. **Per-kind defaults** ("always start Codex like this") are a later, separate decision; this ADR only adds the per-start choice.

## Notes from the macOS review (2026-11-13)
- `confined_only` is a real choice on macOS: Seatbelt confinement works there, so a "don't ask" Claude Code or Codex under `sandbox = worktree` still cannot write outside
  its folder or to the shared `.git` config and hooks (#303). It is not yet a choice on Windows (confinement is disabled there).
- Every flag is checked on the real version before a manifest declares it. A skip flag may show a one-time acceptance screen the first time (Claude Code has shown a
  "Bypass Permissions mode" screen in the past); that screen needs a `blocked` rule and a fixture, or the agent waits and nobody is told. Capturing it is a macOS task
  once this ADR is accepted.

## Open questions (answers as decided above)
- Default node policy: **`allow`** (decided by the maintainer; `forbid` and `confined_only` stay available per machine).
- May a structured (ACP) agent's *mode* that skips permissions also be chosen at start (not only switched later)? (Proposed: yes, same confirmation.)
- Should the "does not ask" badge also require the node's confinement to be reported to the window? (Proposed: show both.)

## Consequences
Schema: `AgentStart.launch_options` (new repeated field, append-only), an audit field, `AgentSnapshot.launch_options` for the badge, error code
`agent.option_forbidden`, capability `agent.launch_options`; manifest format + `docs/specs/agent-manifest.md`; node policy file; clients (start window, badge); shipped
manifests with checked flags; threat model row (a person or agent starting an agent that does not ask).
