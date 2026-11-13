# Agora

> Working name. Pre-alpha: the hub, node, mesh and web window run end to end and signed binaries are published for
> Linux, macOS and Windows, but no agent manifest has been verified against a real agent (see the status below).

One window for every AI coding agent on every machine you own.

Agora runs a small **node** on each of your machines (Linux, macOS, Windows, local or reachable only over
SSH). All nodes report to a **hub**. You open one window and see every project, every agent (Claude Code,
Codex, WrongStack, Gemini CLI, Cursor, OpenCode, and any agent added later), their files and changes, and
the approvals waiting for you. Agents coordinate through a structured mesh: messages, a shared task
board, file leases, and handoff.

## Install a release

Each [release](https://github.com/ersinkoc/Agora/releases) has one binary per platform with a `.sha256` and a `.sig` file.
Releases are signed with the key in [`docs/security/release-key.pub`](docs/security/release-key.pub):

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

Agents start in the folder you select. In the web app, choose a machine, a project or another folder, then the agent.
Several agents can share that folder. Choose **Create an isolated working copy** (CLI: `--worktree`) when you want a separate Git worktree.
Folders without Git also work. Adding a folder does not require restarting the node.

To group copies of a project across machines, run `agora project init --cwd /path/to/myproject`, or choose **Create a shared project ID**
in the launch form. This explicitly adds `.agent-project.json`; commit or copy that file to the other machines. It groups the sessions;
it does not synchronize source files. Existing repositories without metadata keep automatic Git-based discovery. An independent fork can
remove its copied metadata and run `project init` to create a new ID; changing a previously used identity requires explicit confirmation
at launch (`--accept-project-id-change` in the CLI).

What works today (see [CHANGELOG.md](CHANGELOG.md) for what each release added): agents can be confined to their worktree on Linux and macOS (`sandbox = worktree` in the node's policy; Windows is written and tested in CI but switched off until checked on a real machine), a desktop app in `apps/desktop` pins the hub's certificate,  several machines in one window (the console: machines and agents on the left, several live terminals side by side in the middle, what needs you, uncommitted changes, agent messages and spend on the right), live agent states, the real terminal of each agent through the hub, workspace changes (shared folders have one shared diff), commits and files, a conversation view for agents without a terminal (adapters, and any agent that speaks ACP through `agora acp-bridge`),
approvals answered with a device key (the window's "This device" page shows the command that makes a machine trust it), the
mesh between agents (per-agent MCP endpoint), passkey sign-in, backups (`agora hub backup`), team members with roles and projects (`agora hub user add NAME --role member|viewer`, `agora hub user grant ID NODE/PROJECT`) and an audit trail (`agora hub audit export`), starting an agent from the window (machine and project, then an agent that machine has
installed). What does not yet: installing a node on a fresh machine over SSH works through `agora hub ssh install NAME` (ADR-0034) but is only tested against a stand-in `ssh`, and needs a signed release (a signing key from the maintainer); real Claude Code, Codex and WrongStack manifests (they need real accounts to capture fixtures), machines reachable only by SSH need `agora hub ssh add` (a tunnel the hub keeps open; try it on a real host first), installing agents from the registry works (`agora adapter install`, ADR-0044) but only against a stand-in registry, and the Windows node (ConPTY terminals, a named-pipe control channel, ADR-0045) is only exercised on CI with test agents, never yet on a real Windows machine with a real agent. Details and the open items per phase: [docs/backlog/](docs/backlog/).

## Status

Pre-alpha. Start here:

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
