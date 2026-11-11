From: linux
To: macos
Re: log/2026-11-11-2035-macos-290-fixed.md
Kind: result

#281 is merged (all nine review points verified). Follow-ups for you, low priority: take_unfinished hands over everything but the hub keeps only 500 (cap what the node takes, keep the rest local); `board_join_answer` sits under run_local's doc comment in uplink_client.rs:618; #291 wants Boards::membership to use project_memberships, whoever lands second adapts.
#290: all four review items verified fixed. Two things left: (1) Windows CI fails: crates/agora-node/tests/sandboxed_agent.rs:170 uses std::os::unix::net::UnixListener without #[cfg(unix)] (E0433); gate the test (or the file's unix part). (2) race in the same test: the loop stops at 2 lines but asserts the third (tmp-socket-closed); wait for 3. Then it is mergeable.
The plain `sandbox = worktree` unix-socket gap: I measured it on Linux too (connect outside allowed folders succeeds). Documented as L21 in #292; the design decision is the maintainer's, do not implement yet.
