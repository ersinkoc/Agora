# Codex cross-review: web client, manifest engine, ACP bridge (2026-11-09)

Reviewer: Codex (`codex exec`, read-only sandbox, codex-cli 0.167.0), with controlled local reproductions for each finding. Author:
Claude Code. Scope: `apps/web/src` (untrusted text rendering, token and session handling, WebAuthn, view and reconnect logic) and
`crates/agora-adapters` (manifest parsing, rule engine, argv templating, MCP config writing, the ACP bridge) plus consistency of
`manifests/agents/*.toml` with the spec.

| # | Severity | Finding | Status |
|---|---|---|---|
| F001 | medium | The ACP bridge answered the node's `initialize` before the agent answered its own, so the node could send `session/new` too early and a strict agent refused it | Fixed: the node's `initialize` is answered when the agent's is (an agent error is passed on). Test with a slow fake agent in `acp_bridge.rs` |
| F002 | medium | A subscription retry scheduled on a dropped connection fired on the new one and subscribed a terminal or chat stream twice | Fixed: retries carry the connection's generation and do nothing on a newer one. Test in `live.test.ts` (it fails on the old code) |
| F003 | medium | `withTerminal` never shrank, so a reused agent id kept the terminal view instead of the prompt box | Fixed: cleared when an id starts again and when its node restarts. Test in `live.test.ts` |
| F004 | low | The manifest spec said `wrongstack` ships | Fixed: the spec says it is planned |

Confirmed fine by the reviewer: no XSS or injection sink (untrusted text is rendered as React text; no `dangerouslySetInnerHTML`, `innerHTML`
or markdown renderer; no data-driven `href` or `src`); the session token travels in a URL fragment and `sessionStorage`, never a query string
or `localStorage`; no WebAuthn client defect; manifest and MCP arguments are passed as argv (no shell), so `{session_id}` and `{config_file}`
cannot inject commands; MCP config paths are constrained and symlinked directories are refused when writing; the shipped manifests and their
captured screens conform.

With the two earlier reviews (`2026-11-09-codex-board-and-ssh-install.md`, `2026-11-09-codex-security-surfaces.md`) the node, hub, mesh, web
client and adapter engine have each had one Codex pass. Not covered: `agora-vt`, `agora-platform`, `agora-i18n`, the CLI (`crates/agora`), and
the build and release scripts.
