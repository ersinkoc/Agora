# Agora

> Working name. Pre-alpha: design phase, no usable release yet.

One window for every AI coding agent on every machine you own.

Agora runs a small **node** on each of your machines (Linux, macOS, Windows, local or reachable only over
SSH). All nodes report to a **hub**. You open one window and see every project, every agent (Claude Code,
Codex, WrongStack, Gemini CLI, Cursor, OpenCode, and any agent added later), their files and changes, and
the approvals waiting for you. Agents coordinate through a structured mesh: messages, a shared task
board, file leases, and handoff.

## Status

Design phase. Start here:

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
