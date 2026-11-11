# Changelog

Each release lists what a person using Agora can now do. The reasons are in `docs/adr/`; the state of every task is in `docs/backlog/`.

## Next

### Working folders
- Start agents in a chosen machine/project/folder; Git and separate working copies are optional. The CLI defaults to its current folder (`--cwd` to choose another, `--worktree` for isolation).
- Several agents can share a folder with separate sessions and integration files; shared changes appear once.
- Explicitly create `.agent-project.json` (`agora project init --cwd PATH`) and carry it across machines to group project copies. Source files are not synchronized.
- New directory launch needs an unrestricted owner device trusted on the node. Older clients keep their existing worktree launch behavior; fixed-file MCP manifests need explicit worktree mode.

### Windows
- Agent confinement for Windows is written (ADR-0050) and tested on real Windows in CI, but switched off until it is enabled after a check on a real machine.
- A project is found by its id before its name, and a name two projects share is refused.

## 0.2.1

### Security fix
- A team member could add a passkey and then sign in with it as the owner. Passkeys now belong to the person who added them. If you use team mode with 0.2.0, update, and review `agora hub audit export` for `sign_in` rows with `passkey` from people you did not expect. A hub with no team members was not affected.

## 0.2.0

### Teams
- A hub can have several people. `agora hub user add NAME --role owner|member|viewer` gives each a sign-in code and a role; a viewer can only
  read, a member works in the projects they were given (`agora hub user grant ID NODE/PROJECT`), the owner sees everything.
- A member never receives another project's snapshot, events or terminals, and the hub refuses their commands about anything else.
- A machine can limit what a person's device may sign (`device.<id>.projects = a, b` in its `policy` file); the hub cannot change it.
- An append-only audit trail of every routed command, sign-in and change to the team: read it in the app (Activity) or with
  `agora hub audit export`. The owner sees it; nobody can edit or delete a row.
- Upgrading a hub keeps its data: the database is migrated in place (tested from the 0.1.0 schema).

### Safer agents
- Agents can be confined (`sandbox = worktree` in a machine's `policy`): they write only in their worktree, its git folders and a temp
  folder, and read only the system, their program and what you list. Linux (Landlock) and macOS (Seatbelt); a machine that cannot
  confine refuses to start the agent rather than run it unconfined. `agora doctor` says whether a machine can.
- Agent packages can ask for extra folders; the machine's owner decides per kind (`sandbox.trust_manifest`).

### More places to use it
- Windows machines run agents in real terminals (ConPTY) with a process view.
- The web app installs to a phone's home screen, and a notification for a waiting approval links straight to it.
- A terminal client (`agora tui`): machines, agents, terminals, conversations, approvals, starting and stopping.
- A desktop app (Tauri shell, `apps/desktop`): it trusts only the hub's own CA, which you confirm once by its fingerprint
  (`agora hub fingerprint`). Built in CI; unsigned until it has signing accounts.
- The web layout mirrors for right-to-left languages (no right-to-left translation ships yet).

### Adapters and updates
- `agora adapter install|update|list|remove`: signed adapter packages from a registry, checked against a root key you pin.
- `agora update`: replaces the program from a signed release. It does not yet hand running agents to the new process.

### Fixes and tests
- Real-browser end-to-end check (sign-in, terminal, chat agent, approval) and visual regression baselines for the shared components.

### Known limits
- Windows agent confinement is not available (proposed in ADR-0050); a Windows machine with a sandbox policy refuses to start agents.
- `worktree-offline` (no network) is always refused: network rules are not built.
- Per-person limits on a machine bind what is signed; commands the owner leaves unsigned rely on the hub's checks.

## 0.1.0
First public release: nodes, hub, web app, mesh, approvals signed by the person, usage and cost, SSH install, backup and restore.
