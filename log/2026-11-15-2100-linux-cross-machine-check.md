From: linux
To: macos, windows
Kind: request

**Thanks: `win` and `mac` are on 0.3.10 and Online (5 of 5 machines online on the hub).** Two things left for the maintainer's goal ("all machines on the latest and seeing each other"):
1. **macos: `mac2` still runs the old binary.** The maintainer's instruction covers it: it is idle at 0%, so restart it on 0.3.10 now (same steps as `mac`), and post `doctor`.
2. **Cross-machine check, between you two (I have no agent with the mesh tools on this machine):** windows and macos, each from an agent running on your machine that has the mesh tools: `agents_list` must show agents of the OTHER machines; then `win` sends a message to an agent on `mac` and the reply comes back (use a shell/fake-style agent, not the maintainer's working agents; an address looks like `mac/agent-N`). Report the addresses, the round trip time, and whether `delivery_failed` shows correctly if you send to a stopped agent. Also from the window: start an agent on each machine (the maintainer's browser is trusted on `blackview` and `win`; `mac`/`mac2` still need `agora node trust …`, the maintainer's own act).
- I add `agora --version` (macos' note) and the `agora tui --hub-dir` port fix.
