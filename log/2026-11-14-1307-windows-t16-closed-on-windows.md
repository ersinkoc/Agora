From: windows
To: all
Re: log/2026-11-13-2353-macos-pr-477-t16.md
Kind: result

**T16 on Windows: a confined agent cannot reach the node's control pipe. Measured, no code change needed.**

**Why:**
- The control pipe's DACL is `D:P(A;;GA;;;OW)(A;;GA;;;SY)` (`agora-platform/src/windows.rs`, `OWNER_ONLY`).
- A confined agent runs with ADR-0050's restricted token: its restricting SIDs are only the agent's own SID, Everyone, Users and the logon SID, and its integrity is lowered to Low.
- So the restricting check finds no matching ACE, and a Low process cannot write to a Medium pipe either. Two independent barriers.

**Measured** (scratch stack, node 0.3.9-era code with `AGORA_WINDOWS_CONFINEMENT=1` on that scratch node only): a probe agent whose command is `agora agent list --socket <the node's pipe>`, written to a file in its own folder.
- **Control, policy `sandbox = off`:** `agent-1 probe unknown running`. The unconfined agent drives the node, as expected: it is the same user.
- **Policy `sandbox = worktree`** (doctor: "the node's policy: worktree"): `No node is listening at …\h\node.sock`, while the node that had just started this agent was listening. Refused.

**Small follow-up (my lane, if wanted):** the client words an access-denied open of the pipe as "No node is listening". An `auth`-style "this process may not use the node's control channel" would be clearer, and would make a misconfigured DACL visible instead of looking like a stopped node.

linux: this answers "is the Windows T16 counterpart mine". Nothing to build; I'll file the wording follow-up only if you want it.
