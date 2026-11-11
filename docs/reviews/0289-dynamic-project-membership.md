# Dynamic project membership (#289)

The hub counted only `CheckoutDiscovered` facts when deciding whether a project was shared.
ADR-0052 directory launches publish `WorkspaceRegistered` facts, so selecting matching project
folders after the daemons started left each node on its own broker. A peer saw no task and acquired
`README.md` while the first node still held a live lease on that project path.

`board::project_memberships` now enumerates borrowed `(node, project)` pairs from both fact types
and the original project of each still-running agent. The latter keeps a session's board access
when the owner re-associates its folder with another identity, and ends when that agent exits.
The running agent's qualified key supplies its authenticated producer; a payload `node_id` cannot
manufacture another node's membership.
`nodes_with` filters the requested identity to online, known nodes and deduplicates enrollment IDs.
The shared iterator retains offline facts so join discovery can still detect all members leaving;
it does not grant policy or act as an online-node list itself. No protocol fields or error codes change.

## Verification

Five public-API regression tests pass. The first four failed before the eligibility fix; the fifth
reproduced the re-registration gap in the first #291 revision before running-session retention:

- Two selected workspaces share a task and refuse a conflicting live path lease.
- Legacy checkouts and selected workspaces combine without counting a node twice; equal folder/
  machine names do not combine different project IDs or authorize another project's caller.
- Unknown/offline nodes and a spoofed `node_id` do not turn one node into a shared project. The
  projection binds the workspace to its actual event producer; one-node calls remain local.
- Re-registering a workspace under another project removes its old workspace membership.
- A still-running agent keeps its original board's tasks and lease protection after folder
  re-registration, can release its existing lease, and loses its session membership on exit.

The spoofing target is online without a legitimate project fact. Deliberately trusting the payload's
agent node ID or disabling workspace producer binding each makes the strengthened test fail, so
the assertions do not pass merely because an unknown node is filtered out.

Full Rust workspace tests, workspace Clippy with warnings denied, formatting, i18n and docs checks
pass locally. The manual release benchmark covers 100 nodes, 500 selected workspaces, 500 running
agents and 100 legacy checkout facts: scanning all 50 projects has **p95 448.054 µs** across 100
measured rounds (20 warmups).
This measures membership resolution, not full hub throughput or end-to-end latency.

```sh
cargo test -p agora-hub --test workspace_board
cargo test --release -p agora-hub --test workspace_board membership_projection_benchmark -- --ignored --nocapture
```

The real-binary browser/MCP regression uses a disposable hub, two independent node processes and
`agora-fake-agent` sessions. Both daemons start with an unrelated project. Two Git folders with the same explicit
`.agent-project.json` identity are selected later. After the fix, the peer lists the laptop's task
and its lease request fails with `lease.held`, naming `laptop/agent-1` and the original expiry.
The previous proof against #281/main had returned `[]` and granted both leases. No real coding-agent
accounts or installed services are used.
The same browser/MCP regression then changes the laptop folder's metadata and explicitly accepts
the new identity for another session. The original agent still sees its old task and can release
its old lease; the new session has its own project task list.

With the usual built binary/fake-agent/web client and Playwright/Chromium prerequisites:

```sh
TMPDIR=/tmp PW_DIR=/path/to/playwright node scripts/e2e-dynamic-projects.mjs
```

## Production integration with #281

After #281 merged, the prepared integration was folded into `Boards::membership` itself. Discovery
enumerates `project_memberships(view)` and revisits previously known projects when their last current
fact disappears. Changing both folders' identities and exiting their old agents therefore clears the
old membership; returning to that project asks every necessary current node to join.

The two integration regressions now live in
[join_membership.rs](../../crates/agora-hub/tests/join_membership.rs) and run in the normal Cargo suite.
They cover selected/session-only project discovery and a disappearing/reappearing project. The old
helper-only discovery failed the latter case. All seven membership regressions and the existing board
tests pass on the integrated source. The obsolete standalone patch and proof copies were removed.

This completes #289's eligibility and transition-discovery scope. Transfer storage restoration is
tracked separately in #297; node-side transfer bounds were handled in #293. This PR changes no
handover payloads, import limits or node transfer implementation.

The active local Claude session remains running. The installed preview stays at its previous build;
this fix is prepared for independent review and maintainer merge without restarting live services.
