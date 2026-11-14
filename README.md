# Agora

> Working name. Pre-alpha: the hub, node, mesh and web window run end to end on Linux, macOS and Windows.
> Real-agent fixtures and multi-machine runs are recorded; integration coverage and remaining limits are listed below.

One window for every AI coding agent on every machine you own.

Agora runs a small **node** on each of your machines (Linux, macOS, Windows, local or reachable only over
SSH). All nodes report to a **hub**. You open one window and see your projects, agents, files and changes,
and the approvals waiting for you. Agent integrations use manifests, ACP or external adapters; a generic CLI can run
in a terminal. Agents coordinate through a structured mesh: messages, a shared task board, file leases, and handoff.
Nodes keep agents running when the hub or window disconnects.

## Install a release

Download the binary for your platform from [releases](https://github.com/ersinkoc/Agora/releases), along with its
`.sha256` checksum and `.sig` signature. Verify signed releases with the key in
[`docs/security/release-key.pub`](docs/security/release-key.pub). These commands assume all three files are in the current directory:

```sh
f=agora-x86_64-unknown-linux-gnu   # or aarch64-unknown-linux-gnu, aarch64-apple-darwin, x86_64-apple-darwin, x86_64-pc-windows-msvc.exe
sha256sum -c $f.sha256
ssh-keygen -Y verify -f docs/security/release-key.pub -I agora-release -n file -s $f.sig < $f
chmod +x $f && ./$f doctor
```

On macOS the binary is ad-hoc signed but not notarized yet. Downloaded with `curl` or `gh release download` it runs; downloaded with a browser, macOS
blocks it ("cannot be verified"): run `xattr -d com.apple.quarantine agora-aarch64-apple-darwin` first. Notarization needs a Developer ID (open question).

`agora update` checks and installs newer releases the same way once the key is saved as `<agora dir>/release-key`.
The web window is embedded in the binary, so steps 2 to 4 below work with a release too.

## Get started

Type `agora`. The first time it asks one question; then it starts everything this machine needs in the background and opens
the terminal client signed in (`agora web` opens the browser). See [docs/getting-started.md](docs/getting-started.md).

## Try it (from source)

These examples use a POSIX shell. For Windows setup, see the [Windows development guide](docs/fordevs/for_windows_dev.md).

```sh
# 1. Build the web window, then the binary (it embeds the window; skip the first line to serve it with --web DIR instead)
(cd apps/web && npm ci && npm run gen && npm run build)
cargo build --release -p agora

# 2. The hub: prints the address and a one-time sign-in code (data lives in ~/.agora/hub)
target/release/agora hub run --listen 127.0.0.1:7443

# 3. A machine: get an enrollment code from the hub, enroll, run the node on a git project
target/release/agora hub code
target/release/agora node enroll --hub wss://localhost:7443 --ca ~/.agora/hub/ca.pem --code XXXX-XXXX --id laptop
#    The node offers every agent in manifests/agents/ built in (Claude Code, Codex, WrongStack, ...; `agora doctor` lists
#    them and whether each program is found). Files in ~/.agora/manifests replace or add kinds (format:
#    docs/specs/agent-manifest.md); the built-in Shell (your own shell) shows the whole flow without any agent installed:
target/release/agora node run --project ~/code/myproject
#    in the window, "Start an agent"; or in another terminal:
target/release/agora agent start shell --cwd /path/to/myproject

# 4. Open https://localhost:7443/ , accept the hub's own certificate, sign in with the code from step 2
#    (`agora hub login` makes another). Your agents and their terminals appear in the window.
```

To use an installed coding agent, start its kind; for example, with Codex installed and signed in on the node machine:

```sh
target/release/agora agent start codex --cwd /path/to/myproject
```

A manifest does not install the agent or sign in to its account.
See [agent manifests](manifests/agents/README.md) for the available kinds and integration details.

Agents start in the folder you select. In the web app, choose a machine, a project or another folder, then the agent.
Several agents can share that folder. Choose **Create an isolated working copy** (CLI: `--worktree`) when you want a separate Git worktree.
Folders without Git also work. Adding a folder does not require restarting the node.

To group copies of a project across machines, run `agora project init --cwd /path/to/myproject`, or choose **Create a shared project ID**
in the launch form. This explicitly adds `.agent-project.json`; commit or copy that file to the other machines. It groups the sessions;
it does not synchronize source files. Existing repositories without metadata keep automatic Git-based discovery. An independent fork can
remove its copied metadata and run `project init` to create a new ID; changing a previously used identity requires explicit confirmation
at launch (`--accept-project-id-change` in the CLI).

## Status

Pre-alpha. This describes the current source tree; see [CHANGELOG.md](CHANGELOG.md) for release-by-release changes.

- **One console across machines:** live terminals side by side, agent states, workspace files, diffs, commits, messages and estimated spend.
- **Structured conversations:** chat, plans, tool calls, diffs and approvals for ACP agents through `agora acp-bridge` and for external adapters.
- **Coordination:** per-agent MCP endpoints, cross-node messaging, shared project tasks, path leases and handoff. Shared-board calls fail when the hub is unreachable; local agent execution continues.
- **Clients:** the web app, a [Tauri desktop shell](apps/desktop/README.md) that pins the hub CA, and a [TUI](docs/adr/0043-tui-client.md) for terminals, chat, approvals and agent control. `agora panel` prints the enrolled hub's web address and CA location; the person still signs in.
- **Accounts and operations:** passkey sign-in, device-signed approvals, team roles and project access, audit export, hub backup/restore and signed binary updates. English and Turkish interfaces are included.

### Agent integrations

| Manifest | Interface | Recorded validation |
| --- | --- | --- |
| `claude-code` | Native terminal with state detection | Real terminal captures, including trust and permission prompts |
| `codex` | Native terminal with state detection | Real terminal captures; MCP configuration checked, but the recorded live mesh call stopped at the user's hook-review prompt |
| `wrongstack` | Native terminal with state detection | Real captures, resume and a live mesh task creation |
| `wrongstack-acp` | Structured conversation over ACP | A real turn through the bridge and node is recorded |
| `opencode-acp` | Structured conversation over ACP | Manifest included; a real program run through the bridge is still pending |
| `antigravity-acp` | Structured conversation over ACP | Manifest included; a real program run through the bridge is still pending |

The [phase 4 record](docs/backlog/phase-4.md) describes the captured versions and the limits of each run.
The shipped Claude Code and Codex integrations use terminals; they do not provide structured ACP conversations.

### Validation and remaining work

- **Real machines:** the [2026-11-11 LAN run](docs/backlog/lan-test-findings.md) used a Linux hub with Linux, macOS and Windows 11 nodes, including real Claude Code and Codex on Windows. It records cross-node messages, shared-board behavior and unresolved platform issues.
- **SSH:** `agora hub ssh add` maintains a reverse tunnel, and `agora hub ssh install NAME` installs and enrolls a node. The [real SSH run](docs/reviews/2026-11-09-real-ssh-run.md) used a private loopback `sshd`, including installation of a published signed release and rejection of a wrong signing key. That run did not cover a working user service manager, jump hosts, passphrases or macOS/Windows installation targets.
- **Registry:** adapter install/update, signature checks and publisher revocation have local registry tests. A production registry, hub mirroring and automatic updates remain unfinished; see [phase 6](docs/backlog/phase-6.md).
- **Desktop and mobile:** desktop builds and unsigned installer artifacts exist; signed installers, notarization and native background approval notifications remain unfinished. The mobile web app has a PWA manifest and notification links, with no service worker or Web Push; see [phase 7](docs/backlog/phase-7.md).
- **Planned extensions:** helper-agent start/wait/output tools ([ADR-0057](docs/adr/0057-agents-start-agents.md)) and the optional direct terminal/file path ([ADR-0023](docs/adr/0023-direct-data-path.md)) are accepted directions, not implemented features in this source tree. Terminal streams currently use the hub relay.

## Security and isolation

Approvals are signed by a device the node trusts and verified on the node. The window's **This device** page shows the
`agora node trust` command to run on that machine. Routed-command grants depend on the node's `signed_commands` policy;
the default single-owner policy does not require grants for legacy routed commands. Selected-folder inspection and launch
use exact-request signatures separately. Configure the policy before relying on it to contain a compromised hub.
See the [threat model](docs/security/threat-model.md) for device limits and the complete authority rules.

Confinement is opt-in with `sandbox = worktree` in the node's policy on supported Linux and macOS systems.
Windows confinement remains disabled by default and has documented compatibility limits. A worktree isolates Git changes;
leases coordinate cooperating agents and node-mediated writes, but cannot stop an agent's direct shell writes.
See [sandbox recipes](docs/security/sandbox-recipes.md) and the [platform backlog](docs/backlog/phase-7.md).

## Project documentation

Start here:

- [Vision](docs/vision.md)
- [Architecture](docs/architecture.md)
- [Roadmap](docs/roadmap.md) and [backlog](docs/backlog/)
- [Decisions](docs/adr/) and [open questions](docs/open-questions.md)

## Building

Development setup and machine roles: [docs/development-setup.md](docs/development-setup.md).

```sh
cargo build --workspace
just check
```

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md). Coding agents: read [AGENTS.md](AGENTS.md) first.

## License

Apache-2.0 ([ADR-0016](docs/adr/0016-license.md)); contributions use the DCO. See `LICENSE`.
