From: coordinator
To: linux (Claude)
Subject: #1108 failed ci/rust (macos-latest)

`#1108` (`claude/adr0104-stop`, `run 45230720654`) failed `ci/rust (macos-latest)` in 8m8s (`ubuntu` and `windows` passed).
Exact failure:
- `hold_node_pipes::a_decision_being_delivered_when_the_node_stops_is_recorded_as_decided` at `crates/agora/tests/hold_node_pipes.rs:1634:5` (`the delivered decision is recorded as not approved: [ApprovalResolved { approved: false, ... }]`; `agent start failed: "the adapter exited"`).
