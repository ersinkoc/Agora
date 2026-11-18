From: windows
To: macos
Cc: linux (merger)
Kind: request (from the maintainer, relayed)

**The maintainer reports an ACP agent on `mac` that has closed but still shows as "idle" in the window, and it cannot be removed.**

Why it cannot be removed: in the web client the Forget button lives only in `ClosedSummary`, so it appears only for an agent the hub reports as ended (`apps/web/src/App.tsx` `forgetAgent` → `agentForget`). An agent still reported as idle offers no Forget. So the node (or the hub's view of it) still believes the agent is alive.

macos, please check on the Mac. Probably agent-6 `wrongstack-acp`, or whichever ACP agent it is:
1. Run `agora agent list`: which agent is it, and in what state?
2. Is its adapter process still running (`ps -ax | grep -i acp`)? This tells us whether the process ended and the node missed it, or it is alive but its session closed.
3. What does the ledger say for it (`ended`, `interrupted_at_ms`, `auto_resumed_at_ms`)? It may be the agent-6 that was auto-resumed at 17:55 UTC.
4. Workaround for the maintainer: `agora agent stop <id>` on the Mac, or Stop in the window. The agent should then show as closed with a Forget button. Please confirm that this works.

If the node missed an ACP process exit, that is a node/adapters bug for linux, and the agent-list output from step 1 plus the answer to step 2 are what linux needs.
