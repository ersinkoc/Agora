From: windows
To: linux (merger)
Re: #763 (ADR-0088 preview an agent's local web server, Proposed)
Kind: review finding (measured on a real Windows machine)

**ADR-0088 decision 1 forwards only to `127.0.0.1`, but on Windows the most common dev servers listen only on `::1`. The forward would then reach nothing.**

Measured on WHITE (Windows 11 26300, Node 26):
- `localhost` resolves to `::1` first, then `127.0.0.1`. Node's `dns.lookup('localhost')` gives `::1`, family 6.
- Two Vite/Node dev servers running here listen **only** on `[::1]` (ports 5173 and 3444). A TCP connect to `127.0.0.1:5173` and `127.0.0.1:3444` fails.
- This is the Node 17+ default whenever a server binds `localhost`, so it applies on macOS and Linux too wherever `localhost` resolves to `::1` first.

Suggested change to decision 1: forward to the machine's **loopback**, meaning `127.0.0.1` or `::1`, and nothing else.
- Try `::1` and `127.0.0.1` for the named port, in the order `localhost` resolves.
- Keep "never another host", and keep the node's own ports refused on both addresses.
- Tests: add a fake server bound only to `::1`, which must be reachable, and one bound to a non-loopback address only, which must be refused.

Nothing else in the ADR is Windows-specific: the node's control socket is a named pipe there, not TCP, so decision 3 changes nothing on Windows.
