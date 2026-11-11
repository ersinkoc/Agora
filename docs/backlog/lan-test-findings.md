# Findings from the three-machine LAN test (2026-11-11)

A Linux hub with nodes on Linux, macOS and Windows 11, real shells, and real Claude Code and Codex on the Windows node. Messages in the `comms`
branch (`docs/fordevs/`). Each row is something the run showed that no CI run had.

| # | Finding | State |
| --- | --- | --- |
| L1 | A bundled macOS app without `NSLocalNetworkUsageDescription` (and with no stable code identity) cannot reach a hub on the LAN and macOS never asks | fixed in #250 (plist key and an ad-hoc identity) |
| L2 | The desktop shell saved a hub it could not verify and said nothing; it also sent the TLS name `localhost` for an IP | fixed in #242 |
| L3 | `agora tui` mapped an IP address to `localhost`, so it could not reach a hub by IP | fixed in #243 |
| L4 | Windows clients that use the system TLS stack (schannel: `curl.exe`, probably others) refuse the hub's certificate: `the revocation status is unknown` (it has no CRL or OCSP). `--ssl-no-revoke` passes | open: decide whether the hub certificate needs a revocation story, or document it; Edge and Firefox not yet checked |
| L5 | The Windows node id seen in the UI is the machine's host name, while mesh addresses use the node id from enrollment (`win/agent-1`, not `WHITE/agent-1`) | open: show the node id where an address is meant to be typed |
| L6 | A new manifest file is not seen by a running node until it restarts (`agent kinds`) | fixed: the node reads its manifest folder again before it lists or starts agents (a kind whose file was deleted stays until restart) |
| L7 | Claude Code's trust prompt is reported as "Needs you" on a wide terminal and as Idle after the terminal was narrowed to about 40 columns, while the screen still shows the prompt: the rule may depend on line wrapping | open: reproduce on a node; match across wrapped lines |
| L8 | Codex's folder-trust screen is rendered correctly but reported as Idle: the codex manifest has no rule for it | open: add a fixture and a rule |
| L9 | The macOS process readers (`process_name`, `process_cwd`, `process_tree`, `process_stats`) pass their tests but nothing outside `agora-platform` calls them, so no event or UI column shows them | open |
| L10 | Windows: a confined agent's `git commit` fails when the main checkout is under the user profile (git `stat`s every folder above the ref it locks); ADR-0050 work item | open, in the Windows confinement prototype |
| L11 | Windows: granting an agent a large `sandbox.read` folder (`~\.rustup`, about 100k files) takes 20 to 30 s per agent start because the permission is propagated to every file | open: documented in the sandbox recipes; consider non-propagating grants |
| L12 | Messages between nodes work in all six directions over the LAN hub (`send_message` to `<node>/<agent>`; inbox `provenance.node`) | verified; tasks, leases and handoff across machines not yet tried |
| B1 | Shared board (ADR-0039): when one of two nodes drops, the project is no longer "on two online nodes", so the remaining node falls back to its local broker and the hub's tasks vanish from its view; tasks made meanwhile stay local and are never merged | open: needs an amendment of ADR-0039's definition of shared (proposed separately) |
| B2 | Id collision: such a local task gets `t-1`, the same id as a different task on the hub board | open, goes away with B1's fix |
| B3 | A handoff to an offline node answers `mesh.recipient_unknown` (it ran locally) instead of `node.offline` | open, goes away with B1's fix |
| B4 | A node that crashes and restarts reuses its agent ids (`agent-1`), and the hub board still showed the dead agent's claim as the new agent's | fixed: the hub gives back the tasks and leases of a node's agents when the node announces a new process start |
| B5 | `lease.held` carries an empty `expires_at` | known and documented in the broker ("renders as later"); not changed |

