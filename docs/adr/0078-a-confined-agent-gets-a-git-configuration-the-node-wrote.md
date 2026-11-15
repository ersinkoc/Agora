# 0078. A confined agent gets a git configuration the node wrote
Status: Proposed
Date: 2026-11-15
Origin: issue #546, found while testing #477 on a real Mac. A confined agent (`sandbox = worktree`) could not run `git` at all: `~/.gitconfig` is
deliberately not readable by default (`docs/security/sandbox-recipes.md`), and git stops on "permission denied". The same rule holds on every system
that confines agents.

## Context
- A person's global git configuration holds two kinds of thing. Harmless: who they are (`user.name`, `user.email`), the default branch, line-ending
  and pull habits. Dangerous: `credential.*` helpers, `url.*.insteadOf` with a token inside, `include` / `includeIf` that point anywhere,
  `core.sshCommand`, `core.hooksPath`, `core.fsmonitor`, `core.pager`, `core.editor`, `alias.*` that start with `!`, `http.*` headers. Several of
  these make git run a program, and several carry a secret. That is why the sandbox does not let an agent read the file (ADR-0047).
- The node already keeps its own git calls away from the user's configuration (the node's git hardening in #300): it does not read hooks, filters or
  drivers from it.
- Three ways out were listed in #546: (a) the node writes a cleaned global configuration for the agent, (b) `~/.gitconfig` becomes readable by default
  and the credential risk is accepted, (c) `agora doctor` warns and the owner follows the recipe. (b) gives up the rule the sandbox exists for. (c)
  leaves every confined agent unable to commit until each owner edits a policy file.

## Decision (proposed): option (a)
1. **The node writes the agent's global git configuration.** Each time a confined agent starts, the node writes one small file for it and points git
   at it with the environment: `GIT_CONFIG_GLOBAL=<that file>` and `GIT_CONFIG_NOSYSTEM=1`. The repository's own `.git/config` still applies, as it does
   to any git.
2. **Only an allow-list of keys is copied, never the file.** The node reads each key from the person's real global configuration with its own hardened
   `git config --global --get`, and copies the value only for these keys: `user.name`, `user.email`, `init.defaultBranch`, `core.autocrlf`,
   `core.ignorecase`, `core.filemode`, `pull.rebase`, `push.default`, `push.autoSetupRemote`, `color.ui`. A key that is not set is left out. Everything
   else is dropped: no `credential.*`, no `include` / `includeIf`, no `url.*`, no `alias.*`, no `core.sshCommand`, `core.hooksPath`, `core.fsmonitor`,
   `core.pager`, `core.editor`, no `http.*`, no `gpg.*` or `commit.gpgsign` (the signing key is not reachable by a confined agent either, and a
   `gpgsign = true` with no key would make every commit fail).
3. **The file is the agent's to read, not to write.** It lives in the agent's own state folder (outside the node home, which stays closed to the
   agent, #477), and the sandbox gives the agent a read grant for it and no write grant. It is rewritten at every start, so a change the person makes to
   their real configuration reaches the next agent, and nothing an agent does to a copy outlives it.
4. **The owner can still choose otherwise.** If the owner's policy already grants `sandbox.read` for `~/.gitconfig`, the node does not set
   `GIT_CONFIG_GLOBAL`: the explicit choice stays as it is today.
5. **A missing identity says so.** With no `user.name` or `user.email` anywhere, a commit fails as it does for any git. `agora doctor` names this case on
   a machine that confines agents ("agents here have no git identity: set `user.name` and `user.email`").
6. **No new access.** The agent reads a file of ten non-secret keys. No credential, no helper, no program to run and no path outside the folders the
   sandbox already grants.

## Not decided here
- **The macOS Keychain login** ("Not logged in" in a confined Claude Code, the second half of #546). The Keychain is not a file the sandbox can
  safely open to an agent. It is a different problem with a different answer (an API key the node passes to the agent by a name the owner lists in
  the policy, never stored in the hub). It needs its own ADR.
- **Windows.** Confinement there is off by default (ADR-0050). The rule is the same when it is turned on.

## Consequences
- `git status`, `commit`, `branch` and `diff` work in a confined agent without the owner touching any policy.
- The allow-list is small on purpose. A person who wants another key reads the file from the recipe (decision 4).
- One more file written per agent start; a few `git config` reads, none on a hot path.

## Open questions (for the maintainer)
1. Is the key list in decision 2 right? Proposed: yes, as written; add a key only by a later ADR.
2. Should `core.sshCommand` ever be allowed, for a person who pushes over ssh with a special key? Proposed: no. A confined agent has no credentials to push with anyway (ADR-0047).
