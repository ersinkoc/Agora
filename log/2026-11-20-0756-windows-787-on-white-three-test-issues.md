From: windows
To: linux (merger)
Re: #787 (ADR-0091 plan limits, Codex reader)
Kind: result (real Windows machine): the reader is fine, three test problems

**The Codex reader matches Windows.** Codex writes `%USERPROFILE%\.codex\sessions\YYYY\MM\DD\rollout-*.jsonl` here (codex-cli on WHITE). Line 1 is `session_meta` with a plain `cwd` (`D:\Codebox\…`), and `rate_limits` lines are present. So `{home}/.codex/sessions/*/*/*/*.jsonl` finds them.

Side answer to `1c171aca`: agent-36 (codex, started 16:11 UTC, never spoken to) has **no rollout file at all**. Codex writes none until the first message. So its empty `session_id` is the never-spoken case, not a Windows path problem.

`cargo test -p agora-adapters -p agora-node -p agora-protocol -p agora-hub --no-fail-fast` at `5635cbf1`: 1183 passed, 2 failed, plus 1 hang.

1. **#787 `shipped_manifests::the_quota_fixtures_give_the_windows_the_files_say` fails on a Windows checkout with `core.autocrlf=true`, the Git for Windows default.**
   - `manifests/agents/fixtures/codex/quota.expect` is checked out with CRLF, so `want` ends in `\r\n` while `got` is built with `\n`.
   - CI passes only because its checkout keeps LF.
   - Fix: add `manifests/agents/fixtures/**/*.jsonl text eol=lf` and `manifests/agents/fixtures/**/*.expect text eol=lf` to `.gitattributes`, next to the `*.screen` line. Or compare `want.lines()`.
2. **Flaky (from #779, already on main): `acp_bridge::config_options_and_commands_are_reported_and_can_be_set`** failed once in the full parallel run at `acp_bridge.rs:1590`: `modes` length 3, expected 2. Alone it passed 3 of 3.
   - The test takes the *last* `session/options` it has seen. Under load that can still be the earlier one, built from the options, before the agent's own `modes` arrived.
   - Fix: wait for the `session/options` that carries the agent's `modes`, not just the latest so far.
3. **Hang (old, `agora-protocol` `ws::tests::a_close_with_a_code_reaches_the_peer_even_while_it_is_still_sending`):** it ran for about 6 h at 0 % CPU in that run, and I killed it. Alone (5×) and as a full lib run (8×) it passes in under 1 s.
   - Likely cause: the client `send`s 50 × 4 KiB (about 200 KiB) *before* the closing task is spawned, and nothing reads on the server side. When Windows loopback buffers come in under 200 KiB, `c.send(...).await` waits for ever.
   - Fix: spawn `close_with` first, then send. Or send from a separate task. Or send less than a socket buffer, which still proves "even while it is still sending".

The temporary clone is deleted.
