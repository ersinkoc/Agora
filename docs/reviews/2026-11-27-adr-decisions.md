# Architecture decisions for orchestration and recovery

The first vote is whether to add scoped notices for the dispatcher and project channel. The next
vote is whether a participating agent can pull an eligible task. Both build on shared-board Review.
The detection and plugin proposals can be decided independently.

| Record | Recommended decision | Evidence before rollout |
| --- | --- | --- |
| [ADR-0130](../adr/0130-notify-the-dispatcher-and-project-on-task-acceptance.md) | Opt in separately for the verified dispatcher and designated project channel. Keep the notices small and scoped. | Correlate the routed dispatch with the claim, then test reconnect, reassignment and revoked board access. |
| [ADR-0131](../adr/0131-an-idle-agent-can-ask-for-the-next-task.md) | An existing agent asks for the next task. The owner marks tasks eligible and admits specific nodes and kinds. | Race multiple claims and prove a lost reply cannot take a second task. |
| [ADR-0132](../adr/0132-show-negotiated-node-capabilities-in-the-window.md) | Publish a scoped capability hint for the current link, starting with task dispatch. | Test older peers, link replacement, restart and authorization refusal with the hint present. |
| [ADR-0133](../adr/0133-shared-mcp-servers-can-restart-within-an-agent-run.md) | Retry an approved MCP server within the same verified run. Keep the secret-bearing URL bridge a separate implementation step. | Confirm instance ownership across restart and check outbound policy and secret handling. |
| [ADR-0134](../adr/0134-agent-accounts-are-configured-on-the-node.md) | Manage accounts on the node. Add optional account selection from the window with signed launch authorization. | Verify per-kind configuration isolation and account selection with a fake provider, then have a person verify each supported release. |
| [ADR-0135](../adr/0135-bound-hub-replay-with-projection-checkpoints.md) | Measure before checkpointing. Ship checkpoints before opt-in pruning. | Reproduce the budget crossing with current event mixes, then prove restore and replay equivalence. |

The protocol changes in ADR-0130 through ADR-0132 and the later window step of ADR-0134 each need
explicit schema approval. ADR-0133 and ADR-0135 propose local changes in their initial steps.

## Pending detection decisions

### ADR-0117 screen detection

Use 300 ms for idle confirmation and 800 ms for blocked hold as starting values in manifests that
explicitly opt in, pending timed captures. Keep existing defaults for manifests without a detection
table. Measure the intentional idle confirmation delay separately. Keep the 300 ms budget for
screen-detected working and blocked changes. Let the existing no-opinion state cover early startup
until a fixture shows a need for a grace period. Keep the readiness and typing rules out of anchored
regions in the first step. Defer signed detector-rule updates until a signing pipeline exists, then
start opt-in with an owner-pinned key. [ADR-0117](../adr/0117-screen-detection-confirms-idle-keeps-blocked-and-reads-anchored-regions.md#open-questions)

### ADR-0118 agent state reports

Enable reports only for kinds with verified, configured hooks, with a node policy opt-out. Use a
five-second trust window as the starting value. Report Claude Code's ordinary end of turn as
`idle` to avoid a finished notification for every turn. Keep the existing client source label.
Test same-state events at the node and hub before adding a second notifier filter. Leave Codex hooks
disabled while they require a review dialog the node cannot account for. Keep the session mismatch
refusal through `/clear` until a real capture proves what changes and how resume should follow it.

Clarify token behavior in the decision. Replacement processes receive fresh tokens. A running
holder-backed process keeps its original environment when it reattaches. Restore its token and
ordering state only when the node can prove the same run and endpoint ownership. Otherwise use
screen detection for that process until its next launch. Exercise reattachment and reordered
reports before enabling the hook integration. [ADR-0118](../adr/0118-a-terminal-agent-can-report-its-own-state-through-its-mesh-socket.md#open-questions)

## Pending plugin decision

Accept the proposed scope of ADR-0114 Amendment B. A plugin may open its own panel, focus a visible
agent, prepare a draft the person sends, and declare palette commands with user-assigned keys.
Keep the amendment's bounds, reserved keys and extra confirmation for prepared text sent to an
agent that skips permission prompts. Keep host clipboard actions for a future amendment. Verify the
host-observed user gesture first in Chromium, WebKit and desktop webviews. On a host without
usable activation evidence, permit actions only through the host's explicit command path.
[ADR-0114 Amendment B](../adr/0114-community-plugins-run-sandboxed-and-ask-for-what-they-see.md)
