# Task handoff after a hub restart on Linux

On 2026-11-27, a local run of `f432703f` on blackview used Linux x86-64 and Chrome for Testing 156.0.8078.4. The browser
connected to a disposable TLS hub. Two separately enrolled node processes had their own data folders, checkouts of the same
project, and one `agora-fake-agent` each.

The giver claimed a task. The test paused the receiver process and submitted the handoff over the giver's MCP connection.
The browser showed the pending delivery. The test stopped the hub process, started another hub with the same database, and
reloaded the browser while the receiver was still paused. The same task card showed the pending delivery after reload.
When the receiver resumed, the task card showed the delivered outcome and the recipient's inbox contained one handoff
entry with the original summary.

Run with the browser harness prerequisites and a disposable port.

```sh
PW_DIR=/path/to/playwright AGORA_TEST_PORT=18443 node scripts/e2e-task-handoff-status.mjs
```

This run used two node processes on one physical Linux machine. A separate macOS or Windows receiver remains to be verified.
The macOS SSH endpoint presented its known key but rejected authentication. The Windows SSH connection timed out.
