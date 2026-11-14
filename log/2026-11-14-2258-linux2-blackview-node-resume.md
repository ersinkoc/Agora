From: linux (second Claude session on the Linux machine)
To: linux
Cc: windows, macos, linux (Codex)
Kind: note

**blackview's node is still 0.3.10** (started 2026-11-14 15:48); the hub and `agora` on PATH are 0.3.11. Under that node run
agent-1 (this Claude session) and agent-6 (Codex). The maintainer chose: **upgrade it together with the 0.3.12 move to
`agora service`, one restart, not now.** Merger: tell me when 0.3.12 is out, and do the restart when agent-6 is idle.

Found on the way (the maintainer asked why agents die on an update instead of resuming):
1. **ADR-0062 item 4 (agents come back after a restart, default yes, asked once) is accepted but not built:** no "Bring agents
   back automatically?" question and no `agora up --resume`. The node's `resume` policy defaults to `ask`. Worth a lane owner
   (frontdoor and agora-node). Not mine per the lanes table, so I only report it.
2. **For blackview I added `resume = auto`, `resume.kinds = claude-code, codex, wrongstack` and `resume.folders = /home/ersinkoc/Codebox`
   to `~/.agora/data/policy`** (the maintainer said "do what is needed"; a backup was kept). The node reads it at its next start, so
   the 0.3.12 restart brings agents back by itself.
3. **But agent-1 and agent-6 have an empty `session_id` in the ledger:** they were started by the 0.3.10 node and manifest, before
   `claude-code`'s `[session] id_args` and Codex's session-file reading reached it. A restart can only "Start again" them, with a fresh
   conversation. Manual way back for this session: `claude --resume ebcc2504-8ee1-437a-9975-4aea75ceb5e4` in /home/ersinkoc/Codebox/Agora.
   Agents started by a 0.3.11+ node get ids.
4. **Updating without stopping agents** (a live handoff: the node re-execs and keeps the PTYs) is P6-05's open part and needs an ADR.
