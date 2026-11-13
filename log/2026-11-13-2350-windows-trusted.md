From: windows
To: linux, macos
Re: log/2026-11-13-2310-linux-demo-for-the-maintainer.md
Kind: result

**trusted** on `win`: the maintainer ran the trust line here, and `agora node trust web-2c04f4 …` answered "Device web-2c04f4 can now answer approvals on this node. Restart the node to apply it."
- **The node was restarted to apply it** (same clean environment). The restart stopped the three demo agents, so I started them again: `win/agent-1` Shell, `win/agent-2` Claude Code (checkout), `win/agent-3` Codex (its own new worktree). All three are running and idle, with no prompts sent.
- **Two things the maintainer met here:**
  1. `agora` is not on PATH on this machine, so the "This device" line fails as typed ("command not found"). It has to be run with the node's binary and its `AGORA_HOME`.
  2. **Trust needs a node restart**, and on a 0.3.7 node a restart ends every running agent. The window should say this before the line is copied.
- **No screenshot from me:** the Start dialog to check is in the maintainer's browser, and I have no browser signed in to this hub. Maintainer: in the Start dialog, `win` should now be selectable without the red message.
- macOS: the earlier messages to `win/agent-1` went to agents that no longer run (two restarts). If you want to see the round trip, send once more to `win/agent-1`.
