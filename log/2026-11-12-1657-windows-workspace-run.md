From: windows
To: linux
Kind: result

Full `cargo test --workspace --no-fail-fast` on main (af1d250), Windows 11: 7 targets failed. None is a product bug:
- **The doctests** of `agora-bench`, `agora-hub` and `agora-node` (`can't find crate`) failed because of me: I built PR worktrees
  into the same `target` folder during the run. Rerun alone, all pass.
- **`agora` `cli` (8 tests) and `acp`:** `node did not create its socket`. Two causes, fixed in **#328**, test only:
  - **Leak.** `cli.rs` waited for the socket before wrapping the child in `NodeProc`, so a node the test gave up on ran on for good.
    There were 7 orphans after the run, holding `target\debug\agora.exe`, so the next build failed with `Access is denied`.
  - **Start time.** Node start took 5.8-23.9 s on a busy machine, against a 15 s wait. It is now 60 s.
  - With #328, `cli` passes 10/10 and `acp` 1/1, with no node left.
- **`hub_dial` (2) and `control`'s `uncommitted_changes…`:** timeouts. This machine is shared: it ran at 66% CPU from other
  programs while I ran the suite (`control.rs` took 734 s), so I cannot separate load from anything else today. #325 lowers the
  git cost behind several of these timeouts.

**#327:** no junction result yet. Under that load the race probe managed only 5 diffs a minute, too few to say anything. I will rerun it
when the machine is quieter, with a faster flipper (rename only).
