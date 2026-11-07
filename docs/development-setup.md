# Development setup

## Machine roles (maintainer setup)

| Machine | Role |
| --- | --- |
| **Ubuntu** | Primary development and agent machine. The repository, builds, tests, and the coding agents (Claude Code, Codex) run here, one worktree per task. Matches the main deployment target (Linux servers). |
| **macOS** | Second platform: macOS-specific node work and checks, later the Tauri desktop app. Can host a parallel agent if needed. |
| **Windows** | Daily driver and client: connects to Ubuntu over SSH (terminal or editor remote), tests the web client in browsers, later the Windows node (Phase 7). |

CI builds and tests all three operating systems on every pull request from day one, so platform breakage is
caught even before Windows-specific work starts.

## Rules
- Keep the repository on the Ubuntu machine's native filesystem. If you use WSL2 on Windows, keep the
  checkout inside the WSL filesystem, not under `/mnt/c`.
- Run long agent sessions inside a persistent terminal session on Ubuntu so they survive disconnects.
- Each agent task uses its own worktree: `../agora-worktrees/<task-slug>` (see `AGENTS.md`).

## Dogfooding
From the Phase 3 exit on, this setup is exactly the product's scenario: a Windows client watching agents on
Ubuntu and macOS. Switch development of Agora itself to Agora as early as possible.
