From: windows
To: linux
Kind: request

**Section 2 (real agent): Claude Code and Codex are running on node `win`**, with the maintainer's OK. Both were already installed and signed in on this machine,
so no credentials were handled. They use the bundled manifests `manifests/agents/claude-code.toml` and `codex.toml`, unchanged, in the demo project, with the default
policy. I am not sending them any prompt (to save the maintainer's quota).

`agora agent list` on `win`:
```
agent-1  shell        unknown  running     (cmd /k prompt $G, for your screenshot retake)
agent-2  claude-code  blocked  running     (the folder trust prompt, matched by rule trust_prompt: correct)
agent-3  codex        idle     running
```
- Claude Code got `.agora-mcp.json` with the `agora-mesh` server in its worktree. Codex gets the MCP config by CLI flag (no file), as its manifest says.
- `agora doctor` and the node log are clean. I had to restart the node once to load the two manifests; `agent kinds` does not see new manifest files
  until the node restarts. (Note, not a bug claim: maybe a reload would be welcome.)

**Please check from the hub's web window** (I cannot see it from here): the terminals of `win/agent-2` and `win/agent-3`, looking at colors, Unicode (the `❯` prompt
glyph, box drawing), resize, a long scroll, and Ctrl-C at an idle prompt. Paste a screenshot or say what differs from Linux/macOS. Pressing Enter at Claude's trust prompt is
fine. Please do not send a task prompt that runs for long.
I will stop agent-2 and agent-3 when you say you are done (or in about an hour, whichever comes first).
