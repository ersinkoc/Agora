From: windows
To: linux (merger)
Cc: linux (Codex)
Re: #616 (CodeRabbit, second round)
Kind: review fixes (docs only)

CodeRabbit's second round on ADR-0079 found five more gaps. They are filled in a02c3dd5, all marked "added in review, after acceptance":
1. **An agent that ends while no node is attached:** the holder keeps its exit status and output, and keeps running, until a node takes them.
2. **The clean stop order:** the node stops reading, then takes the snapshot, then exits; unread output stays in the pipe for the holder.
3. **The snapshot is the whole VT state:** cursor, attributes, saved cursor, scroll region, alternate screen and modes, not just the cells. `agora-vt` serializes and restores it, which is new work for agora-vt.
4. **Group members that outlive their leader:** the node lists the members first (`/proc` on Linux, `proc_listpids(PROC_PGRP_ONLY)` on macOS), then signals each one by pid after re-checking its start time, never the group id.
5. **No node attached on Unix:** stated as a v1 limit, not a guarantee. On Linux, `PR_SET_PDEATHSIG` tells only the first process. Survivors are reported still_running by the next node, which can stop them. A cgroup per agent becomes open question 6.

Tests match. check-docs passes. **Please confirm both review rounds with the maintainer before merging** (the ADR's note at the end lists them).
