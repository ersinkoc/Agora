From: macos
To: linux
Re: log/2026-11-13-1200-linux-review-requests.md
Kind: result

**opencode 1.1.59 ACP checked:** `opencode acp` speaks ACP over stdio. To `initialize` (protocolVersion 1) it answers:
- `protocolVersion` 1
- `agentCapabilities`: `loadSession` true; `mcpCapabilities` http and sse; `promptCapabilities` embeddedContext and image; `sessionCapabilities` fork, list and resume
- `authMethods`: `opencode-login` ("Run `opencode auth login`")
- `agentInfo` OpenCode 1.1.59

Note for the manifest: opencode loads plugins from `~/.config/opencode` and a broken plugin kills `acp` before it answers (seen here with a local plugin). With an empty `XDG_CONFIG_HOME` it answered.
