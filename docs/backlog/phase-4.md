# Phase 4: ACP (L2) and approvals

Exit criteria (roadmap): Claude Code, Codex and WrongStack in structured mode (ACP or adapter) with approvals answered from the
window. Items that need real agent accounts are marked and skipped, never faked.

| Task | State | Notes |
| --- | --- | --- |
| P4-01 ACP client | done (against a fake ACP agent) | `agora-adapters::acp`: `agora acp-bridge -- <agent argv>` is the ACP client towards the agent and an adapter towards the node; manifests say `[structured] acp = { argv = [...] }`; kinds report level L2; tested end to end with `agora-fake-agent --acp` (bridge unit and integration tests, and a real node through the CLI). Mapping in `docs/specs/adapter-protocol.md` |
| P4-02 structured chat view | done | the node keeps each chat agent's transcript and streams it (`chat.subscribe`, ADR-0038); the console shows the conversation with a prompt box; plans, tool calls and diffs have their own rendering |
| P4-03 approvals queue across clients | done | the console lists waiting approvals from every machine and answers them with this browser's device key; hidden-tab notifications; stale approvals are cleared when a node restarts |
| P4-04 node-local policy and user-signed approvals | done (ADR-0020, ADR-0028) | trusted-devices file reloaded when it changes |
| P4-05 real agents in structured mode | needs a human | manifests and captured fixtures for Claude Code, Codex and WrongStack need real accounts (P2-09, P2-12); the Claude Code and Codex ACP entry points differ by version and must be tried on a machine that has them |
