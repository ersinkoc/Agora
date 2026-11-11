# 0047. Agent sandboxing with the operating system's own profiles
Status: Accepted
Date: 2026-11-10

## Context
Open question Q13 asks how agents are confined. Threat T8 (`docs/security/threat-model.md`) says an agent that escapes its
worktree is "not prevented in v1". Today an agent runs as the node's user: it can read `~/.ssh`, other projects and the node's own
files (device keys, trust file, usage files). Approvals (ADR-0020) gate what Agora mediates, not what the agent's own shell does.
Agents are data, not code (AGENTS.md), so any confinement must apply to a manifest-described process without agent-specific code.

## Decision
1. **Opt-in, per project and per agent kind:** a `sandbox` setting in the node's policy file with three levels: `off` (today's
   behaviour, still the default), `worktree` (the agent may write only inside its worktree, its own state folder and a temp folder;
   it may read the system and its worktree but not the user's home outside what the manifest lists; no access to `<AGORA_HOME>`),
   and `worktree-offline` (as `worktree`, and no network except what the manifest names, for agents that work through a local model).
   *Implementation note (macOS):* Seatbelt names only `localhost` or every host, so on macOS the level allows this machine and the
   node's unix sockets and nothing else; manifest-named hosts are not expressible there. Linux and Windows refuse the level until they
   have network rules.
2. **The operating system's own mechanism, no container runtime:** Linux Landlock (kernel 5.13 or newer, applied between fork and
   exec), macOS Seatbelt profiles, Windows a restricted token with the job object that already exists (ADR-0045). The code lives in
   `agora-platform/src/<os>.rs` behind one function (`confine(command, &Policy)`); no `cfg` elsewhere.
3. **Fail closed when asked, honest when unable:** if a level other than `off` is configured and the machine cannot enforce it,
   the agent does not start (`agent.sandbox_unavailable`, a new error code) rather than running unconfined. `agora doctor` shows what
   each machine can enforce, so the UI can say so before a start.
4. **Agents' own needs are declared:** a manifest may list extra read paths and network hosts the agent needs (its CLI's config
   folder, its API host); they are shown to the owner on first start and need a signed grant (ADR-0041) like other widening.
   *Implementation note:* the owner's consent is a line in the node's own `policy` file (`sandbox.trust_manifest = kind, ...`), which
   needs no network and cannot be forged by the hub; it is stricter than a signed grant (the person at the machine decides), and a
   grant-based path can be added later without changing the manifest field.
5. **Not a claim of safety:** the profile narrows what a compromised or confused agent can touch; it does not make untrusted code
   safe. The threat model keeps T8 as "reduced when enabled", and docs say what each OS does and does not stop.

## Consequences
- Needs a new error code, a policy field, platform code for three systems and tests that run real confined processes in CI
  (Linux Landlock needs a kernel that has it; the test skips and says so otherwise).
- A new dependency on the `landlock` crate or hand-written syscalls on Linux; decided in the implementing PR with its justification.
- Agents that expect the whole home folder (many do) need manifest additions; the default stays `off` to avoid silent breakage.
- Terminals are not confined by this ADR (they are the user's own shell); only agents are.

## Alternatives considered
- **Containers (Docker, Podman):** strongest isolation but needs a runtime on every machine, breaks agents that need host tools, and
  is absent on Windows and many SSH-only servers.
- **bubblewrap/firejail:** an outside program to install and trust; Landlock gives most of the file confinement from the kernel.
- **Do nothing and document:** the status quo; leaves T8 open for the users who run unattended agents.
