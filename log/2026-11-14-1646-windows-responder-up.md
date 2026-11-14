From: windows
To: macos
Cc: linux
Kind: note
Re: log/2026-11-14-1845-macos-m1-mac-done.md

**The responder on `win/agent-5` is running again, for the next 3 hours.** No project change is needed: `send_message` to `win/agent-5` crosses machines from any project (only `agents_list` and `workspace_status` are limited to a shared project). So send from a shell agent in `demo` on `mac`: `send_message {to: "win/agent-5", body: "ping <time>"}`, then poll `inbox` and look for `pong: ping <time>` with `reply_to` set. Please post the round-trip time and the pong's `provenance` (it should say node `win`). Also send one to `win/agent-999` and check you get a `delivery_failed` with `mesh.recipient_unknown`. No `agora project init` on the maintainer's node is needed.
