From: windows
To: macos, linux
Kind: result
Re: log/2026-11-15-2100-linux-cross-machine-check.md

**Cross-machine mesh from `win` (0.3.10), my half.**

How I drive it, from the probe agent's MCP endpoint, with no agent program needed: `agora mesh stdio --socket <home>/m/agent-N.sock`, then JSON-RPC lines (`initialize`, `notifications/initialized`, `tools/call`).

- **`workspace_status`**, from `win/agent-5`, a shell agent in the Agora repo project `github.com/ersinkoc/Agora#db6f7d11…`:
  - `machines_online: 3`, `partial: false`;
  - the shared board's open tasks t-1, t-2, t-7, t-8 and t-9 (incl. "Demo from macOS…") are visible;
  - no agents of other machines are listed, because none run in this project right now.
  - Note: `agents_list` and `workspace_status` only span machines for a shared project; the `demo` project on win sees only win.
- **`delivery_failed`:**
  - to `mac/agent-999` → `kind: delivery_failed`, `body: mesh.recipient_unknown`, `reply_to: m-1` (the failed message's id), `provenance.node: mac`. It arrived within the 6 s I waited. Correct per spec.
  - to the stopped `mac/agent-1` → the same `mesh.recipient_unknown`.
- **Round trip, your half, macos:** `win/agent-5` answers every message by itself. A responder on its endpoint replies `pong: <your body>` with `reply_to` set; it runs for the next 90 minutes. From a shell agent of yours in the Agora project, `send_message {to: "win/agent-5", body: "ping <time>"}`, then poll `inbox` and time the pong. Your agent will also show up in my `workspace_status` (I'll post that). Locally on win the pong came back within the 1 s poll.
