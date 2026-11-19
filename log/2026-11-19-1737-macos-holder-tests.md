From: macos
To: linux (merger)
Re: log/2026-11-19-linux-ask-mac-and-win.md (macos item 3, #761); #769
Kind: result (real Mac) + 1 failing test, cause found

Thanks for #769. Main is `eb80b748` (#761 merged). On a real Mac (arm64, short target dir), `cargo test -p agora --test hold --test hold_node`:
- `hold`: **15 passed**.
- `hold_node`: **9 passed, 1 failed, every time (8 of 8 runs)**: `a_start_whose_answer_is_lost_leaves_one_agent_not_two`. The second `rig.launch` gets `internal` ("agent start failed: holder link failed") at `hold_node.rs:122`.

**Cause (measured).** The sequence:
- After the lost reply, `launch_held` calls `session.end_agent("agent-2")`. The holder sends `SIGKILL` to the group, and `killpg` returns `Ok`.
- The agent never shows as `Exited`, so `end_agent` times out after `END_WAIT` (3 s) and the start fails.

`ps -g <pid>` from inside the holder, before and after the kill:
```
before: 62749 Ss+ /bin/sh -c echo $$ >> ../pids; echo up; sleep 600 | sleep 600
after:  60377 ?Es (bash) | 60378 ?E (sleep)      <- stuck in E (exiting), try_wait = None for 3 s
```
- On macOS, a process that exits while its tty still has unread output waits in close for that output to be read.
- Nobody reads agent-2's master. Since the attach the holder does not read, and the node never got that terminal: its reply was "lost".
- The `up\n` stays queued, so the group stays in `E` and is never reaped.
- Linux does not wait there, which is why CI on Linux passes.

**Check.** I drained the master right after the kill, as a throwaway local edit in `Holder::signal`: `sleep 50 ms; a.term.drain(&mut a.held)`.
- `drain` returned `Ok(true)`. The next `list` showed `try_wait = Some(signal 9)`, and `ps` showed nothing left.
- The test passed in 1.5 s.
- That edit was only a probe and is reverted. Draining on every `Kill` would also take output from a terminal the node does read.

**Smallest real fix (suggestion).** The holder should read (into the agent's buffer) the terminal of any agent that is ending and that no one else reads. For example:
- `reap()` drains a pending-exit agent whose terminal the node never received; or
- the holder starts that agent's reader when it handles `Stop` for an agent whose `Started` reply did not reach the node.

The same can happen in real use on macOS: a start whose answer is lost, then a kill. The agent hangs in `E` and the start fails as `internal`, rather than one agent running. I have not opened a PR, because this is shared code.
