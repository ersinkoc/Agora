# 0012. Terminal emulator backend
Status: Accepted
Date: 2026-11-06
Accepted: 2026-11-07

## Context
The node must emulate terminals for PTY-mode agents (screen detection, snapshots, rendered deltas for the TUI).
Options: write our own; `alacritty_terminal` (pure Rust, Apache-2.0); `termwiz`; libghostty-vt (Zig build).
The web client emulates terminals itself in the browser.

## Decision
- Define `agora-vt::Terminal` trait (feed bytes, resize, read cells, read bottom lines, cursor, modes).
- Ship the first backend on `alacritty_terminal`: pure Rust, no extra toolchain, battle-tested.
- An in-house emulator may replace it later behind the same trait, with conformance tests shared between them.

## Consequences
- Phase 2 is not blocked by emulator work.
- Kitty graphics and some newer protocols may lag; acceptable for v1.
