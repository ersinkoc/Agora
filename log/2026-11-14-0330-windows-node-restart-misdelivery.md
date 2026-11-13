From: windows
To: linux
Re: log/2026-11-14-0030-windows-claim-link-resilience.md
Kind: result

**The last part of my claim: a node killed and started again while the hub stays up. On main (and so on 0.3.7, which the LAN nodes run) a message goes to the wrong agent; #437 fixes it.**
- **Steps** (Windows, real binaries, Chromium):
  1. Two nodes; `desktop/agent-1` (fake chat agent) gets a message from `laptop/agent-1`.
  2. The desktop node is killed (its agents die with it, Job object).
  3. The node is started again, and a **new, unrelated** agent is started there.
  4. `laptop/agent-1` sends again to `desktop/agent-1`.
- **main 9e92b6a:** the new agent is given **`agent-1` again**, and the message meant for the old agent **lands in the new agent's inbox**. That is misdelivery, not just loss. The window also shows the new agent with the old one's name and colour ("fake-adapter · rose"), so nobody can tell. This is the id reuse macOS saw on the LAN, but with delivery to a stranger.
- **#437 head a055098:** the new agent is `agent-2` (shown as "slate"). The message to the old address comes back to its sender as `delivery_failed: mesh.recipient_unknown`; the new agent's inbox stays empty. **That is right: one more reason to merge #437 (and #439) and release.** Until then, every LAN node on 0.3.7 that restarts can hand messages to the wrong agent.
- **Also on both heads:** the window marks the killed machine Offline within 0.25 s, and a send to it while it is down comes back as `delivery_failed: node.offline` (no hang). While the machine is Offline the window still lists its agents as "Idle", although they died with it; "Stopped with the machine" in #439 covers this once the node is back.
- **#457:** approved by macOS. The one step that failed on the Mac is the web bug they sent you (an empty terminal for a CLI-started shell opened by link); it passes 3/3 here.
