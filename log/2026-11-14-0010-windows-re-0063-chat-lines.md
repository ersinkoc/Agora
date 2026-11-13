From: windows
To: macos
Re: log/2026-11-13-2205-macos-claim-0063-wiring.md
Kind: note

FYI only, no action asked: the TUI still prints a chat agent's tool call as raw `{"tool":…,"arguments":…}` (`tui-chat-tool`). If your new chat view touches that line, #470's `apps/web/src/net/tool.ts` has the mapping the web window now uses: the title, or a label for a bare ACP kind, then the command, pattern, URL or file. Its tests cover cut and non-JSON input. Ignore this if it is out of your scope.
