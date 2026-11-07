# Vision

## Problem

Developers now run several AI coding agents at once: Claude Code, Codex, WrongStack, Gemini CLI, Cursor, OpenCode,
and more every month. They run them on different machines: a laptop, a VPS, a build server, a Windows
box. Today this means many terminals, no shared view of which agent is working, blocked, or done, no
clean way to review what each agent changed, and no way for agents to coordinate except by one agent
typing into another's terminal.

Terminal multiplexers built for agents solve the "many terminals" part on one
machine and infer agent state by pattern-matching screen output. Editor integrations (ACP clients) solve
the "structured agent" part inside one editor. Nothing combines both, across all machines, in one place.

## Goal

One window to see and control every AI coding agent on every machine you own, organized by project,
with agents that can coordinate with each other.

## Goals

1. **Every machine.** Linux, macOS, Windows; local and remote, including machines reachable only over SSH.
2. **Central view.** All machines report to one hub. The user connects to the hub only.
3. **Every agent, now and later.** New agents plug in without changing core code.
4. **Project-centric.** A project can host several agents, of different or the same kind, possibly on
   different machines. Each agent's changes are visible and reviewable separately.
5. **Agents coordinate.** Structured messaging, a shared task board, file leases, and handoff between agents.
6. **Global from day one.** Every user-facing surface is localizable; protocol and SDKs are usable from
   any programming language.
7. **Fast and delightful.** Nothing stutters, at any scale: many machines, many projects, many agents,
   many clients. Performance budgets are requirements (ADR-0022).
8. **Striking and simple.** Visually impressive, never complicated: progressive disclosure, plain words,
   zero configuration to start (ADR-0024).
9. **Safe by default.** Central control of code-executing agents is a high-value target; security is a
   core feature, not an add-on.

## Non-goals (for now)

- Building our own coding agent. We host and connect agents; we do not compete with them.
- Replacing the editor. The file explorer and diff view exist for supervision and review, not as a full IDE.
- Hosting model inference or proxying LLM API traffic.
- Sharing one checkout between agents by default (isolation via worktrees is the default).

## Differentiation

| Capability | Agent multiplexers | ACP editor clients | Agora |
| --- | --- | --- | --- |
| Native agent TUI in a real terminal | Yes | No | Yes (L0/L1) |
| Structured state, permissions, diffs | Inferred from screen | Yes | Yes (L2/L3) |
| Many machines in one view | Partial | No | Yes, hub-and-spoke |
| Agent-to-agent messaging | Keystroke injection | No | Structured mesh |
| Per-agent change review | No | Per session | Per agent worktree |
| New agent without a release | Remote manifests | Registry | Manifests, ACP, adapters |

## Users

- **Solo developer** with a laptop and a few servers: runs the hub locally or on a VPS.
- **Team**: shared hub, multiple users, policies and audit (later phase).
- **Cloud** (later): hosted hub, with nodes on the customer's own machines.

The model treats a solo user as a team of one so that the team and cloud modes do not require a rewrite.

## Product principles

- Agents keep running no matter what happens to the window or the hub.
- Show state, do not guess it, whenever the agent can report it. Guess only as a fallback.
- One concept per word (see `glossary.md`).
- Restraint in the UI: the main window is projects, agents, files, changes, mesh, approvals.
