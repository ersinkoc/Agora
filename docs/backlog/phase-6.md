# Phase 6 backlog (extensibility and global readiness)

Started with the terminal client (ADR-0043). Other Phase 6 items (adapter registry with signed packages, node self-update, RTL) have no tasks yet.

| Task | State | Notes |
| --- | --- | --- |
| P6-01 TUI slice 1: read-only console | done | `agora tui [--hub HOST:PORT --ca FILE --code CODE] [--hub-dir DIR]`: signs in with a one-time code (made by itself on the hub's own machine), follows the hub's snapshot and draws machines, agents grouped by project with their state and the attention list; `NO_COLOR` spells states out without escape codes; text in `locales/{en,tr}/tui.ftl`; pure render with unit tests, a real hub and node in `crates/agora/tests/hub_dial.rs`. It refreshes by asking the hub for a new snapshot (at most every 500 ms after something changed); applying single events is later work |
| P6-02 TUI slice 2: approvals | done (approvals); signed grants open | `y`/`n` answers the first waiting request with this terminal's ES256 key (`<AGORA_HOME>/tui-device.p8`, owner-only file, id `tui-<hex>`; `agora-tui::device`), sent as `approval_resolve` through the hub; a machine that does not trust the key answers `approval.signature_invalid` and the console shows the `agora node trust ID KEY` line to run there; raw-mode key reading (`agora-platform::RawMode`), no keys when stdin is not a terminal; tested end to end with a real hub, node and adapter agent in `hub_dial.rs`, and the signed bytes against `agora-node`'s own function. Signed grants (ADR-0041) for commands the machine's policy lists are not in the console yet |
| P6-03 TUI slice 3: terminal | open | attach through the relay (ADR-0029) |
| P6-04 TUI slice 4: chat, start/stop, usage line | open | ADR-0038 stream, `agent.start`/`agent.stop`, `usage.report` |
