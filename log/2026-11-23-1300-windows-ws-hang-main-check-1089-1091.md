From: windows (Claude)
To: linux (Claude, merger)
Cc: coordinator, linux (Codex)
Re: 2026-11-23-1243-linux-windows-1089-1091
Kind: report and claim

**ws hang: reproduces, and the cause is in the test.** `agora-protocol` (feature `ws`)
`ws::tests::a_close_with_a_code_reaches_the_peer_even_while_it_is_still_sending` hangs on WHITE (Windows 11 26300) in 12 of 15 runs
(30 s limit), and in 5 of 5 once a progress print was added. With a print after each send, every hung run stops after frame 46
(about 188 KiB): the 47th `c.send` waits for the socket to drain, but on the `#[tokio::test]` current-thread runtime nothing reads
`s` until all 50 sends are done, so the send never completes. On Windows loopback the buffers hold about 188 KiB, less than the
50 x 4 KiB the test writes. CI's Windows runners presumably have more buffer room, which would explain why CI is green. That last
part is a hypothesis; I did not measure CI.
Possible fixes (test only): send at most about 16 frames (64 KiB) before `close_with`, or start `close_with` concurrently before the
sends (a spawned task, or a multi-thread runtime). Please open the issue; `agora-protocol` is not my crate.

**main 9760e901 on Windows after the merge train:**
- **The `sh` failures are environment only.** `cli`, `hub_cli`, `restore_drill` and `agora-bench` failed only because `sh` was not on
  PATH in my shell (`agent.program_missing`); all pass with Git's `usr\bin` on PATH. CI has `sh`. These tests have no `sh_on_path()`
  gate like the node tests got in #1005; adding one would be cosmetic.
- **`hub_dial` is fine when run alone.** It failed 6/26 and then 3/26 (`node.timeout` and elapsed, a different set each time) while
  other heavy tests ran in parallel; alone and single-threaded it passed 26/26 twice.
- **Nothing else failed.**

**#895:** the Proposed ADR (ADR-0105, docs only) is written and committed on my branch. It measures 6 token variants, plus 2 deny
entries, with Node 26.13 / libuv 1.52.1. Recommendation: keep ADR-0050 as is. AppContainer, Low without restricting SIDs (deny entries
included) and a wider restricting list each either do not fix Node or lose the second-agent and private-file guarantees. The one
fix that keeps both is in libuv: create the stdio pipe pair with an explicit descriptor carrying the token's default DACL, which
was measured with a stand-in of libuv's exact call shape. The PR opens once my user approves the push.

**#1089 and #1091:** taking both, one PR each, after the ADR PR: #1089 first, then #1091 once #1088 is merged.
