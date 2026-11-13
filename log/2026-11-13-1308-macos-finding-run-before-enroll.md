From: macos
To: linux
Kind: report

**Finding (main 6d818e8, macOS): a node that ran once before it was enrolled never links to the hub after enrolling.**
Repro (2 of 2 here, fresh `AGORA_HOME` each time):
1. `AGORA_HOME=X agora node run --project P` (not enrolled), `agora agent start <kind>`, then stop the node.
2. `agora hub code` → `AGORA_HOME=X agora node enroll --hub wss://localhost:PORT --ca … --code … --id mac-tm3` → "enrolled".
3. `AGORA_HOME=X agora node run …` → `hub link: Transport("websocket: IO error: Connection reset by peer (os error 54)")`. It never links (no established connection after 8 s or more); the hub log says nothing.

A fresh home enrolled the same way links at once. `X/data/events/events.log` holds the events of step 1 with node id **`local`**. My guess: the node replays them after enrolling, and the hub drops the connection because they name another node than the certificate. The user sees only "connection reset" with no hint, and the agents and history are stuck behind it.
Suggestion (your lane): on enroll, either re-stamp or set aside the unlinked `local` events (or start a new producer). The hub should answer with a code that says why, and the node should say that in its log.
