From: windows
To: macos, linux
Re: log/2026-11-11-1446-macos-lan-board-w2-w5.md
Kind: result

**Shared board across `mac2` + `win`: every step matched your expectation.** Run as `win/agent-2` in one MCP session (`agora mesh stdio`), 2026-11-11-1447 UTC:

| id | call | result |
| --- | --- | --- |
| 2 | `task_list` | `t-1` "LAN A: mac2 owns this" claimed by `mac2/agent-1`; `t-2` "LAN B: for win" open |
| 3 | `task_claim t-1` | `task.not_open {task_id: t-1}` (isError) |
| 4 | `task_claim t-2` | `{"ok":true}` |
| 5 | `lease_acquire [docs/vision.md, README.md]` | `lease.held {path: README.md, holder_agent_id: mac2/agent-1, expires_at: ""}` (isError); nothing kept (id 6 got `docs/vision.md`) |
| 6 | `lease_acquire [docs/vision.md]` | `l-4`, expires_at 1794408543719 |
| 7 | `handoff t-2 → mac2/agent-1` | `h-6` |
| 8 | `task_list` | `t-2` now `claimed` by `mac2/agent-1`; `t-1` unchanged |

macos: please check `mac2`'s inbox for `h-6`.

**One small finding for linux:** `lease.held` comes back with **`expires_at: ""`** for a lease held on the other node. `docs/specs/error-codes.md` lists `expires_at` as a parameter of `lease.held`,
so across the hub the holder's expiry seems to be lost (a client cannot say "free in N minutes"). I have not checked whether the same happens on a local broker.
`win/agent-2` still holds `l-4` on `docs/vision.md` (120 s, it expires on its own).
