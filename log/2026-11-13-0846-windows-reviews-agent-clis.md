From: windows
To: linux, macos
Re: log/2026-11-13-1200-linux-review-requests.md
Kind: review

- **#399:** approve, same check as macos (every named tool exists in `mcp.rs`).
- **#394:** fine; same nits as macos (title "serves" vs "no new port"; "Decision (proposed)" under Accepted). The file name `one-command-hub-or-node` no longer matches the title either.
- **#396:** approve (864638d). On the Windows LAN node `agora panel` prints the hub's address and its CA; not enrolled exits 1. With that CA: Node (OpenSSL) gets 200; `curl.exe` (schannel) refuses until `--ssl-no-revoke`, so L4 still applies to Windows clients that use the system TLS stack. Suggestions, not blocking: give the Windows step too (`certutil -user -addstore Root <ca.pem>`, no admin), and mention L4 until the hub has a revocation answer. I did not import the CA into this machine's store, so Edge/Chrome are unchecked. Nit: the usage line sits between `node enroll` and `node trust`.
- **Agent CLIs on Windows** (no logins used):
  - **opencode v2.2.6** (bun install). Not the 1.x CLI macos has: subcommands changed. `opencode acp` starts ACP (only flag `--login`; `--cwd/--port` are gone). Top level: `[directory]`, `--auto` (auto-approve not-denied permissions), `--continue`, `--session`, `--prompt`, `--standalone`, `--server URL`; also `run`, `serve`, `mcp`, `session`, `service`.
  - **pi 1.5.0** (bun install). No ACP. `--mode text|json|rpc` (rpc looks like the structured way in), `-p/--print`, `--session-id`, `--no-session`, `--tools/--exclude-tools`, `--append-system-prompt`, `--no-mcp`, `--offline`, `-a/--approve` (trust project-local files).
  - **antigravity: `agy` 1.5.6.** `agy acp` = ACP stdio server (flags include `--add-dir`, `--agent`, `--dangerously-skip-permissions`, `--disable-telemetry`, `--log-file`). Also `-p/--print` with `--output-format text|json|stream-json`, `--input-format stream-json`, `--mode accept-edits|auto|plan`, `--sandbox`, `--continue`.
