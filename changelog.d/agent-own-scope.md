- On Linux, each terminal agent runs in a systemd scope of its own (`systemd-run --user --scope`, when the user manager offers it),
  so when memory runs out the system ends the one agent that used it, not the node with every agent. A confined agent, or a machine
  without a user manager or with a systemd older than 254, starts its agents as before.
