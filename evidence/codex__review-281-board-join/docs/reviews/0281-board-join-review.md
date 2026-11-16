# Codex cross-review of #281 at cf5881d

Reviewed on Ubuntu against `cf5881d72767b1c50996b2ed801b5423106b6270`. Production files are unchanged.
The existing board tests pass (15), and the existing unfinished-work mesh test passes (1).
Three additional public-API proofs fail, and a disposable two-node MCP/browser run confirms the
selected-directory membership gap. These are separate local processes, not a new physical LAN run.
No real coding agents/accounts or live hub state were used.

## Blocking: a join can replace the persisted board after a hub restart

`Hub::board_joined` calls `Boards::joined` without loading `board.<project>` first. A newly opened
Hub has an empty in-memory `Boards`; `joined` creates a new board and the caller writes it over the
existing setting. The normal `Hub::board_call` path loads that setting, but the new join path skips it.

Public-API proof: persist an open task, reopen the hub, deliver a join with one local task before
the first board call, then list tasks. Only `imported-1` remains; `already persisted` is gone. This is
an introduced data-loss path in `crates/agora-hub/src/hub.rs:239`.

Load the saved board before importing the join answer, preserving both tasks and live leases. Keep
the established hub-lock ordering. Add a hub-restart regression at this API boundary.

## Blocking: extraction and import limits discard excess work

`Mesh::take_unfinished` removes every unfinished task and live lease from the local broker. The
node serializes them all, but `Boards::joined` silently imports only `MAX_JOIN_TASKS` /
`MAX_JOIN_LEASES` (500 each). Work past the limit has no remaining owner or destination.

Public-API proof: a join result containing 501 valid small tasks imports exactly 500. The transfer
returns normally and offers no continuation for the last task. The source removal is visible in
`broker.rs::take_unfinished`; the cutoffs are `board.rs:626` and `board.rs:654`.

Keep bounded import, but batch/acknowledge the transfer or preserve the excess on the source until
it can be imported. Do not solve this by accepting unbounded input. Both this and the persistence
finding were also reported by CodeRabbit; the tests here independently reproduce them.

## Existing companion defect: dynamically selected project membership (#289)

`nodes_with` reads only checkouts and the new `membership` project scan also ignores workspaces.
Directory launches publish `WorkspaceRegistered` / `AgentWorkspaceBound`, so matching project IDs
selected after node startup do not form a shared board. This already affects main and remains in
#281; it is not claimed as introduced by this PR.

Real-binary proof: start two nodes with an unrelated project, then select two Git folders carrying
the same explicit identity using `agent start shell --cwd`. The laptop creates a task and takes a
120-second lease on `README.md`. The peer sees `[]` and also acquires `README.md`; both grants end
one millisecond apart. The pure workspace-membership proof returns no nodes instead of alpha/beta.
The controlled run finishes while both leases are live. See #289 for acceptance and coordination.

## Reproduce

The Rust source is [proofs/0281-board-join.rs](proofs/0281-board-join.rs). This runner temporarily
installs it as a Cargo integration test, refuses to overwrite an existing file, and cleans up after
the expected three assertion failures (exit 101):

```sh
python3 - <<'PY'
from pathlib import Path
import subprocess
source = Path('docs/reviews/proofs/0281-board-join.rs').read_bytes()
target = Path('crates/agora-hub/tests/codex_review_281.rs')
with target.open('xb') as output:
    output.write(source)
try:
    raise SystemExit(subprocess.run(['cargo', 'test', '-p', 'agora-hub', '--test', 'codex_review_281', '--', '--nocapture']).returncode)
finally:
    target.unlink()
PY
```

The two-node proof is [proofs/0281-dynamic-projects.mjs](proofs/0281-dynamic-projects.mjs). With the
usual browser-harness prerequisites (built binary/fake-agent, built web client, Playwright/Chromium):

```sh
TMPDIR=/tmp PW_DIR=/path/to/playwright node docs/reviews/proofs/0281-dynamic-projects.mjs
```

`AGORA_TEST_BINARY` and `AGORA_TEST_WEB` may point at the binary and client build being reviewed.
The proof prints both successful lease grants and the empty peer task list, fails the shared-task
assertion, then removes its temporary folders and stops its test processes.

These are failure proofs for review, intentionally kept outside the normal test suite. They should
be promoted into passing regression tests along with the corresponding fixes. This review does not
claim that other reported findings are fixed or that the pending ADR amendment has been merged.
