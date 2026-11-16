# 0084. A confined agent gets its login from a secret the node holds
Status: Accepted
Date: 2026-11-16
Accepted: 2026-11-17 (the maintainer's own reply in the merger chat, "1 kabul et geç", to the merger's list that named ADR-0081, ADR-0082, ADR-0083 and ADR-0084 as the decisions waiting for them). The open questions' proposed answers stand.
Origin: issue #546, second half. On macOS a confined Claude Code says "Not logged in": its login is in the Keychain, and `~/Library/Keychains` is unreadable under the same rule that blocks everything below `/Users` (ADR-0047). The first half (git configuration) is ADR-0078.


## Context
- **Why the login is missing.** The Seatbelt profile denies reading file contents below `/Users`, `/Volumes` and the temp folders unless listed
  (`macos.rs`, `profile`). Claude Code on macOS keeps its subscription login in the login Keychain, which is a file below `~/Library/Keychains`
  read through the system's security service. Opening that folder to an agent hands it every password in the Keychain, so the sandbox does
  not do it, and should not. On Linux the same login is a file under `~/.claude`, which the owner already lists in `sandbox.write`
  (`docs/security/sandbox-recipes.md`), so the problem is mainly macOS (and Windows, once confinement is on).
- **What the agents read instead.** Checked against the installed versions on the machine this was written on (strings in the
  binaries; a variable named there is read by the program, but how and in which mode was not run):
  - Claude Code 2.1.325: `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `CLAUDE_CODE_OAUTH_TOKEN` (the token `claude setup-token` makes, a long-lived
    subscription token), the `apiKeyHelper` setting, and file-descriptor variants (`CLAUDE_CODE_API_KEY_FILE_DESCRIPTOR`).
  - Codex 0.169.0: `OPENAI_API_KEY`, `CODEX_ACCESS_TOKEN`, `CODEX_HOME`; `codex login --with-api-key` reads a key from stdin.
  Other kinds (opencode, `agy`, wrongstack) were not checked. Per ADR-0059's rule, a variable that was not checked on a real version is not
  declared in a manifest.
- **The node already passes its whole environment on.** `Supervisor::launch` (`supervisor.rs`) starts an agent from the node's own
  environment (it removes only Claude Code session markers). A token in the node's service environment therefore already reaches every
  agent of every kind, confined or not. That works as a workaround today, but it is by accident: a Codex agent gets the Claude token
  too.
- **What is true of any secret given to a confined agent.** `worktree` has no network rules, so the agent can send what it holds
  anywhere. The blast radius is that one token. An agent can print its environment into its terminal (clients show terminal bytes); on
  Linux a process of the same user can read another's `/proc/<pid>/environ`, and on macOS `ps eww` shows it for the same user: a
  confined agent of another kind could read it (not measured here; the implementing PR measures it).
- Invariants that bind this: secrets never leave the node; the hub stores references, never keys or tokens (AGENTS.md; ADR-0020);
  agents are data, not code (a manifest field, no agent-specific Rust).

## Options
1. **Open the Keychain to the agent.** Rejected: all passwords, and the file is read through a service the profile starts from
   `(allow default)`.
2. **The node reads the Keychain item and passes it.** Rejected: the node would copy a refresh-token document into an environment
   variable of a program that does not expect that shape, it triggers a Keychain prompt, and it makes the node a reader of another
   program's secret.
3. **A secret the owner gives the node, passed by name** (chosen). Works on every system and for every kind that reads an environment
   variable.
4. **Document `claude setup-token` / API key in the node's service environment.** The workaround that exists; keep it working but
   make it deliberate (below).

## Decision (proposed)
1. **A manifest field names what the kind reads**, additive, with `min_engine` raised (ADR-0059 style):
   `[accounts] credential_env = ["CLAUDE_CODE_OAUTH_TOKEN", "ANTHROPIC_API_KEY"]`. Only names checked on a real version are declared.
   The first listed name is the one `agora doctor` suggests.
2. **The owner stores the secret on the node, never in the hub.** `agora node secret set <NAME>` reads the value from a no-echo prompt or
   standard input (never an argument), and writes `<home>/secrets/<NAME>` with `write_private` (mode 0600; an owner-only ACL on Windows).
   `agora node secret list` shows names only; `agora node secret remove <NAME>`. Nothing about a secret is in an event, a log, the ledger, the
   audit, the hub or the web client except a boolean "set / not set" per name.
3. **Policy lists what each kind may receive**, in the node's own policy file (not over the hub): `credentials.<kind> = NAME, ...`. A name
   not in that kind's manifest `credential_env` is refused with a stable error at policy load (and named by `agora doctor`).
   The value comes from `<home>/secrets/<NAME>`, else from the node's own environment variable of that name.
4. **Passing.** The node adds the variable to that kind's agent environment at start (the same place the git variables of ADR-0078 are
   added), only for agents of that kind, only the listed names, never in argv, never written to a file in the worktree. A helper of
   another kind does not get it (ADR-0057). It applies to confined and unconfined agents alike.
5. **Scrub the rest.** Every name that any installed manifest lists in `credential_env` is removed from the environment the node
   inherited, for every agent, unless that kind's policy line lists it. So a token in the node's service environment no longer leaks to
   agents of other kinds. (An owner who relied on the accidental workaround adds one policy line.)
6. **The node does not read a Keychain, a credential store or `~/.claude`/`~/.codex` for the secret.**
7. **Code placement.** The secret store and the environment are in `agora-node`; nothing here is OS-specific except the owner-only
   file, which `write_private` already is. No agent kind appears in Rust.
8. **Text.** `agora doctor` says, per confined kind with no login the agent can use: "Claude Code cannot see your login in confined mode.
   Give it a token: run `claude setup-token`, then `agora node secret set CLAUDE_CODE_OAUTH_TOKEN`" (a Fluent message with the names as
   parameters; no text in Rust).

## What cannot be solved
- **The subscription login in the Keychain stays out of reach.** The agent gets a token the owner made for it, not the login: a
  `setup-token` needs a subscription and is long-lived; an API key bills per use, not against a subscription. Switching is the owner's choice.
- **A confined agent can exfiltrate the token it is given**, and may be visible to other same-user processes (above). The advice is a
  token made for this purpose, revocable on its own.
- **Interactive login inside a confined agent** cannot persist: the write to the Keychain is denied, which is the point.
- **Per-agent accounts** (two Claude logins on one node) are not solved by an environment variable per kind; `config_dir_env` (the
  existing `[accounts]` field) remains the route there.
- Passing the secret by an inherited file descriptor, so it is not in the environment, is possible for the variants that exist
  (`..._FILE_DESCRIPTOR`) but needs a spawn change in `agora-platform` and per-kind support. Later, separate.

## Not decided here
- Windows (Credential Manager): confinement is off by default (ADR-0050); the mechanism above is OS-neutral and applies when it is on.
- Whether the web client may start "store a secret on this machine" for the owner (it would pass through the hub: an end-to-end sealed
  value, as `docs/design/agent-accounts.md` describes). The first version is the node's CLI only, so the hub sees nothing.

## Consequences
- A confined Claude Code or Codex can work on macOS without opening the Keychain; the owner makes one extra step once per machine.
- New: one manifest field, one policy key family, one small store and three subcommands; a schema change is not needed.
- Behavior change: a credential variable in the node's environment no longer reaches agents of other kinds (decision 5).
- No hot-path impact: read at agent start.

## Open questions (for the maintainer)
1. Is a node-held secret store (`agora node secret`) the right place, rather than only the node's service environment? Proposed: yes; the environment stays as a fallback source.
2. Apply decision 5 (scrub other kinds' credential names) now? Proposed: yes, it removes an accidental leak.
3. First version CLI-only on the node, with no web client entry? Proposed: yes.
4. Which manifests get `credential_env` first? Proposed: Claude Code and Codex only, as checked above.
