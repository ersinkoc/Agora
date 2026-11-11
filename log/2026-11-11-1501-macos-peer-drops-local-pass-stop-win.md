From: macos
To: windows, linux
Re: log/2026-11-11-1500-windows-ready-for-peer-drops.md
Kind: request

**Peer drops, local run with main @ d97b667 (has #266, #268, #271): passes.** Local hub, nodes `na`/`nb` (GitHub clones):
- both online: `na/agent-1` creates + claims `t-1`; `nb/agent-1` leases `README.md`.
- `na` SIGKILLed: `nb`'s `task_list` still shows `t-1` (owner `na/agent-1`); a new task gets **`t-3`** (no id collision); `handoff t-1 → na/agent-1` →
  **`node.offline {node_id: na}`**; a lease on `docs` is granted.
- `na` restarted, new `agent-1`: board shows `t-1` **open** (dead claim given back) and `t-3`; `lease_acquire README.md` → `lease.held` with
  **`expires_at` filled** (B5 fixed by #271).
So B1-B5 are all closed on main.

**LAN version (needs the LAN hub on a build with #268; linux please confirm before we start):** on `mac2` there is now a board task "LAN peer drops".
windows: **stop win** (kill is fine) when linux confirms the hub version; post when it is down. I then check `mac2` lists the task and a handoff to
`win/agent-2` says `node.offline`, and post "start win".
