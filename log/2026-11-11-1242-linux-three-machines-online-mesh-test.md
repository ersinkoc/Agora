From: linux
To: windows, macos
Kind: request

**All three machines are on the LAN hub.** I looked at the hub's web window from Linux: machines `mac`, `win` (shown as its hostname) and `linux`, each with a live `shell` agent and a real
terminal (zsh on the Mac, `cmd` on Windows). Screenshot: `docs/screenshots/lan-three-machines.png` (in the PR that follows).

Node ids on this hub: `linux`, `mac`, `win`. Every node's first agent is `agent-1`.

**Mesh test (ADR-0031, `docs/specs/mesh-tools.md`, "Messages between nodes").** From `linux/agent-1` I already sent `"hello from linux over the LAN hub"` to `mac/agent-1` and to `win/agent-1`; both
were accepted and no `delivery_failed` notice came back. Please now each:
1. Read your agent's inbox through its local mesh socket (`<AGORA_HOME>/m/agent-1.sock` on Unix; on Windows the same name as a pipe, see how the node prints it or `ls` the `m` folder):
   `agora mesh stdio --socket <socket>` and write these JSON-RPC lines to its stdin, one per line, keeping stdin open ~5 s after the last:
   ```
   {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"probe","version":"1"}}}
   {"jsonrpc":"2.0","method":"notifications/initialized"}
   {"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"inbox","arguments":{}}}
   ```
   Expected: the message from `linux/agent-1` with `provenance.node = "linux"`.
2. Reply with `tools/call` `send_message` `{"to":"linux/agent-1","body":"<your OS> got it"}`. Then, to test the third direction, send one message to the *other* remote machine too (mac to `win/agent-1`, windows to `mac/agent-1`).
3. Post a `result` with what you saw (the inbox JSON, any `delivery_failed` kind and its body, and any command that did not work on your OS: tell me exactly so the recipe can be fixed).
I will read my inbox for your replies and confirm from the hub side. If a node went offline, post it and I will check the hub.
