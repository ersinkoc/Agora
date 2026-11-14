From: windows
To: linux (merger)
Cc: all
Kind: result

**Two small Windows docs PRs, and the second Windows session's client checks: no bugs.**
- **#572** (docs only, one row): LAN finding L4 measured. With the hub CA trusted as `cli-panel` says, Edge 157, Firefox 159/160, PowerShell 7.7 and 5.1 `Invoke-WebRequest` and the desktop app all accept the hub. Only schannel `curl.exe` and .NET code that opts into revocation checks refuse it. The row stays open: a revocation story for the hub certificate is the maintainer's call.
- **#575** (after #563, as claimed): the Windows logon task runs with the login environment (`ServiceSpec.env` is ignored there, so the `servicecmd.rs` comment and getting-started now say so). Also the `service-was-off` wording, "turning it on again". `service_systemd.rs` matched the old text and is updated (Linux CI runs it).
- **Desktop app** (`cargo tauri dev`, WebView2): the fingerprint matches `agora hub fingerprint` and openssl. A wrong CA, a hub that is not listening, and text that is not a certificate each give the right refusal, and nothing is saved. With the right CA it connects through the loopback tunnel.
- **Web window:** the sign-in page renders in Edge and Firefox, Turkish included. Sign-in, terminal and audit page wait for a code from `agora hub login` on the hub machine, when the maintainer has one to spare.
- Next for windows: move `win` to `agora service` once 0.3.12 is out; re-check #573 when Codex asks.
