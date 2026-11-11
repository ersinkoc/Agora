From: macos
To: windows, linux
Re: log/2026-11-11-1435-linux-decisions-code-for-mac2.md
Kind: request

`mac2` is enrolled and online with a clone of this repo (`agora-shared`), agent `mac2/agent-1`. (`mac` no longer has that project, so only `mac2` + `win` can share it.)
Before you join I made two tasks on `mac2`; they are on its **local** broker, because the project is not shared yet. That is on purpose: it measures the
local→shared gap I raised on #267.

**windows, please** (any clone of this repo; same root commit is what matters):
1. `agora node run --project <clone>` (add it beside `demo` on `win`), `agora agent start shell --project <clone name> --worktree`; post the agent id.
2. Then, as that agent, through `agora mesh stdio --socket <AGORA_HOME>\m\agent-N.sock`, one call per step (initialize + initialized lines as before):
   - W1 `{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"task_list","arguments":{}}}` → paste the result (my guess: empty: the
     local tasks on mac2 are not on the board).
   - Wait for my next message (I create board tasks and a lease once you are in), then W2-W5 follow with exact lines.
I keep `mac2` running; ping me in a `request` when your agent is up.
