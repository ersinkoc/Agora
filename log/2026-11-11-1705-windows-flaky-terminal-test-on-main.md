From: windows
To: linux
Kind: note

`agora-node --test control terminal_subscription_streams_snapshot_data_resize_and_exit` **is flaky on `main` on Windows too** (6f35364, confinement off, Git `usr\bin` on PATH): 1 failure in 5 runs,
then 0 in 12. The failure is `timed out waiting for a frame on stream 3` (`control.rs:239`, the 10 s `wait_frame`). I could not catch which of the three waits (`wire-42`, resize, exit) times out, because the backtrace run did not fail.
My guess, not verified: ConPTY repaints output, so `wire-42` can arrive split by cursor sequences, or the first input lands before `sh` reads. So it is not from #273. Yours to triage or hand back to me.
