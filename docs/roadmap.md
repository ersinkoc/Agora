# Roadmap

Each phase ends with a releasable, usable state. Do not start a phase before the previous phase's exit
criteria are met.

## Phase 0 — Bootstrap
- Maintainer decisions on ADR-0009/0010/0011/0016; repository settings; CI on three OSes; i18n lint; fake agent skeleton.
- Benchmark harness skeleton (ADR-0022).
- **Exit:** see `backlog/phase-0.md`.

## Phase 1 — Protocol and schema
- Choose schema format and codegen (open question Q2), write ADR.
- Define node, client, and adapter handshake; capability model; error code dictionary.
- Golden fixtures for every message.
- Error model, event sequencing and replay, adapter protocol messages, domain model v0.
- **Exit:** generated Rust and TypeScript types compile; fixtures round-trip. Tasks: `backlog/phase-1.md`.

## Phase 2 — Node core (Linux, macOS)
- `agora-platform`, `agora-vt`, terminals, process inspection, files, git, worktrees.
- L0 and L1 adapters; first manifests: claude-code, codex, wrongstack.
- Agent accounts on nodes (login flow, status checks); resource limits; local event buffer.
- **Exit:** a node can run agents in worktrees and report state, controllable via CLI locally; node capacity
  and footprint budgets measured.

## Phase 3 — Hub MVP and web client
- Hub: single user, SQLite, device enrollment, event store, routing.
- Web client: main window (projects, agents, terminal, files, changes).
- SSH bootstrap for one remote node; onboarding and enrollment flow; review screen.
- Notifications (in-app, desktop, web push); `agora doctor`; hub backup/restore; usage and cost view.
- Design tokens, component gallery, visual regression tests (before screens).
- Home (Glance) with live tiles and attention strip; Focus view.
- Local-first client store, GPU terminal rendering, command palette, attention flow; direct data path if ADR-0023 is accepted.
- **Exit:** two machines' agents visible and controllable in one browser window; interaction and rendering
  budgets met; time to first agent ≤ 60 s; first-glance test passed with five users.

## Phase 4 — ACP (L2) and approvals
- ACP client in `agora-adapters`; structured chat view; approvals queue across clients.
- Node-local policy and user-signed approvals (ADR-0020).
- **Exit:** Claude Code, Codex, and WrongStack in structured mode (ACP or adapter) with approvals answered from the window.

## Phase 5 — Mesh
- MCP server injection; messages, channels, task board, leases, handoff.
- **Exit:** two agents on different machines complete a task with handoff and no file collisions.

## Phase 6 — Extensibility and global readiness
- L3 adapter protocol and host; adapter registry with signed packages.
- First non-English locales, RTL in web; TUI client.
- Node self-update with live handoff (ADR-0019).
- **Exit:** a third-party adapter in another language connects without a core change.

## Phase 7 — Scale out
- Windows node; desktop app (Tauri); team mode with Postgres, policies, audit UI; mobile approvals.
- Optional agent sandboxing (threat model T8); privacy features for cloud (region, E2E terminal encryption).
- Cloud (multi-tenant hub) planning.
