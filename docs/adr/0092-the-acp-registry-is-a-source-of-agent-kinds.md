# 0092. The ACP registry is a source of agent kinds
Status: Accepted
Date: 2026-11-20
Accepted: 2026-11-20 (the maintainer's explicit message in the merger chat: "ADR-0092 ve ADR-0093'ü kabul ediyorum"). The open questions' proposed answers stand.

Builds on ADR-0004 (agents are data), ADR-0044 (the signed registry), ADR-0059 (skipping permissions), ADR-0069 (a missing kind says why),
ADR-0087 (install agents from the window) and ADR-0090 (a complete structured conversation). Touches the threat model.

Origin: the maintainer, 2026-11-20: "look at the ACP registry repo, we need to support these" (`github.com/agentclientprotocol/registry`).

## Context
- The Agent Client Protocol registry lists 47 agents (measured 2026-11-20 on a clone of `main`): Claude, Codex, Gemini, Goose, Cursor, Kiro,
  Junie, GitHub Copilot, opencode, Qwen Code, Mistral Vibe and more. Each entry is one `agent.json`: id, name, version, repository, license,
  and a `distribution` of one or more of `npx` (21 agents, plus 3 that also ship binaries), `binary` (21 plus those 3, per OS and
  architecture, with an archive URL, a SHA-256, a command and arguments) and `uvx` (2). The registry's CI starts every agent and checks
  that it answers `initialize` with valid `authMethods`.
- Agora has six manifests today (Claude Code, Codex, WrongStack, WrongStack structured, Antigravity, opencode). Every other ACP agent
  needs a hand-written manifest. ADR-0004 says an ACP agent is "a flag", so most of that work is data the registry already holds.
- The registry is a CDN file and a Git repository: **nobody signs it**. A node that ran what it says would run what whoever can write to
  that repository (or the CDN) says, with the user's files and login. That is the exact risk ADR-0087 and ADR-0044 exist to close.

## Decision
1. **A registry entry becomes an Agora manifest by a pure, deterministic conversion**, a script in `scripts/` (and the same code as a
   library function in `agora-adapters`, tested with fixtures). Input: one `agent.json`. Output: one L2 manifest
   (`[structured] acp = { argv = [...] }`) plus a package entry for ADR-0087. `npx` becomes `npx --yes PACKAGE@VERSION ARGS` (exact version,
   never a tag), `uvx` becomes `uvx PACKAGE==VERSION ARGS`, `binary` becomes ADR-0087's `binary-download` entry with the entry's URL
   per platform and SHA-256. An entry without a SHA-256 for its platform is converted as "not installable from the window"; it is still
   shown. The conversion refuses a package name or argument that is not a plain token (no shell syntax) and never copies `env` values.
2. **The nodes never read the registry.** The conversion runs in the repository, on the maintainer's machine or in a reviewed pull request
   that updates `manifests/agents/registry/` and the signed package list, and ships in a release (ADR-0044's signed index, ADR-0087's
   pinned installers). A node installs only what the signed list says, after the owner's signed approval (ADR-0020). A hub, a CDN or a
   registry compromise can therefore not install anything by itself; the worst it can do is propose a diff to review.
3. **The window can show what exists.** A "Add an agent" list shows every converted kind, installed or not (ADR-0069's wording for a missing
   program), with the registry's name, version, license and repository link. Nothing in the list is run by looking at it.
4. **Authentication stays the agent's.** The registry only lists agents that do Agent Auth or Terminal Auth. Agora's ACP client must
   handle both (an `authenticate` request for the agent's own flow; a terminal for Terminal Auth, opened as a normal terminal agent of the
   same machine, never sending the credential anywhere). Secrets stay on the node (invariant). Agents whose only method needs a secret
   Agora does not hold are shown as "needs sign-in on the machine".
5. **Permission skipping** is never taken from the registry. The ADR-0059/0090 classifier and the owner's choice decide; a converted
   manifest has no `skips_permissions` option unless a person adds it by hand and a test passes.
6. **Updates are a pull request.** An hourly or weekly job (a GitHub Action, no secret) runs the conversion on the newest registry and
   opens a pull request with the diff (new agents, version bumps, changed hashes). A changed archive hash for an unchanged version is
   flagged in the title. The maintainer merges; the next release carries it.

## Alternatives
- **Nodes fetch the registry themselves.** Easier, but unsigned third-party content would decide what runs on every machine. Rejected.
- **Only list, no install.** Safe, but the list then is a to-do list for the owner. It stays as the first rung; the install is ADR-0087.
- **Hand-write every manifest.** Does not scale to 47 agents and goes stale on every release.

## Build order
1. The conversion (library and script) with fixtures of real `agent.json` shapes (invented names, not copied vendor text), and a read-only
   "Add an agent" list in the window.
2. ACP `authenticate` handling (agent auth, terminal auth) and its states in the window.
3. The update job and its pull requests.
4. Installing from the list, once ADR-0087's installers exist.

## Open questions
- Does the maintainer want all 47 listed, or an allowlist (for example only agents that report `loadSession` and the permission options
  Agora's chat renders)? Proposed answer: list all, mark the ones that were never run through Agora "untested".
- Agent icons are remote SVGs. Proposed answer: fetched at conversion time, vendored after a size and content check, never fetched by a client.
- The registry is Apache-2.0 and each agent has its own license. Proposed answer: show the license text link, never bundle agent code.

## Consequences
New dependency: none (JSON parse already exists). Schema: none (manifests and package entries are data). Docs: `docs/specs/agent-manifest.md`,
the threat model row "converted registry entry". Tests: conversion fixtures for npx, uvx, binary, a bad token, a missing hash.
