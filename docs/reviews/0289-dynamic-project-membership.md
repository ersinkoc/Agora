# Dynamic project membership (#289)

The hub counted only `CheckoutDiscovered` facts when deciding whether a project was shared.
ADR-0052 directory launches publish `WorkspaceRegistered` facts, so selecting matching project
folders after the daemons started left each node on its own broker. A peer saw no task and acquired
`README.md` while the first node still held a live lease on that project path.

`board::project_memberships` now enumerates borrowed `(node, project)` pairs from both fact types.
`nodes_with` filters the requested identity to online, known nodes and deduplicates enrollment IDs.
The shared iterator retains offline facts so join discovery can still detect all members leaving;
it does not grant policy or act as an online-node list itself. No protocol fields or error codes change.

## Verification

Four public-API regression tests fail before the fix and pass afterwards:

- Two selected workspaces share a task and refuse a conflicting live path lease.
- Legacy checkouts and selected workspaces combine without counting a node twice; equal folder/
  machine names do not combine different project IDs or authorize another project's caller.
- Unknown/offline nodes and a spoofed `node_id` do not turn one node into a shared project. The
  projection binds the workspace to its actual event producer; one-node calls remain local.
- Re-registering a workspace under another project removes its old workspace membership.

Full Rust workspace tests, workspace Clippy with warnings denied, formatting, i18n and docs checks
pass locally. The manual release benchmark covers 100 nodes, 500 selected workspaces and 100 legacy
checkout facts: scanning all 50 projects has **p95 190.391 µs** across 100 measured rounds (20 warmups).
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

With the usual built binary/fake-agent/web client and Playwright/Chromium prerequisites:

```sh
TMPDIR=/tmp PW_DIR=/path/to/playwright node scripts/e2e-dynamic-projects.mjs
```

## Integration with #281

This main-based fix covers existing eligibility. #281 adds transition discovery for moving earlier
local work; its project enumeration must also use `project_memberships(view)` rather than checkouts
alone. [The integration hunk](proofs/0289-board-join-membership.patch) applies to #281 at `be3438f`
after this helper is present; `git apply --check` was verified against that file. Ownership and the
required change are coordinated on #281. This patch does not edit its transfer/schema/node implementation.

Moving work made before the second project member joins remains #281/B6's responsibility, including
the separately reviewed persistence and transfer-limit defects. This change does not claim to fix
those transfer defects or the existing node-side ten-second negative-cache transition window.

The active local Claude session remains running. The installed preview stays at its previous build;
this fix is prepared for independent review and maintainer merge without restarting live services.
