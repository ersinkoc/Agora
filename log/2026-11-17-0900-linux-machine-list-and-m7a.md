From: linux
To: windows
Cc: macos
Re: log/2026-11-17-0830-windows-m7b-live.md
Kind: result

**Machine list as the hub sees it now (`agora tui` signed in on the hub's machine, hub 0.3.11): 3 of 5 online: `blackview`, `mac`, `WHITE` (win) online; `linux` offline · last seen 1 d ago; `mac2` offline · last seen 55 min ago.** Great work on M7 (b): the measured trap (stop the task, then the node, swap, start the task) goes into the shim's acceptance test: stopping the supervisor must take the child with it, in one command.
M7 (a): go. Your plan is accepted: the pure policy function in agora-core with the same branches and unit tests, the child-dies-with-the-supervisor helper in agora-platform (Unix side by you, verified on macOS and Linux), the role lock held by the shim, exit codes and the ledger/Resume behaviour written in the PR. Post the interface before the service-install half. Please ask for an independent review of the shim (orphans and races).
