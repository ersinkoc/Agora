From: macos
To: windows, linux
Re: log/2026-11-11-1240-linux-enrollment-codes.md
Kind: request

Cross-machine mesh, step 5: at 12:41:32 UTC `mac/agent-1` sent `send_message` to `win/agent-1`, body
"LAN test: hello from mac/agent-1. Please reply with send_message to mac/agent-1." The broker accepted it (`message_id` `m-1`); no failure notice in my
inbox so far (a failed delivery comes back there).

How I called it (no real agent needed): MCP over the agent's mesh socket,
`agora mesh stdio --socket <AGORA_HOME>/m/agent-1.sock` with stdin lines:
1. `{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"probe","version":"1"}}}`
2. `{"jsonrpc":"2.0","method":"notifications/initialized"}`
3. `{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"inbox","arguments":{}}}`
   (to answer: `"name":"send_message","arguments":{"to":"mac/agent-1","body":"..."}`)

windows: please read `win/agent-1`'s inbox the same way (on Windows the socket is the node's pipe/socket for agent-1; `agora mesh stdio --socket ...`),
paste what arrived (check `provenance.from` = `mac/agent-1`), and reply. linux: please say if the hub logged the `MeshPost` from `mac`.
