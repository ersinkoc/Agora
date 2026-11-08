# 0030. `agent.kinds` command: which agents a node can start
Status: Accepted
Date: 2026-11-08
Accepted: 2026-11-08 (maintainer's explicit approval in chat)

## Context
A client that starts an agent (web "Start an agent", TUI, CLI) must offer the user a choice of agents. Which agents a node can
start is node-local data: its manifests (L1), ACP flags (L2) and adapters (L3), and whether each is actually installed there.
Today the only way to learn it is to try `agent.start` and read `agent.kind_unknown`, so a client cannot show a list, and the
first-agent flow (Phase 3 exit: time to first agent <= 60 s) cannot be a pick-from-a-list screen. `agora doctor` already computes
exactly this list on the node (manifest valid, binary found), but only for the CLI.

## Decision
Append one command and its result to `schema/agora/v1/commands.proto` (new messages and new field numbers; nothing published
changes, ADR-0006):

- `AgentKinds {}` as `CommandRequest.agent_kinds = 16`: no parameters; routed to a node like any command (ADR-0029).
- `AgentKindsResult { repeated AgentKind kinds = 1; }` as `CommandOk.agent_kinds = 13`.
- `AgentKind { string kind = 1; string display_name = 2; IntegrationLevel level = 3; bool installed = 4; }`: `kind` is the
  manifest's kind id (what `AgentStart.kind` takes), `display_name` its human name, `level` the integration level (L1/L2/L3),
  `installed` whether the launch program is found on this node. Kinds whose manifest is invalid are omitted (the doctor reports
  them); kinds that are not installed are listed with `installed = false` so a client can say so instead of hiding them.

The node answers from its loaded manifests and a `PATH` lookup (the same check `agora doctor` uses); it never reveals paths,
environment or credentials. Clients show `display_name` and treat `kind` as an opaque id.

## Consequences
- One new command, three new messages; golden fixtures are added, none edited. `agent.list` (ADR-0025) is unchanged.
- The web and TUI first-agent flow can be built on it. Until then clients may only offer kinds they already see running.
- The node's `PATH` lookup moves from `agora doctor` into a shared function in `agora-node` (no new dependency).

## Alternatives considered
- Put installed kinds in `NodeOnline`: grows an event that is stored forever for data that changes only when a manifest does,
  and a node cannot refresh it without a new event. A command is asked for when needed.
- Have the hub keep a catalog of kinds: the hub must not know what is installed on a machine; manifests are node data
  ("agents are data, not code", architecture.md).
