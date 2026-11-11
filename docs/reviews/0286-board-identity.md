# Distinct same-named project boards (#286 / LAN L15)

Two projects can have the same checkout basename while their logical identities differ. Previously
the board projection discarded that identity, and Console used the display name as each card's React
key. A selected-directory agent's leases were also missing because the projection only looked for
the legacy worktree-to-checkout association.

The projection now retains project IDs and indexes machine/folder context once. Console keys cards
by identity, exposes the full selectable ID, and offers expandable machine names, enrollment IDs and
paths. Checkouts of one identity stay together; equal display names stay separate. Selected Git
subdirectories deduplicate at the checkout root, non-Git workspaces retain their paths, and a known
checkout's name remains preferred over a temporary isolated working copy's name.

## Evidence

- Three initial regression cases fail on main: missing identity/context, missing selected-folder
  leases, and missing identity when checkout information has not arrived yet.
- All 86 web tests pass after the change, including five board tests covering the above cases,
  legacy behavior, temporary worktree naming and 500 selected-directory sessions across 50 projects.
- Type checking and the production client build pass. English and Turkish keys have parity.
- `scripts/e2e-board-identity.mjs` uses a disposable hub, two enrolled node processes with equal host
  names, two same-named Git projects with different explicit IDs and two copies of each project.
  Shells use real MCP endpoints to create tasks and acquire leases. No real coding agent or account
  is used, and the existing live hub is untouched.
- The older installed embedded client fails that same proof: both project tasks appear but their
  identity-labelled cards are absent. The updated client displays the right tasks, held paths and
  two machine/folder contexts on each board, preserves expanded details when another board updates,
  and restores both boards and their leases after snapshot reload.

Browser proof (with the usual built binary/fake-agent/client prerequisites):

```sh
TMPDIR=/tmp PW_DIR=/path/to/playwright \
  AGORA_SCREENSHOTS=docs/screenshots/board-identity \
  node scripts/e2e-board-identity.mjs
```

Set `AGORA_TEST_BINARY=/path/to/old/agora AGORA_TEST_EMBEDDED_WEB=1` to repeat the old-client baseline.

Board projection with 500 agents, 50 projects and 50 samples: **p95 1.04 ms** on Ubuntu. The fixture
warms the projection first and checks each result has 50 boards, two folder contexts and ten leases
per board. This measures pure projection, not full browser rendering or an end-to-end capacity run.

## Screenshots

| Language | Dark | Light |
| --- | --- | --- |
| English | [dark](../screenshots/board-identity/boards-en-dark.png) | [light](../screenshots/board-identity/boards-en-light.png) |
| Turkish | [dark](../screenshots/board-identity/boards-tr-dark.png) | [light](../screenshots/board-identity/boards-tr-light.png) |

[Turkish phone layout](../screenshots/board-identity/boards-tr-phone-light.png) also fits without
horizontal scrolling. The data and paths in these screenshots belong only to disposable fixtures.

No protocol, persisted model, backend lease semantics, dependencies or agent settings change. The
active local Claude session is preserved; this task does not restart the installed services.
