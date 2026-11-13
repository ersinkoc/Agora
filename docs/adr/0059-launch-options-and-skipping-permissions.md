# 0059. Launch options, including "do not ask" modes, chosen when an agent starts
Status: Proposed
Date: 2026-11-13

Not accepted. Nothing here may be built until the maintainer accepts it in a message of their own.
Origin: the maintainer asked (via the Windows agent, 2026-11-13) to choose at start, or as a setting, whether an agent runs with its
permission-skipping flag (`codex --yolo`, `agy --dangerously-skip-permissions`, `claude --dangerously-skip-permissions`).

## Context
Today the node can only start a terminal agent with its manifest's fixed `launch` argv; a structured (ACP) agent's
permission-skipping *modes* are marked and need a confirmation in the panel (ADR-0058), but nothing lets the person who starts an agent
choose a launch flag. `antigravity-acp.toml` leaves `--dangerously-skip-permissions` out on purpose so permission requests reach the panel.
A person who trusts a folder (a throwaway worktree, a VM) will want agents there to run without stopping at every request; a person who does
not must be sure no agent can be started that way by accident, by the hub, or by another agent.

## Decision (proposed)
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
4. **Node policy decides, per machine.** `skip_permissions = "forbid" | "allow" | "confined_only"` (default **forbid**; the owner sets it in the node's own
   configuration, never over the hub). The node refuses a start that violates it with a stable error code (`agent.option_forbidden`), and
   `confined_only` requires the agent's confinement (ADR-0047/0050) to be active, so a "don't ask" agent still cannot write outside its folder.
5. **Helpers never inherit it.** An agent started by `agent_start` (ADR-0057) gets no `skips_permissions` option unless the signed approval names it
   explicitly; a grant never covers it.
6. **Per-kind defaults** ("always start Codex like this") are a later, separate decision; this ADR only adds the per-start choice.

## Open questions for the maintainer
- Default node policy `forbid` (proposed) or `allow`? A fresh machine that forbids means the choice is greyed out until the owner changes the node's configuration.
- May a structured (ACP) agent's *mode* that skips permissions also be chosen at start (not only switched later)? (Proposed: yes, same confirmation.)
- Should the "does not ask" badge also require the node's confinement to be reported to the window? (Proposed: show both.)

## Consequences
Schema: `AgentStart.launch_options` (new repeated field, append-only), an audit field, `AgentSnapshot.launch_options` for the badge, error code
`agent.option_forbidden`, capability `agent.launch_options`; manifest format + `docs/specs/agent-manifest.md`; node policy file; clients (start window, badge); shipped
manifests with checked flags; threat model row (a person or agent starting an agent that does not ask).
