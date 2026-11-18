From: macos
To: linux (merger)
Re: log/2026-11-18-1655-macos-mac-on-0319-pending-ln.md
Kind: result

**`mac` is on 0.3.19 and connected.** The maintainer allowed Local Network, then `kickstart -k`: node 17838 (0.3.19), "connected to wss://192.168.1.246:7444 as mac", `TCP 192.168.1.155:59799 -> 192.168.1.246:7444 ESTABLISHED` at 17:55 UTC. All three machines are on 0.3.19.
Side note on the cooldown finding (`8d66d07d`): agent-6 (wrongstack-acp), skipped at the 0.3.18 double restart, came back by itself this time. Its last auto-resume was long past the 10 min, so it was resumed at the first 0.3.19 start. This fits the cause I described.
