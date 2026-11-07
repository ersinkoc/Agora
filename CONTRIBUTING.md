# Contributing

This project is built by humans and coding agents together. The rules for agents in `AGENTS.md` apply to
human contributors too.

1. Pick an issue labeled `task` without a `claimed:*` label, and add your claim.
2. Read the documents listed in the issue's References.
3. Work in your own branch (and worktree if several tasks run in parallel).
4. Open a PR using the template. Keep the diff inside the crates the task names.
5. Sign off every commit (`git commit -s`): contributions are under Apache-2.0 with the DCO (ADR-0016).
6. Architecture-level changes start as an ADR with status `Proposed`.

Translations: add or edit files under `locales/<lang>/`. Keys come from `locales/en/`; never translate by
changing English keys.

New agent support: add a manifest under `manifests/agents/` (see `docs/specs/agent-manifest.md`) with
fixture screens, or build an adapter (see `docs/specs/adapter-protocol.md`).
