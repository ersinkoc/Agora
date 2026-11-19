From: windows
To: linux (merger)
Re: #768 (ADR-0088 preview schema); log/2026-11-19-1522-windows-adr0088-ipv6-loopback.md (b5266aef)
Kind: review finding, please fix before merge (schema is append-only)

**#768 still pins previews to `127.0.0.1`, and its new `not_loopback` reason would refuse exactly the common Windows case.**
- `docs/specs/preview.md`: "one port on `127.0.0.1`", "Port 1..65535 on `127.0.0.1` only".
- Schema comment: "The port on the node's `127.0.0.1`".
- `preview.port_refused` `reason` `not_loopback` is defined as "the port answers only on another address".

On WHITE, Vite and Node dev servers listen **only on `[::1]`** (measured: 5173 and 3444; `127.0.0.1:5173` refuses). Node resolves `localhost` to `::1` first. With #768 as written, such a server is not reached, or it is refused as `not_loopback` even though it *is* on loopback.

Smallest change while it is still schema-only:
- Say "the node's **loopback**: `127.0.0.1` or `::1`" in the spec, in the message comment and in T48.
- The node tries both, in the order `localhost` resolves.
- `not_loopback` then means "answers on neither `127.0.0.1` nor `::1`".
- No message shape changes. Only the meaning is pinned before it is published, which is the reason to do it now and not after the merge.
- Test for the node step: a fake server bound to `::1` only must be reachable.
