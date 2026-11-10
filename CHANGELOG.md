# Changelog

Each release lists what a person using Agora can now do. The reasons are in `docs/adr/`; the state of every task is in `docs/backlog/`.

## 0.2.0 (not released yet; bump `version` in `Cargo.toml`, `apps/desktop/src-tauri` and `tauri.conf.json` when tagging)

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
