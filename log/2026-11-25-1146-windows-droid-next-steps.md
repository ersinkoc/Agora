From: windows (Droid, agent-60)
To: all
Kind: note

Project review completed on `main` (`0b7295d1`, v0.3.38). Here is a realistic, prioritized next-steps list aligned with `docs/backlog/roadmap.md`, `docs/capabilities.md`, and the currently open PRs (`#1188`, `#1203`, `#1204`, `#1207`, `#1208`):

### P0 — Close in-flight PRs & CI stability
- Finish review/CI/merge for `#1188`, `#1203`, `#1204`, `#1207`, `#1208`.
- Follow up on the `#1207` product note (`crates/agora-node/src/daemon.rs:2063` `announce_present` vs `approvals.request_start` ordering window).

### P1 — Windows real-machine verification & parity (starting on `win`)
- Re-verify Windows confinement (`cargo test -p agora-platform --test winsandbox -- --nocapture`) and Windows workspace/i18n health on `main`.
- Complete missing rows in `docs/reviews/real-machine-checklist.md` (M1.1: ADR-0100 repaint/history, ADR-0101 approval-always, ADR-0103 team chat reload, ADR-0107 interrupted agents, ADR-0108 agent forget).
- Windows browser scale measurements (125% / 150%, M1.3) and Windows holder/restart parity measurements (ADR-0079/0093, M2.6).

### P2 — Structural debt (split oversized production files, merger-coordinated)
- Split the largest production files by responsibility to reduce review cost and merge conflicts:
  - `crates/agora-node/src/supervisor.rs` (5,740 prod lines)
  - `crates/agora/src/tuicmd.rs` (5,308 prod lines)
  - `crates/agora-node/src/daemon.rs` (3,511 prod lines)
  - `crates/agora-adapters/src/acp.rs` (2,982 prod lines)
  - `apps/web/src/App.tsx` (54 KB) and `apps/web/src/screens.tsx` (50 KB)

### P3 — Finish half-built M2 capabilities
- **M2.13**: Carry negotiated node capabilities in the hub snapshot so the web window knows machine capabilities without trial-and-refusal (schema gate).
- **M2.1**: Install/update coding agents from the window (ADR-0087).
- **M2.4 / M2.5**: Remaining Team mode steps (ADR-0049) and Desktop Tauri real-device checks (ADR-0048).
- **M2.12**: Windows/macOS code signing (ADR-0077, maintainer gate).

### Starting now (`win` / `agent-60`)
1. Run Windows `winsandbox` confinement check and `i18n-lint` on `main`.
2. Add missing M1.1 checklist rows (`ADR-0100`, `ADR-0101`, `ADR-0103`, `ADR-0107`, `ADR-0108`) in a separate worktree without touching any files from `#1188`/`#1203`/`#1204`/`#1207`/`#1208`.
