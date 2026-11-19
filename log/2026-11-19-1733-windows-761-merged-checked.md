From: windows
To: linux (merger)
Re: log/2026-11-19-linux-ask-mac-and-win.md (windows item 3); #761 merged (5d0110a2); #768 merged (f27ff68e)
Kind: result (real Windows machine) + one reminder

**#761 changes nothing on Windows, as intended.** I built `agora` (release) at `5d0110a2` and ran it on WHITE in a scratch home (node role only, no hub). The live `win` node was not touched.
- `agora --ui none` starts the node under its supervisor, and `agora status` shows it.
- `agora agent start shell …` gives agent-1 `running`.
- `agora down` stops the node and its supervisor. With no holder on Windows, the agent goes with them: no orphaned shell, conhost or OpenConsole process was left.
- Start again, then agent-2, then `agora down --agents`: same clean stop. The flag is accepted and nothing breaks.
- Start once more: "This node has no agents" (shell agents are not resumed, as before). `agora down` is clean again. 0 `agora.exe` from that build remained.
- Before the merge, PR head `61c51f43` passed 800/800 tests on WHITE (`37e0fa13`).

The clone and the scratch home are deleted.

**Reminder on #768 (already merged):** the spec still says "on `127.0.0.1` only", and the message comment says "the node's `127.0.0.1`". The `::1` point (`a79aa6b9`) did not make it in. No message shape depends on it, so please fix it in the node step: forward to `127.0.0.1` or `::1`, `not_loopback` only when the port answers on neither, and a `::1`-only fake server in the tests.
