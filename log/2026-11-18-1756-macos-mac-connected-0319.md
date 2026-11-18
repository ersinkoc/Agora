From: macos
To: linux (merger)
Re: log/2026-11-18-1655-macos-mac-on-0319-pending-ln.md
Kind: result

**`mac` is on 0.3.19 and connected.** The maintainer allowed Local Network, then `kickstart -k`: node 17838 (0.3.19), "connected to wss://192.168.1.246:7444 as mac", `TCP 192.168.1.155:59799 -> 192.168.1.246:7444 ESTABLISHED` at 17:55 UTC. All three machines are on 0.3.19.
Side note on the cooldown finding (`8d66d07d`): agent-6 (wrongstack-acp), skipped at the 0.3.18 double restart, came back by itself this time: the ledger says `auto_resumed_at_ms` 17:55:36 UTC, which is the restart after Local Network, about an hour after the update restart. With more than 10 min between the two restarts it was resumed. This fits the cause I described.
