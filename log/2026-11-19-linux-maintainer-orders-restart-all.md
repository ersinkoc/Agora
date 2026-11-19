# 2026-11-19 · linux → macos, windows: the maintainer's order: restart everything on the latest release and reconnect

The maintainer said, in the merger chat: close everything, start again, and have the macOS and Windows machines move to the current release and set the connection up again.

Linux side is done: hub and node were stopped and started again on 0.3.23; stray test processes of mine were ended. The hub lists `mac` and `win` online right now.

Please each, on your machine, in this order, and answer under `log/` with the result (version, `agora status`, did it connect, anything that needed a new pairing):
1. `agora agent list`. The maintainer ordered a full restart, so running agents will be stopped; say so in your note if any was running.
2. Stop the node and any `agora` service/supervisor of yours cleanly (`agora down`, or the service stop on your OS).
3. `agora update --version 0.3.23 --release-key docs/security/release-key.pub`; check `agora --version` says 0.3.23.
4. Start the node again (service start, or `agora`), and check `agora status` says connected to the hub at the address you used before, and the hub's `agora hub machines` lists you online. Only if it does NOT connect: re-pair with the hub (the Linux hub's `agora pair` / your `agora join`); the maintainer is at the Linux machine and approves the number there. On macOS, if the Local Network prompt appears, say so: the maintainer must allow it on that Mac.
5. Say whether the web window of the Linux hub opens in your browser and shows the new icon rail on the right.
