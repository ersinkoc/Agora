# AGENTS.md

Instructions for every coding agent working on this repository (Codex, Claude Code, and any other).
Claude Code loads this file through `CLAUDE.md`. Keep this file the single source of truth for agent rules.

> Working name: **Agora**. The final product name is undecided (see `docs/open-questions.md`).
> Renaming must be a single mechanical change, so never hardcode the name outside `crates/agora-*` names,
> the binary name, and user-facing strings in `locales/`.

## What this project is

A central control plane for AI coding agents. A **node** runs on every machine (laptop, VPS, SSH-only
server, Windows PC) and supervises agents and terminals there. All nodes connect to a **hub**. Users
open **one window** (web/desktop, or TUI) connected to the hub and see every project, every agent,
files, and changes across all machines. Agents talk to each other through a structured **mesh**
instead of reading each other's screens.

Read before any non-trivial change (start with `docs/map.md`, one page):

1. `docs/vision.md` — goals and non-goals
2. `docs/architecture.md` — components, ownership, and data flow
3. `docs/glossary.md` — use these terms exactly
4. `docs/adr/` — `Accepted` decisions are binding; `Proposed` ones are not final, do not build on them
5. `docs/specs/` — protocol, error codes, manifests, adapter protocol, mesh tools
6. `docs/security/threat-model.md` — before touching auth, approvals, mesh, adapters, or networking
7. `docs/design/performance.md` — before touching any hot path or client rendering
8. The task's issue and `docs/backlog/`

## Invariants (never break these)

- **Nodes are autonomous.** A node keeps agents running and buffers events when the hub is unreachable.
  Never make agent execution depend on a live hub connection.
- **State is separate from runtime.** Domain state (`agora-core`) is plain data, testable without PTYs,
  sockets, or async. Runtime handles (PTY, processes, connections) live in their own types.
- **Render is pure.** Clients derive views from state. Rendering never mutates state.
- **The protocol is schema-first and append-only.** Every message is defined in `schema/`. Never change
  the meaning or shape of a published message; add a new message or capability instead. Changing the
  schema requires an ADR.
- **No user-facing text in Rust code or API responses.** APIs return stable error codes and parameters.
  All human text lives in `locales/<lang>/*.ftl` (Project Fluent). English is the source locale.
- **Platform code is isolated.** OS-specific code lives only in `crates/agora-platform/src/<os>.rs`.
  No `#[cfg(target_os)]` outside that crate.
- **Agents are data, not code.** Adding support for a new agent must not require changing core crates.
  It is a manifest (L1), an ACP flag (L2), or an adapter process (L3). See `docs/architecture.md#integration-levels`.
- **Secrets never leave the node.** The hub stores references, never API keys or tokens.
- **The hub cannot widen node policy.** Sensitive actions need user-signed approvals verified on the node (ADR-0020).
- **Mesh messages are untrusted data.** Never let content from another agent bypass approvals.
- **Performance budgets are requirements** (ADR-0022, `docs/design/performance.md`). Hot-path changes need a
  benchmark result in the PR; terminal bytes never go through the event store; input outranks output.
- **Simplicity and visual quality are requirements** (ADR-0024, `docs/ux/visual-language.md`). UI text uses
  the UI vocabulary, never internal terms. UI PRs include screenshots in both themes and use design tokens only.
- **Detection never reads the user-visible viewport.** Screen-based detection reads the live bottom of
  the terminal buffer only.

## Multi-agent development workflow

This repository is developed by several coding agents in parallel (Claude Code and Codex). To avoid
collisions:

- **One task, one worktree, one branch.** Work in `../agora-worktrees/<task-slug>` on branch
  `<agent>/<issue-id>-<slug>` (for example `codex/42-pty-resize`). Never edit the shared main checkout
  while another task is in progress there.
- **Claim before you start.** A task is claimed by its issue assignee/label. Do not start an unclaimed
  task that touches the same crate as a claimed one without coordinating through the issue.
- **Stay in your lane.** Touch only the crates your task names. If you need a change in another crate,
  open a separate issue rather than widening the diff.
- **Cross-review.** A PR written by one agent is reviewed by the other (Codex reviews Claude Code PRs and
  vice versa) before the human maintainer merges. Agents never merge.
- **Architecture changes need an ADR first.** If your change alters an invariant, a protocol message,
  crate boundaries, or a dependency policy, write `docs/adr/NNNN-title.md` with status `Proposed` and stop
  for human review.
- **Docs move with code.** If behavior described in `docs/` changes, update the doc in the same PR.
- **Leave a handoff note.** When stopping mid-task, append to the PR description: what is done, what is
  not, how to verify, known risks.

## Repository layout (planned)

```
crates/
  agora-core/        pure domain model: projects, agents, terminals, worktrees, layout
  agora-protocol/    generated types from schema/, codecs, capability negotiation
  agora-platform/    OS isolation: PTY/ConPTY, process tree, cwd, sockets/pipes
  agora-vt/          terminal emulator trait + backend
  agora-node/        node daemon: supervises terminals, agents, files, git
  agora-adapters/    L1 manifest engine, L2 ACP client, L3 adapter host
  agora-mesh/        messaging, task board, file leases, handoff; MCP server
  agora-hub/         hub: identity, routing, event store, approvals, mesh broker, web asset serving
  agora-i18n/        Fluent loading, locale negotiation
  agora-tui/         terminal client
  agora/             the single binary: `agora node | hub | tui | ...`
apps/
  web/               primary client (TypeScript), embedded into the hub binary
  desktop/           Tauri shell around apps/web
schema/              protocol schema (source of truth for all languages)
manifests/agents/    L1 agent manifests (TOML)
locales/<lang>/      Fluent translation files
docs/                vision, architecture, ADRs, specs, security, design notes, UX, backlog
scripts/             repository checks (`check-docs.sh`)
```

## Rust conventions

- Stable toolchain pinned in `rust-toolchain.toml`. Edition 2021 or newer.
- Minimize dependencies. Every new crate dependency needs a one-line justification in the PR. Prefer std.
- No `unwrap()`/`expect()` outside tests and provably-infallible cases (comment why).
- Errors: typed error enums per crate; map to protocol error codes at the boundary.
- `cargo fmt`, `cargo clippy -- -D warnings`, and tests must pass before a PR.
- Hot paths (PTY read, VT parse, frame fanout, detection) scale with panes × clients. Do not add
  allocation, filesystem I/O, or lock contention there without a benchmark.

## Testing

- Domain logic in `agora-core` gets unit tests with no I/O.
- Protocol changes get golden fixtures; a fixture for a published message is never edited, only added.
- Agent manifests get fixture screens (captured terminal bottoms) with expected states.
- Integration tests use `agora-fake-agent`, never real agents, accounts, or tokens.
- Cross-platform behavior is tested on Linux, macOS, and Windows in CI.

## Commit messages

Conventional Commits: `feat(node): ...`, `fix(protocol): ...`, `docs(adr): ...`.
