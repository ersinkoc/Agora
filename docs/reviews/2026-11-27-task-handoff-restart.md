# Task handoff after a hub restart

## Two nodes on one Linux machine

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

## Linux and Windows on separate machines

On 2026-11-27, blackview (Linux x86-64) hosted a disposable hub, a sender node and Chrome for Testing. WHITE (Windows 11)
hosted a separately enrolled receiver node. The machines connected over Tailscale. The Linux binaries were built from
`f432703f` and the Windows binaries from `7b69310f`. Each node ran one `agora-fake-agent` under a separate test home.
Both checkouts used the same portable project ID.

1. The Linux agent created and claimed the task. The Windows agent's MCP task list showed the same task and Linux owner.
2. A trigger in the test hub's SQLite database held updates to the delivery queue while allowing a new delivery to be
   inserted. The task handoff committed one queue record and the browser showed the pending status.
3. The hub process was killed and restarted with the same database. After a full browser reload the task still showed
   the pending status. Removing the trigger allowed the hub to send the delivery.
4. The Windows receiver returned a matching durable receipt. The Linux browser then showed the delivered status and the
   hub queue had no remaining record for the task.

The same two nodes then tested a restart after receipt and before settlement. A separate trigger refused deletion of the
new delivery's queue record. The hub logged the settlement error after receiving a matching receipt from Windows. The
hub was killed with that record still pending. On restart it retried the recorded delivery and again received a matching
receipt on the Windows link. After the trigger was removed, the browser showed the delivered status and the remaining
queue count was zero. The receiver used its live run across both restart scenarios.

The Windows operator checked the receiver's own MCP inbox after both scenarios. It held two handoff messages, one with
each original summary. The receiver's test data directory held two durable handoff files, one per delivery.
