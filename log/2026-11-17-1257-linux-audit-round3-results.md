# Linux audit to Linux merger: round 3 results (14 findings, all proven and patched, nothing committed)

Base 6418bf85. Patches are uncommitted in `../agora-worktrees/audit-<slug>`; ledgers and proofs in each `.temp_files/`.
No PRs yet: waiting for the maintainer's go. Pairing ones overlap your `pairnames` / `paircards` branches (different lines); fold them in if you prefer.

| ID | sev | what | worktree |
|---|---|---|---|
| F830+F840 | High | adapter/ACP starts skip both ADR-0082 (control token) and ADR-0084 (secrets) refusals: a confined adapter can read every stored secret | audit-node-sandbox-scrub (supersedes audit-node-credentials) |
| F841 | Med | agent private temp dir keyed by the folder's last name: `/a/web` and `/b/web` share one (macOS/Windows) | audit-node-sandbox-scrub |
| F870 | Med | resync announcement from a stale agent list rewrites the session ledger (finished agent auto-resumed; restarted one loses its pid) | audit-node-daemon-ledger |
| F820 | Med | join: the 10-min clock allowance is never applied; a machine 5-10 min behind is refused after approval | audit-node-pairing-client |
| F821 | Med | `agora pair open` spins a full core forever when the hub goes away | audit-node-pairing-client |
| F800 | Med | pairing "retry in N s" is one attempt too early (obeying client refused 20x) | audit-hub-pairing |
| F801 | Low | closed switch still counts reveal-first messages: 5 can lock an address out before the owner opens | audit-hub-pairing |
| F811 | Low | two approvals of one node id: windows say "approved" for a machine that gets no cert | audit-hub-pairing (+ spec §12 line) |
| F810 | Low | switch closing by itself (5 failures / timeout) doesn't wake the waiting machine | audit-hub-pairing-net |
| F850 | Low/Med | cert serial = hash of key: re-pairing the same key hits `UNIQUE devices.serial` | audit-hub-ca-enroll |
| F851 | Low | `agora doctor` silent on an unreadable cert file | audit-hub-ca-enroll |
| F860 | Low | fleet: same-hostname machines merge approval rows | audit-web-fleet |
| F861 | Low | fleet: groups reorder when an agent starts/stops working | audit-web-fleet |

Unproven follow-ups: hub half of F870 (stale state in hub event log); idle unauthenticated connection with no first-frame timeout (server.rs/web.rs); D-Bus/tmux fall back to default sockets after the ADR-0083 scrub; `agent_spawn::an_adapter_agent_is_in_the_mesh...` flaked 2/4 under load; `docs/specs/pairing.md` §13 should say the cert may start up to 10 min ahead (F820).
Env note: `launch_options::confined_only_needs_the_agents_confinement` fails on this host on unmodified main (control token reachable from long worktree paths / live node).
