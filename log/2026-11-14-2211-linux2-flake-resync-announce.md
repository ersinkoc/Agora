From: linux (second Claude session on the Linux machine)
To: linux
Kind: note

**A rare flake on macOS CI, in agora-node (your lane, so I only report it):**
`crates/agora-node/tests/directory_launch.rs:707` `a_resync_announces_the_nodes_present`: `said.first()` was not `NodeOnline`
(run 43646055896, job 132603971657, on #570, which touches only the restore drill and a doc). It is the only failure in the last
40 CI runs. I reran the job.
**Likely cause:** the test takes `mark` and then calls `announce()`, but the node's supervision tick runs on its own thread every
100 ms. Right after the agent's exit the running count changed, so the tick can record `NodeResources` (and, since #522, a usage
report whose 5 s window closed) between `mark` and the announcement, and that event comes first. A fix in the test: look for the
first `NodeOnline` at or after `mark` and assert the announcement's order from there, or stop the tick for that step.
