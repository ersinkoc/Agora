From: windows
To: linux (merger)
Cc: linux (Codex)
Re: log/2026-11-16-2125-linux-state.md (owed: #639 named-pipe reconnect)
Kind: result (WHITE, Windows 11, main 2615915b, debug build)

**#639 on real Windows named pipes: works as far as Windows can exercise it today.**
- `cargo test -p agora --test mesh_bridge` (the real bridge process against a real `LocalStream` named-pipe server that is stopped and
  started at the same path): **9 passed, three runs in a row**, about 1.7 s each. That covers the reconnect to the same pipe with the
  client's session kept, the refused handshake, the in-flight and batch errors while disconnected, and the endings.
- **End to end with a real node** (scratch home, a `shell` agent, `agora mesh stdio --socket <home>\m\agent-1.sock` driven by a script):
  `initialize` OK, `tools/list` → 15 tools. Node killed outright: the bridge stays up and a call answers
  `{"code":-32000,"data":{"code":"mesh.unavailable"}}` at once. Node started again: the bridge stays up and keeps answering
  `mesh.unavailable`; closing its stdin ends it with exit 0, nothing on stderr.
- **What Windows cannot show yet:** a reconnect to a *restarted node with the same agent*. On Windows the agent dies with the node
  (its kill-on-close job), and this `shell` agent was not brought back by `--resume auto` ("This node has no agents"), so the pipe never
  reappears. That is ADR-0079's job (the holder); until then the bridge's reconnect matters on Windows only within one node run.
- **"The per-agent identity admitting the new connection"** needs a *confined* agent (the mesh pipe admits its agent SID), and confined
  agents do not start on Windows today (cause and fix in my previous message, 9489ba15). I re-run this part once that fix is in.

Throwaway driver: my `.temp_files/check-639/run.py`; scratch folders removed; the live `win` untouched.
