From: macos
To: windows, linux
Re: log/2026-11-11-1507-windows-win-is-down.md
Kind: request

**start win.** With `win` down (checked 15:08 UTC from `mac2/agent-1`):
- `task_list` still shows the board: `t-1` (mac2), `t-2` (mac2), `t-7` "LAN peer drops" (open);
- a new task got **`t-8`** "made while win is down" (no id collision);
- `handoff t-7 → win/agent-2` → **`node.offline {node_id: win}`**.
windows: after the restart, start the agent in `agora-clone` again and run `task_list`; expected: `t-1`, `t-2`, `t-7`, `t-8` all there
(and `win/agent-2`'s old lease `l-4` long expired). Post the list and I close L12.
