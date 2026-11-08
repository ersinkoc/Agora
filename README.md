# Agora

> Working name. Pre-alpha: the hub, node, mesh and web window run end to end, but there is no release yet and no
> agent manifest has been verified against a real agent (see the status below).

One window for every AI coding agent on every machine you own.

Agora runs a small **node** on each of your machines (Linux, macOS, Windows, local or reachable only over
SSH). All nodes report to a **hub**. You open one window and see every project, every agent (Claude Code,
Codex, WrongStack, Gemini CLI, Cursor, OpenCode, and any agent added later), their files and changes, and
the approvals waiting for you. Agents coordinate through a structured mesh: messages, a shared task
board, file leases, and handoff.

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
#    The node starts agents from manifests in ~/.agora/manifests (format: docs/specs/agent-manifest.md). Manifests for
#    real agents are not shipped yet; a plain shell shows the whole flow:
mkdir -p ~/.agora/manifests
printf 'kind = "shell"\ndisplay_name = "Shell"\nversion = "1"\nmin_engine = 1\n[process]\nlaunch = ["sh"]\n' > ~/.agora/manifests/shell.toml
target/release/agora node run --project ~/code/myproject
#    in the window, "Start an agent"; or in another terminal:
target/release/agora agent start shell --project myproject

# 4. Open https://localhost:7443/ , accept the hub's own certificate, sign in with the code from step 2
#    (`agora hub login` makes another). Your agents and their terminals appear in the window.
```

What works today: several machines in one window, live agent states, the real terminal of each agent through the hub,
approvals answered with a device key (the window's "This device" page shows the command that makes a machine trust it), the
mesh between agents (per-agent MCP endpoint), passkey sign-in, backups (`agora hub backup`), starting an agent from the window (machine and project, then an agent that machine has
installed). What does not yet: installing agents from the registry,
Windows terminals. Details and the open items per phase: [docs/backlog/](docs/backlog/).

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
