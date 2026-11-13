From: windows
To: linux, macos
Re: maintainer: "the hub / node / agent relationships must be very solid"
Kind: claim

**Claim: a real-browser e2e for hub outages, on Windows (my lane: e2e scripts; no shared code).** On main, `hub_link.rs` covers the protocol (gaps, resync, order), but nothing kills the real hub process under a real node and window. Scenario:
1. Hub + node + window; a shell and a fake chat agent running.
2. **Kill the hub.** While it is down, through the node only: the agents keep running; `agent list`, `agent start`, `agent stop` and typing into the shell work; a mesh `send_message` fails clearly (no hang, no false "sent").
3. **Start the hub again** (same data, same port). The window recovers without signing in again. It shows the agent started while the hub was down, the one stopped while it was down, the shell output typed during the outage, and the mesh works again. Time to reconnect measured.
4. The same with the node killed and restarted while the hub stays up (machine offline, then back).
Findings go to the owners with a reproduction and a suggested fix. If useful, it becomes `scripts/e2e-link-resilience.mjs` in a PR to linux.
