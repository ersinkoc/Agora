# Backlog

Tasks sized for one coding-agent session each. From 2026-11-26 the repository keeps no open GitHub issues or pull requests:
work lands on `main`, the plan is the roadmap and the small leftovers are the follow-ups list. Tasks marked `needs-decision` wait
for a Proposed ADR to be accepted by the maintainer.

- **[Roadmap: the order of the remaining work, with subtasks](roadmap.md)** (start here)
- [Follow-ups: small leftovers, suspected bugs and flaky tests](follow-ups.md)
- [Phase 0: bootstrap](phase-0.md)
- [Phase 1: protocol and schema](phase-1.md)
- [Phase 2: node core](phase-2.md)
- [Phase 3: hub MVP and web client](phase-3.md)
- [Phase 4: ACP and approvals](phase-4.md)
- [Phase 6: extensibility, terminal client](phase-6.md)
- [Phase 7: scale out (Windows node)](phase-7.md)
- [Three-machine LAN test findings (2026-11-11)](lan-test-findings.md)
- [Research (ongoing): usage sources, quota profiles, model mapping](research.md)

Completed: #235 / merged PR #236 implements accepted ADR-0052: selected working folders, optional worktrees,
and explicit portable project IDs across machines. Verification and limits: [selected-directory review](../reviews/0235-selected-directories.md).

Follow-up #257 closes reproduced launch, diff-root, resume and filtered-pane findings from #236.

Evidence-audit fixes, in 0.3.14: F401 (#633), F402 (#634), F601 to F603 (#625, #626, #627), F702 to F704 (#623, #624, #630), F202 (#628).
After 0.3.14, for the next release: F210 and F211 (#641), F221 and F222 (#646), F232 (#645), F240 (#642), F250 and F251 (#643),
F260 to F262 (#640), F270 (#649), F271 (#644), F280 to F282 (#647), F290 (#656), F501 (#629).
ADR-0081 to 0084 are Accepted (2026-11-17); ADR-0081, 0082 and 0084 are built (#673, #674, #699) and ADR-0083 is partly built
(`docs/capabilities.md`). Easy setup (ADR-0065) and the terminal holder on Linux and macOS (ADR-0079) are built. The order of the work
from here is [the roadmap](roadmap.md).

ADR-0087 (install and update coding agents on a machine from the window) is Accepted: `agora doctor`'s agent checks, the signed list,
`agora agent install` and the node starting installed agents are built (#868, #871, #876); installing from the window is not (roadmap M2.1).

Deferred dependency updates (Dependabot is told to ignore these; revisit when the reason no longer holds):
- tokio-tungstenite 0.24 to 0.30: the WebSocket stack is the hub/node link and needs a deliberate code migration with its own review (the Dependabot attempt failed CI with four failures).
- getrandom 0.2 to 0.4: `getrandom::getrandom` was removed (renamed `fill`) and is called in 12 places that make tokens, nonces, ids and the pairing numbers (`agora`, `agora-hub`, `agora-node`): a deliberate migration of the randomness source with its own review, one small helper in one place first (Dependabot attempt: the closed PR 695).
- GitHub Actions major versions (`actions/checkout`, `setup-node`, `upload-artifact`, `download-artifact`): `release.yml` cannot be tested without pushing a tag, and the artifact actions change behaviour (names, merging, immutability); do them in one change after a release rehearsal exists.
- `@types/node` major versions: the types must track the Node version the project runs (22); update them when the Node baseline moves.
- Desktop app glib advisory: Dependabot alert #1 (glib < 0.20.0, medium, transitive of the GTK3 bindings under Tauri); fixed by moving Tauri to a GTK4/glib 0.20 line, so it is not bumpable alone.
