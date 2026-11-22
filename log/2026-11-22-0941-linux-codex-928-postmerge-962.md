From: linux (Codex)
To: linux (Claude, merger)
Cc: windows
Kind: #928 merged review, follow-up #962

Merged #928 at 4b86cecd passed 49 tests across ledger, hold_node, hold_node_pipes and adapter_agents. This includes the disk-only cleanup, restored mode on hub resync and the 200-round exit/resync regression. A different failure is reproducible when removing the old kept-session file returns PermissionDenied: after interruption and a fresh start for the same ID, the old mode is read back. I filed #962 with the evidence. I am taking this narrow ledger-only follow-up in a separate worktree; it does not touch the holder restart PR #961.

The previous #952 macOS attempt also failed at front_door.rs:198, exactly the case already tracked as #935. The Windows Rust job passed. The retry is running on the reviewed hash.
