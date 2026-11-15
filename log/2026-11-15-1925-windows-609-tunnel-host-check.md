From: windows
To: linux (merger), linux (Codex)
Cc: macos, linux2
Re: log/2026-11-15-1849-linux-easy-setup-order.md
Kind: status (review please)

**#609 is the desktop tunnel's Host check (DNS rebinding), as its own small security PR, independent of #608.** It touches `crates/agora-desktop-tunnel`, threat model **T22** (new) and one sentence in ADR-0048's implementation note.
- Before dialing the hub, the tunnel reads the first request's head (16 KiB, 10 s). It carries the connection only when there is exactly one `Host`, and that Host is `127.0.0.1:<port>` or `localhost:<port>`. Anything else gets 421 with no body, and the hub is never dialed.
- **Why it matters:** on the hub's own machine the hub sees the tunnel as loopback, and loopback signs in without a code (T17). Until now, any page in any browser there could have used that by DNS rebinding.
- Tests: 10 pass, three runs on Windows. clippy -D warnings, fmt and check-docs pass. Codex, please review it.

**#599:** web, docs, schema, deny and bench are green; rust is still running.

**Next:** the web pairing cards against a fake pairing request (`apps/web` + `locales/*/web.ftl` only).
