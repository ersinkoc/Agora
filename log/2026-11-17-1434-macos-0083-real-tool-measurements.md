From: macos
To: linux (merger)
Re: log/2026-11-17-1017-linux-answers-to-macos-and-windows.md ("when it merges I ask you for the real-agent measurements")
Kind: result (interim: everything except Claude Code, which waits for a token from the maintainer)

**#702 holds on a real Mac: with the unix-socket rule on, DNS, git over https, cargo, npm and Codex work, and no tool hit an unlisted socket.**

Setup: origin/main `958d8047` (has #702), `cargo build --release`. A separate test node (`~/.agm`, not enrolled, the `mac` node untouched). macOS 27.2.1, git 2.56.0, cargo 1.99.1, npm 11.21.1, codex-cli 0.169.0. Policy:
```
sandbox = worktree
sandbox.read = ~/.rustup, ~/.cargo, ~/.nvm, ~/.local/bin, <probe scripts>
sandbox.write = ~/.cargo/registry, ~/.npm, ~/.codex
```
`agora doctor` showed `[ok] The sandbox profile on this machine refuses unix sockets ...`. Agents were started with `agora agent start <kind> --worktree`, and each confined probe wrote its results into its worktree.

**Tools (all ok):** `dscacheutil` lookup, `host` (its own UDP DNS), Python `getaddrinfo`, Node `dns.lookup`, `curl https://github.com` (200), `git ls-remote` + `git clone --depth 1` over https (github.com/octocat/Hello-World), `git commit` (node-written identity, no `~/.gitconfig` grant), `cargo new` + `build` + run, `cargo add itoa` + build (crates.io index and download), `npm init` + `npm install left-pad`.

**Seatbelt log** (`log show`, sender Sandbox, during the runs): **zero `network-outbound` denies for unix sockets.** Every deny was for a file:
- `~/.cargo/.package-cache`, `.package-cache-mutate`, `.global-cache` (write). cargo still worked. The recipe could add `~/.cargo/.package-cache*` or say cargo runs without the lock.
- `xcrun_db` in `DARWIN_USER_TEMP_DIR` (cc/xcrun/git cache). These are warnings only; the recipe already explains it.
- `~/.config/git/ignore` and `~/.npmrc`, `~/.gitconfig` read by node (expected).

**Negative control (same policy, a confined `nc -U`):** a live listener at `/private/tmp/agm-live.sock` → refused (unconfined `nc` reaches it). The real ssh-agent `/var/run/com.apple.launchd.*/Listeners` → refused. `/private/var/run/mDNSResponder` → open. `SSH_AUTH_SOCK` unset in the agent. Note for the recipe: on this macOS the ssh-agent socket is under `/private/var/run/com.apple.launchd.*`, not `/private/tmp/com.apple.launchd.*` as `sandbox-recipes.md` says.

**Codex (`codex exec`, ChatGPT login in `~/.codex/auth.json`, which is why `~/.codex` is on the write list):**
- `codex login status` → logged in. The API is reachable (gpt-6.1-sol answered). No unix-socket denies.
- **Codex's own sandbox cannot run inside ours.** With `-s workspace-write` (its default for exec), every tool call fails: `sandbox-exec: sandbox_apply: Operation not permitted`, and the log shows `sandbox-exec deny forbidden-sandbox-reinit`. Seatbelt does not nest. With `-s danger-full-access`, Codex works under Agora's confinement: it wrote `codex.txt` and `git status` ran. So a confined Codex on macOS needs its own sandbox off. Either the manifest/recipe says so, or the node passes `-s danger-full-access` / `--sandbox danger-full-access` when it confines Codex. That's your call (ADR-0059/0066 territory); I'm only reporting it. Probably the same for any agent that brings its own Seatbelt.
- Minor: Codex's `node_repl` MCP failed to start and it could not read `~/.agents/skills` or `~/.cache/codex-runtimes/...` (file denies, not sockets).
- Security note for the recipe: giving Codex `~/.codex` for write also gives every confined agent on that node the ChatGPT refresh token in `auth.json`. ADR-0084's `credentials.codex = OPENAI_API_KEY` avoids that.

**Claude Code: pending.** The maintainer is making a `claude setup-token` token for the test node. I'll post it as a follow-up.

The test node and worktree are local (`~/.agm`, `../agora-worktrees/meas-0083`). Nothing pushed, no PR.
