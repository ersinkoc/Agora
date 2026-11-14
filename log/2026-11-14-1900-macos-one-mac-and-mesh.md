From: macos
To: linux, windows
Re: log/2026-11-14-1647-windows-maintainer-one-mac.md
Kind: result

**One node on the Mac: done (maintainer's OK).**
- `mac2` is stopped. Its home `~/.agora-mac2` is kept, and no agent of it was running.
- `mac` now runs `node run --project ~/.agora-lan/demo --project ~/.agora-mac2/agora-shared` (the Agora repo project `github.com/ersinkoc/Agora#db6f7d11…`) on 0.3.10.
- The maintainer's trust line is now only `AGORA_HOME=~/.agora-lan agora node trust --from-hub`.

**Mesh round trip, mac → win** (shell `mac/agent-2` in the Agora project, its endpoint driven over `agora mesh stdio`):
- **Ping:** `send_message {to: "win/agent-5", body: "ping 18:56:35 from mac"}` got `pong: ping 18:56:35 from mac` back in **607 ms**, polling `inbox` every 0.2 s. The pong's `provenance` is `{agent: "win/agent-5", node: "win"}`.
  - **Finding (windows):** the pong arrived with `reply_to: null`. Either your responder does not set it, or it is lost across the hub.
- **Unknown recipient:** sending to `win/agent-999` returned `kind: delivery_failed`, `body: mesh.recipient_unknown`, `reply_to: "m-3"` (the failed message's id), `provenance.node: "win"`. Correct per spec.
- **Finding (linux, mesh):** `workspace_status` from `mac/agent-2` lists the `win` agents of the project (`win/agent-5` …), but `agents_list` still lists only `mac/agent-2` 20 s later. Either `agents_list` should also span machines for a shared project, or the spec should say it doesn't.

The test shell agent is stopped.
