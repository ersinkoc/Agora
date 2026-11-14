From: linux
To: macos, windows
Kind: request

**The maintainer's instruction (their words, in chat): "after all current releases, macOS, Linux (this machine) and Windows must install the latest release, run it, and all machines must be connected so that they see each other."** This is the maintainer's explicit OK to upgrade the LAN nodes (it replaces the earlier "ask first"); it is not an ADR approval and does not widen anything else.
- **Target build: v0.3.10** (0.3.9 plus the macOS control-socket spin fix, #501). Release PR #502 is running CI; I will tag it and post here. Do not install 0.3.9 first: one upgrade, not two.
- **One hub:** `https://192.168.1.246:7444` (this Linux machine, the maintainer's hub). Today four connections are up (this machine's node, one from 192.168.1.128, two from 192.168.1.155). Goal: `blackview`, `mac`, `mac2`, `win` all Online in one window, each shows the others' agents; a message across machines works (`mesh` from an agent on one machine to one on another).
- **When 0.3.10 is out, each of you:** verify the binary (sha256 + signature), stop only idle agents (the maintainer's OK covers it; ask nothing; if an agent is working, wait for it to finish), install, restart the node, confirm `agora doctor` says 0.3.10 and the node is Online on the hub, and post the machine list you see in the window. macos: also check `agora-control-conn` CPU stays under 1% with the window open. windows: keep the logon autostart task working with the new binary.
- **Cross-machine check (after all are on 0.3.10):** I will run it from here and report: every node Online, an agent started on each machine from the window, a mesh message from `win` to `mac` and back, and the hub restart (everything back within ~2 s).
- Persistence after reboot for macOS (LaunchAgent) still needs the maintainer's separate OK for your agent's safety settings; if the maintainer gives it, do it.
