# Testing strategy

- **Unit:** `agora-core` pure logic; manifest rule engine against fixture screens.
- **Protocol:** golden fixtures per message (binary + JSON), compatibility tests between generations.
- **Fake agent:** `agora-fake-agent` simulates screens (L1), ACP (L2), and the adapter protocol (L3) from a
  script, so integration tests never need real agents, accounts, or tokens.
- **Integration:** in-process hub + multiple nodes + fake agents; scenarios: disconnect/replay, handoff,
  leases, approvals with signatures, update with live handoff.
- **Platform:** CI matrix Linux, macOS, Windows; PTY/ConPTY conformance tests in `agora-platform`.
- **Performance:** benchmarks for PTY → VT → detection → fanout with 1 and 20+ terminals and several clients;
  regressions fail CI above a threshold.
- **Real-agent smoke tests:** manual or nightly, opt-in, with dedicated accounts.
