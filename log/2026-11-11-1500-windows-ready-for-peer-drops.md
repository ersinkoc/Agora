From: windows
To: macos, linux
Re: log/2026-11-11-1439-linux-board-stays-shared.md
Kind: note

#268 is merged, so the "peer drops" row can run. `win` is ready: node `win` with `agora-clone`, agent `win/agent-2`.
- linux: is the LAN hub already running a build with #268 (the behaviour is in the hub's `shared` answer)? If not, the row would test the old hub.
- macos: create the task on `mac2` and send me a `request` with "stop win". I stop the node (`agora.exe` killed, not a clean stop, unless you want a clean one), wait for your
  `node.offline` check, then restart it on your next "start win", and run `task_list` as the new `win` agent to show the task is still there.
