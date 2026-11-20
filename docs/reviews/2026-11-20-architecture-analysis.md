# Architecture analysis

Date: 2026-11-20

Baseline: `origin/main` at `952a1a62` (the local checkout was 44 commits behind; this report was
refreshed against the current remote branch).

Scope: a static review of the repository architecture, implementation boundaries, operational risks
and improvement priorities. This is not a security audit, a performance certification, or a
build/test result: no build, test, benchmark or real-machine run was executed for this report.

The previous review in this folder (`2026-11-13-architecture-analysis.md`) identified ten priorities.
Several have since been closed: board mutations are transactional, subscription replay is bounded,
large diffs are windowed, the capability inventory exists, hub metrics exist, certificates renew
themselves (ADR-0085), and the terminal holder is landing (ADR-0079). This report reflects
`origin/main` and does not repeat findings that are already fixed.

## Executive summary

The architecture fits the product. A single hub owns coordination, while autonomous nodes own
execution. The dependency graph is clean, the protocol is schema-first and append-only, and agents
are integrated as data (manifests and adapters) rather than as code in the core. The strongest
engineering assets are the boundary discipline and the evidence culture: about 3,070 tests, golden
protocol fixtures, three-OS CI, a benchmark gate, real-machine run notes, and a capability table that
separates "decided", "built", "tested" and "seen on a real machine".

The remaining risks are mostly about assurance and lifecycle, not topology:

1. **The approval trust root lives in the origin the hub serves, and the default policy is open.**
   A non-extractable browser key prevents copying, not signing. A compromised hub, or a page on the
   same loopback origin, can invoke it. Separately, an empty `signed_commands` list leaves legacy
   routed commands (`agent.start`, `terminal.input`) available to a compromised hub. This is the
   strongest remaining threat to the "the hub cannot widen node policy" claim.
2. **The hub is a single availability and durability domain.** ADR-0086 (takeover) is accepted but
   not built. Startup replays the whole event log, and there is no event retention or projection
   checkpoint. Hub loss stops visibility and approvals even though agents keep running.
3. **The hub's write path is serialized.** `Hub::ingest` holds one global mutex across a synchronous
   SQLite transaction. The documented 5,000 events/s target has no ingest benchmark behind it.
4. **Cross-component verification is outside the loop.** Twenty-two browser e2e scripts exist, but
   CI runs none of them. The capability table has also started to lag the code: ADR-0079 is partly
   built but listed as "not built", and certificate renewal (ADR-0085) has no capability row.

None of these requires replacing the topology. They are bounded, testable improvements.

## 1. System shape

```
Web / TUI / Desktop / CLI
        |  client protocol: protobuf, WebSocket over TLS
        v
      HUB  - SQLite event log, projection, router, shared boards, approval relay, web assets
        |  node protocol: mutual TLS, node-initiated outbound connection
        v
   NODE(s) - PTY/ConPTY, supervisor, files/Git, confinement, per-agent MCP endpoint, disk event buffer
        |
   Agents (L0 generic / L1 manifest / L2 ACP / L3 adapter)
```

Four layers, three planes:

- **Runtime plane (node):** terminals, processes, files, Git, confinement. It keeps working without
  a hub.
- **Agent plane (L0-L3):** a new coding tool is a manifest, an ACP flag or an adapter process, not a
  core change.
- **Mesh plane:** messages, tasks, leases and handoff through an MCP endpoint injected into each
  agent.
- **Coordination plane (hub):** identity, event order, command routing, approvals, audit, web
  serving.

Authority is split consistently: the node is authoritative for runtime facts (process alive,
terminal bytes, file contents, Git state); the hub is authoritative for coordination facts (tasks,
leases granted, approvals, messages); clients derive views and own no facts.

## 2. Implementation scale

| Area | Size |
| --- | ---: |
| Rust source | ~136,300 lines |
| Rust including tests | ~181,200 lines |
| `agora-node` (source) | ~40,100 |
| `agora-hub` (source) | ~28,700 |
| `agora` CLI and front door (source) | ~17,500 |
| `agora-platform` (source) | ~16,600 (almost all OS code and `unsafe`) |
| `agora-tui` (source) | ~14,300 |
| `agora-adapters` (source) | ~8,000 |
| `agora-mesh` (source) | ~4,200 |
| `agora-protocol` (source) | ~2,100 |
| `agora-core` (source) | ~1,500, no dependencies |
| Web client (hand-written TS/TSX) | ~26,900 |
| Protocol | 2,175 lines of proto, 63 command variants, 41 event bodies |
| Tests | ~2,124 Rust + ~946 web = ~3,070 |
| ADRs | 93, all accepted |
| Locked crates | 259 |

The dependency direction is clean. `agora-core` has no package dependencies. `agora-platform` holds
the OS-specific code. `agora-hub` and `agora-node` do not depend on each other; `crates/agora` is the
only composition point. Production code outside `agora-platform` does not use `cfg(target_os)`.

One documentation nuance matters for new contributors: `architecture.md` presents `agora-core` as
the domain model, but the hub's real domain is the protocol event projection
(`crates/agora-hub/src/projection.rs`), and the node's runtime domain is the supervisor
(`crates/agora-node/src/supervisor.rs`, now 4,700 lines). `agora-core` is mainly the node's shared
vocabulary. That is workable, but it means agent/task/lease semantics exist in more than one
representation.

## 3. What is strong

1. **Boundary discipline is real.** Zero-dependency core, hub/node independence, platform isolation,
   and a single composition crate. The architecture document and the code agree on the important
   boundaries.
2. **Protocol engineering is mature.** Schema as source of truth, golden fixtures, a breaking-change
   check against the target branch, capability negotiation, `Unknown` fallbacks, and stable error
   codes. The append-only rule is enforced in CI.
3. **Node autonomy is implemented, not just claimed.** The supervisor knows nothing about the hub,
   events are buffered on disk, and agents do not stop when the hub is unavailable.
4. **Security design is serious.** Mutual TLS, approvals signed by a trusted device and verified on
   the node, exact-request signatures for folder actions, per-agent mesh sockets, secrets kept on
   nodes, and OS-level confinement. Certificate renewal (ADR-0085) now closes the former 90-day
   cliff: the node renews at a third of its life, verifies the answer against the pinned CA, and
   keeps a fresh key and request until the answer is stored; the hub replaces its own leaf before it
   ends.
5. **The runtime is getting more durable.** The terminal holder (ADR-0079) is landing: the Unix
   primitives, the private handover link and the Linux service unit exist, and an agent can outlive
   a node stop. Structured agents now have a complete conversation model (ADR-0090), plan limits
   (ADR-0091), and a path to surviving a node restart (ADR-0093).
6. **Performance is treated as a requirement.** Budgets are numbers, the benchmark harness gates
   throughput regressions, and measured findings have produced real fixes (Nagle, the hub's 20 ms
   wake-up, unnecessary screen copies).
7. **The evidence culture is honest in intent.** `docs/capabilities.md` separates the decision, the
   code, the automated test and the real-machine observation. That is rare and should be preserved;
   the current drift is a process problem, not a design one.

## 4. Risks and technical debt

| # | Severity | Finding | Impact |
| --- | --- | --- | --- |
| R1 | Critical | Approval key lives in the hub-served origin; default `signed_commands` is empty | A compromised hub can obtain signatures; by default it can also start agents and type into terminals |
| R2 | High | Single hub, no takeover yet (ADR-0086 accepted, not built), full-log replay, no retention/checkpoint | Hub loss stops visibility and approvals; startup and storage grow with history |
| R3 | High | `Hub::ingest` holds a global mutex across a synchronous SQLite transaction | Ingest throughput is unproven; a slow disk stalls every node and client |
| R4 | High | 22 browser e2e scripts are outside CI; real-machine evidence is sparse | UI and cross-component regressions can merge |
| R5 | Medium | The same state model is implemented in the hub projection, web `view.ts`, TUI and mesh broker/board | New events must be implemented in several reducers; semantic drift |
| R6 | Medium | Large modules and mixed concurrency models | Review cost and concurrency reasoning cost |
| R7 | Medium | Accepted decisions outrun delivery, and the capability table has drifted | Readers cannot tell what the current commit supports; half-built surface area |
| R8 | Medium | The L3 adapter promise has one minimal Python example; some ACP manifests are not validated against real programs | The extensibility thesis is unproven in the field |
| R9 | Medium | No structured logging or tracing; `eprintln!` plus metrics counters | Multi-hop failures are hard to localize |

### R1 - Approval trust root and the default policy

`apps/web/src/net/approval.ts` stores an ECDSA P-256 key pair in IndexedDB with
`extractable: false`. That prevents the private key from being exported. It does not prevent a script
on the same origin from calling `crypto.subtle.sign` with it. The web client is served by the hub, so
a hub that has been compromised (the T1 threat) can serve modified JavaScript on its own origin and
produce a valid approval signature for a pending approval id. A page that occupies the same loopback
port while the panel is down has the same origin storage.

This contradicts two statements in the current tree:

- `crates/agora-node/src/approvals.rs` says "a compromised hub can neither approve nor widen the
  policy";
- `docs/security/threat-model.md` (T27) says a port squatter "cannot approve anything".

The second claim is not true for a same-origin page: non-extractable keys are usable, only not
copyable.

There is a second, independent opening. `Policy` starts with an empty signed-command list; an empty
list is replaced by the team default only when at least one device is limited to projects
(`crates/agora-node/src/daemon.rs:1245`). In the default single-owner setup, a compromised hub can
send legacy `agent.start` and `terminal.input` commands without a user-signed grant. The threat model
states this plainly; the README repeats it. The architecture should either make the safe profile the
default or stop claiming that hub compromise cannot execute code.

The new structured-chat surface (ADR-0090) increases the relevance of this finding: the same origin
now renders more model and tool output, so an XSS bug there has a direct path to the signing key.

Recommended direction: bind approval signatures to a WebAuthn assertion with user verification
(challenge = the exact approval message), or move the signing surface to an origin the hub does not
serve (the desktop app or the TUI). At minimum, ship an explicit "shared deployment" policy profile
that signs `agent.start`, `terminal.input`, `files.write` and the other mutating commands, and correct
the two statements above.

### R2 - A single hub with unbounded history

ADR-0056 deliberately keeps one authoritative hub. ADR-0086 (takeover) is now accepted, but no
takeover or standby code exists yet. Today the hub's machine is a single point of failure for
visibility and approvals. Nodes keep executing, but an L2/L3 agent waiting for a person's answer
blocks while the hub is unreachable.

The hub rebuilds its view by replaying the entire event log at start. The measured cost is about
0.55 ms per 1,000 events: 18 ms at 21k events, 591 ms at 1M, and 1.1 s at 2.1M. The 500 ms cold-start
target is passed near 900k events. The usage-merge change removed 94.5% of the log volume on a real
trace, which buys years for a small fleet, but there is still no projection checkpoint, no event
retention and no archival policy. The privacy design names configurable retention as future work;
today the event log, audit table and board snapshots grow without bound.

Recommended direction: a versioned projection checkpoint plus replay of its tail, a retention policy
that distinguishes audit/cost evidence from replayable runtime history, and a restore drill. Keep
SQLite until measurements justify a backend change.

### R3 - The serialized ingest path

`Hub::ingest` takes the hub's single `Mutex`, calls `NodeSession::event` (a synchronous SQLite
transaction), applies the projection and fans out to subscribers while still holding it. A tokio
worker thread blocks on disk during that window. The design gives a clean exactly-once ordering
guarantee and simple reasoning, and the lock-wait and ingest histograms make the cost visible. What
is missing is evidence at the documented target: 5,000 events/s sustained, 100 nodes and 50 clients.
The benchmark suite covers terminal throughput, PTY/VT, twenty terminals and footprint, not hub
ingest.

Recommended direction: measure ingest with a representative event mix before changing anything. If
the result is short of the target, move writes to one ordered writer actor with batching, and keep
the append/projection/subscriber boundary atomic from the clients' point of view.

### R4 - Verification outside the loop

CI runs `cargo test` on Ubuntu, macOS and Windows, the web type check/build/tests, schema generation
and the breaking-change check, the i18n lint, the docs check, `cargo deny` and the benchmark gate.
That is a strong baseline. However, the twenty-two `scripts/e2e-*.mjs` browser scenarios (login,
terminal, approvals, reconnect, board identity, phone layout, shared size, link silence) are not in
CI, and many capability rows still say "none recorded here" for real machines. Windows confinement is
built but off by default. The OpenCode and Antigravity ACP manifests have not been run against the
real programs.

The result is a gap exactly where this product is hardest: cross-component behavior in a real
browser against a real hub and node. A small, stable e2e subset in CI would catch more than another
unit test.

### R5 - Several copies of the same state model

The hub projection (`crates/agora-hub/src/projection.rs`), the web client (`apps/web/src/net/view.ts`),
the TUI client, and the mesh broker/board (`crates/agora-mesh/src/broker.rs`,
`crates/agora-hub/src/board.rs`) each implement part of the same agent/task/lease semantics. The web
file even says it is "the client's copy of the hub's view". Local-first clients need a local
projection, so this is a deliberate trade-off, not a mistake. The cost is that every new event body
must be implemented in several reducers that can disagree without a compile error. The protocol grew
from 33 to 41 event bodies in the last 44 commits, which makes the cost concrete.

Recommended direction: add shared conformance vectors - one event sequence, the expected hub view,
the expected web view and the expected TUI view - and run them in all three places. That is cheaper
than unifying the runtimes and catches the drift that matters.

### R6 - Large modules and mixed concurrency

The largest production files are `crates/agora-node/src/supervisor.rs` (4.7k),
`crates/agora/src/tuicmd.rs` (4.3k), `crates/agora-tui/src/app.rs` (4.2k),
`crates/agora-platform/src/lib.rs` (3.0k), `crates/agora-node/src/daemon.rs` (3.0k),
`crates/agora/src/hubcli.rs` (2.7k), `crates/agora-node/src/approvals.rs` (2.6k),
`crates/agora-adapters/src/acp.rs` (2.6k), `crates/agora-mesh/src/broker.rs` (2.4k) and
`crates/agora-hub/src/server.rs` (2.4k). File length alone is not a defect. The concern is that
lifecycle, policy, transport and persistence concerns share the same modules, and that the node uses
std threads plus mutexes while the hub uses tokio plus a synchronous core. A change to approval
policy or restart behavior is hard to review without also reading transport code.

Recommended direction: split internal modules along state-machine boundaries (authorization,
lifecycle, routing, persistence, projection) without adding crates or changing the protocol.

### R7 - Accepted decisions, delivered slices and capability drift

`docs/capabilities.md` was updated for the ADR statuses of 0085-0087, but not for the features that
landed with them. It still says ADR-0079 (terminal holder) is "not built", although the Unix
primitives, the private handover link and the Linux service unit are in the tree. It has no row for
certificate renewal (ADR-0085), which is built. ADR-0086 (takeover) and ADR-0087 (install agents from
the window) are accepted but not built; pairing, the direct client-node path, team mode, workspace
cleanup, in-place self-update, ADR-0082, ADR-0083 and ADR-0084 remain incomplete. Six new ADRs
(0088-0093) landed in the same window.

This is normal for a pre-alpha, but the table is the project's answer to "what does this commit
support?" If it drifts, the answer becomes unreliable. It also makes half-built verticals expensive
to keep correct and easy to misrepresent.

Recommended direction: for every accepted ADR, either land a thin end-to-end slice with a named test
and a capability row in the same PR, or mark the feature explicitly experimental/off and keep it out
of the default path. Pairing is the best candidate to finish next because it removes the "paste a
string / type a command" setup step.

### R8 - The adapter ecosystem is still a promise

L0-L3 is the right extensibility model. In practice the tree ships L1 manifests for Claude Code,
Codex and WrongStack, L2 manifests for WrongStack, OpenCode and Antigravity, and exactly one L3
example (`adapters/examples/minimal-python`). The registry, signature checks and conformance
transcript exist. ADR-0092 (the ACP registry as a source of agent kinds) is a useful step, but there
is still no second real adapter or a public SDK, so the "a third party can add an agent without a
core change" claim has not been exercised outside the repository.

Recommended direction: build one first-party L3 adapter for a tool with a real protocol, publish the
minimal SDK, and run the conformance suite in CI. That turns an architectural claim into an
ecosystem artifact.

### R9 - Observability

The hub exposes Prometheus-format metrics behind `--metrics` (ingest and lock-wait histograms,
subscriber and relay counts, terminal bytes, resyncs, command queue depth). That is a good start.
The runtime still logs with `eprintln!`, has no log levels or structured fields, and no cross-hop
trace id, even though request ids exist. For a distributed control plane, "which hop failed" is a
first-class question.

Recommended direction: structured logs with redaction and a trace id propagated on commands and
events; spans for client -> hub -> node -> adapter; queue depth, oldest unacknowledged event, resync
count and approval latency as first-class operational signals.

## 5. Recommended sequence

| Horizon | Work | Completion evidence |
| --- | --- | --- |
| **0-30 days** | Fix the approval trust root or correct the threat model; ship a safe default signed-command profile | A test proves a hub with no device key cannot approve; default policy is documented and tested |
| | Bring `docs/capabilities.md` back in sync with ADR-0079, ADR-0085, ADR-0086 and ADR-0087 | Every accepted ADR has an accurate row; `check-docs.sh` stays green |
| | Add a small e2e subset to CI: sign-in, terminal, approval, reconnect, pairing | The jobs run on every PR and fail on a broken flow |
| **1-3 months** | Projection checkpoint, event retention/archival, restore drill | Checkpoint plus tail equals full replay; a corrupt checkpoint falls back; restore preserves board and runtime state |
| | Measure hub ingest; if needed, one ordered writer with batching | Published events/s at the documented target; input and approval latency stay within budget under flood |
| | Structured logging, trace ids, operational metrics | A failed multi-hop request can be localized from logs and metrics |
| | Projection conformance vectors across hub, web and TUI | One event sequence produces the same visible state in all three |
| **3-6 months** | Implement ADR-0086 takeover, or declare the hub critical infrastructure with a documented restore SLA | A hub loss is either survived or recovered within the published target |
| | Split supervisor/daemon/server internal modules; explicit state machines | Policy, lifecycle and transport changes can be reviewed and tested independently |
| | First real L3 adapter, SDK and CI conformance run; finish or park half-built features | A third-party adapter works without a core change; every default-path feature has an end-to-end test |

## 6. Verdict

| Dimension | Assessment (subjective) |
| --- | --- |
| Conceptual architecture | **9/10** - topology and authority split are right |
| Boundary discipline | **9/10** - dependency direction and platform isolation are exemplary |
| Security design | **7.5/10** - strong model, but the approval root and default policy are open |
| Operational maturity | **6/10** - certificate renewal is fixed; single hub, retention and observability remain |
| Scalability | **6/10** - replay, retention and ingest evidence are missing |
| Maintainability | **6.5/10** - large modules and duplicated state |
| Test and evidence | **8.5/10** - broad, but e2e is outside CI and the capability table has drifted |

The architecture is coherent and better than most pre-alpha systems. The main gap is not the design
of the topology; it is the assurance layer around it. The approval trust root and the open default
policy are the issues that most directly threaten the "install it and forget it" promise. The rest is
scale and maintenance debt that can be paid down in the order above.

## Validation boundary

This review inspected the component sources, the schema, the ADRs, the capability table, the
performance and threat-model documents, the CI workflows and the change set between `d45edbab` and
`952a1a62`. It did not build the workspace, run the Rust or web tests, execute the benchmark suite,
start a hub and node, open a browser, or exercise SSH, Tailscale, tunnels or real agents. Historical
measurements and real-machine results above are attributed to the repository documents, not
re-measured here. The report adds one new file and changes no code or existing document.