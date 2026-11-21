# 0086. Any machine can be the hub: one authoritative hub with takeover
Status: Accepted
Date: 2026-11-17

Accepted: 2026-11-19 (the maintainer's explicit message in the merger chat naming this ADR: "ADR-0086'yı kabul ediyorum"). The open questions' proposed answers stand.

Amends ADR-0056 decision 1 ("The hub stays single": the single authority stays, takeover is added). Touches ADR-0002, ADR-0065, ADR-0070,
ADR-0085 (Accepted), ADR-0020, ADR-0049, ADR-0054, ADR-0055, ADR-0075, ADR-0079 and the threat model.

Origin: the maintainer, in the Windows developer's session on 2026-11-17, relayed on branch `comms`
(`log/2026-11-17-0952-windows-maintainer-every-machine-a-hub.md`, replaced by `log/2026-11-17-1000-windows-maintainer-hub-decision-revised.md`).
The goal, in their words: "every machine can be both hub and node; whichever machine I open the GUI, web UI or TUI on, I must be able to run
AI agents on every node I can reach". Offered federation (a hub on every machine, connected), they did not choose it; after the trade-off was laid out
they chose **one hub plus takeover**. This ADR designs that choice.

## Context
- ADR-0056 already gives "any window on any machine reaches every node": `agora panel` (Amendment A) opens the hub's app on `127.0.0.1:<fixed port>`
  through the node's pinned-CA link. What is missing is the other half of the goal: the hub lives on one machine, and when that machine is off,
  asleep or gone, every window on every other machine shows nothing. Agents keep running (ADR-0002, ADR-0079) and events wait in the node buffers
  (ADR-0026), but nobody can see, approve or start anything.
- What the hub holds today (`crates/agora-hub`): `hub.db` (SQLite, `store.rs`): the append-only `events` log (`pos`, `producer`, `seq`), `producers`,
  `settings`, `enrollment_codes`, `devices` (certificate serials, revocation), `signin_codes`, `sessions`, `passkeys`, `users`, `grants`, `audit`
  (append-only by trigger, not a hash chain), `resyncs`, `user_devices`, `device_endorsements`, `device_revocations`, `metrics_tokens`; and beside it
  `ca.pem` and `ca.key` (`hubcli.rs`, `write_private`), the CA's private key in a 0600 file. The hub's server certificate is issued in memory at start.
  Projections (agents, terminals, boards, leases) are derived from the event log. A consistent online backup (`SqliteStore::backup_to`, `VACUUM INTO`),
  `verify` and `restore` exist and are drilled; a restore older than what nodes trimmed to is handled by ADR-0054/0055 resync.
- What a node holds: its own certificate and key, the hub address and pinned CA (`link/`, `HubLink` in `uplink_client.rs`), the trusted-devices file and
  remembered revocations (`device_trust.rs`), approval nonces and pending approvals (`grants.rs`, `approvals.rs`), and its unacknowledged event buffer
  (64 MiB by default). **The node, not the hub, verifies approvals** (ADR-0020): the hub relays.
- A node today dials one `wss://host:port` and checks the hub's server certificate against the one CA it pinned at enrolment. A browser's device key and
  sign-in belong to the page's origin (ADR-0048, ADR-0056 Amendment A); `agora panel` keeps that origin stable per machine (`127.0.0.1:<port>`).

## Decision
1. **Every machine has the hub role installed. Exactly one hub is authoritative at a time**, identified by a signed, numbered **hub record** (epoch).
   Nodes and clients follow only the highest epoch they have seen. There is no second master and no merge of two hubs' state (that is federation, rejected, see
   "Alternatives").
2. **The other hub-capable machines are standbys**: dormant, serving nothing, receiving the hub's state continuously.
3. **A takeover is a signed approval by the owner** (ADR-0020), verified by nodes and standbys, which bumps the epoch. Manual first; automatic later, only on
   three or more hub-capable machines and only under a pre-signed owner grant (sections 4, 8).
4. **No CA private key is copied to any other machine.** A takeover rotates the CA: the new hub makes its own, and an owner-signed record moves every node's
   pin to it (section 3). The cost is real and stated there.
5. **Nodes and clients find the current hub from the record**, with LAN discovery as the fallback (section 5). Nobody is enrolled again.
6. **Agents never depend on any of it.** Nothing in this ADR is on an agent's execution path (ADR-0002); a takeover, a partition or a failed takeover stops
   no agent and no terminal, and no node waits for a hub.

## 1. What "the hub role installed everywhere" means
Option A (recommended): **a dormant hub service per machine.** `agora service install` (the front door, `servicecmd.rs`) registers a second supervised unit,
`agora-hub`, beside `agora-node`, started in `standby` mode. In standby it:
- listens on **no client port and no node port**; the only listener is a **loopback** takeover endpoint used by this machine's own `agora panel`;
- runs a replication client (section 2) toward the authoritative hub's address, over mTLS with a certificate of a new usage (`hub-standby`), issued by the hub
  when the owner adds the standby ("Allow this machine to take over", approved from an owner window, audited, ADR-0065 style);
- stores: its own `standby.db` (the replicated tables of section 2, never opened by a serving hub until promoted), the latest hub record, its **own** server key
  and certificate request material (generated locally, never shipped), and **nothing secret of the hub** (no CA key, no passkey private data: the hub holds
  only public keys; sessions are token hashes).
- Option B: install the role only on request (`agora hub standby enable`), not by default. Fewer idle processes, but "every machine can be a hub" is then a
  step the owner forgets before the hub dies. Recommended compromise: installed by `agora service install` on machines that are not the hub **only after the owner
  said "this machine may take over"** (it is a security grant, section 3, so it must not be silent), dormant otherwise.
- Option C: promote by starting the hub on demand from the node (no standby process). Rejected: no continuous state (RPO becomes the last backup).
Cost: one more supervised process per machine, a replication connection, a state copy that grows with the event log (the log is already bounded by the same
retention as the hub's own; to be measured in step 4), and the per-OS items in section 8.

## 2. State reaching the standby
**What must be copied** (everything in `hub.db`, none of it secret except where noted): the event log; `producers`; `settings`; `devices` (serials, revocations);
`users`, `grants`, `user_devices`, `device_endorsements`, `device_revocations`; `passkeys` (public keys and counters); `sessions` (hashes: a session survives a
takeover; shorter would sign every browser out, which is acceptable but not required); `audit`; `resyncs`; `enrollment_codes`, `signin_codes` (short lived: not
copied; pending ones die with the takeover); `metrics_tokens` (hashes). **Not copied: `ca.key`** (section 3), the old hub's server key, terminal bytes (never in the
store, ADR-0022).

**The rule: hub state is never copied as files, and never as the folder.** The hub directory holds `hub.db`, `hub.db-wal` and `hub.db-shm` (WAL mode) next to `ca.key`,
`ca.pem`, `server-cert.pem`, `token` and `listen`. A plain copy of `hub.db` while the hub runs is not a backup: the Windows developer measured a copy with `user_version 0`
(the schema was still in the WAL; under write load the gap is larger). Everything that reaches a standby, an export file or any other machine is made only by `VACUUM INTO`
or the online backup API (`SqliteStore::backup_to`), run from a second process while the hub runs (measured on Windows 11: `integrity_check` ok, `user_version` 12, 0.04 s).
**`ca.key` and `token` are excluded by construction, not by a filter on a copied folder:** the export is assembled from named tables (the list above) into a new database
file, so a file that is not in the list cannot enter it; there is no code path that reads the hub directory's other files for export, replication or the table feed; and a
test lists every file and table an export, an import and a feed message can carry and fails if `ca.key`, `token`, `server-cert.pem` or any private key appears (and
if an export from a hub directory seeded with a marker `ca.key` contains the marker). Where a standby stores the received state, `standby.db` is written through `write_private`
(owner-only mode on Unix, owner-only ACL on Windows).

Option A (recommended): **log shipping of the event log plus a small-table feed.**
- Events: the standby asks "everything after `pos` N" (the same resume the browser and the nodes use, `hub.rs` replay); the hub streams new events as stored.
- The other tables are small and mutable, so they are not log-shipped (that would need a change journal in every handler). A **table feed** ships a consistent
  snapshot of them (`VACUUM INTO` of a copy limited to those tables, the existing backup path) every 30 s and **immediately after any security-relevant change**
  (revocation, endorsement, enrolment, user or role change, device registration).
- **Semi-synchronous for what must not be lost:** while at least one standby is connected, the hub reports a revocation, a device removal or an approval decision
  to the owner as done only after one standby confirmed it (timeout 2 s, then it completes alone and records "not replicated" in the audit). Without a standby
  nothing changes from today.
- **Node acknowledgements follow replication.** While a standby is attached the hub acknowledges node events up to `min(committed, replicated)` (ADR-0026 acks
  let the node trim its buffer). So an event a node produced is always either on the standby or still in that node's buffer: **node-produced events lose nothing**
  in a takeover (cost: node buffers hold a little more, bounded by the replication lag; a stalled standby is detached after 60 s so it can never hold a node's
  buffer hostage, and the hub then says "no standby" in the window).
Option B: periodic snapshot only (every N minutes) plus the nodes' buffers. Simple, reuses `backup_to` only, but any hub-produced fact between snapshots and any
event a node already trimmed is lost; a revocation could come back. Acceptable as **step 1** (export/import), not as the end state.
Option C: SQLite-level replication of the whole file (WAL shipping). Rejected: ties the standby to the exact file format and page order, makes a schema migration a
fleet event, and ships the secrets-free and secret tables alike.

**What can be rebuilt from the nodes after a takeover** (ADR-0054/0055 resync facts): which machines exist and are online (the hub owns `NodeLink`, ADR-0070, rewritten
as nodes connect), every running agent and terminal and its state, pending approvals (node-held, section 6), each node's folders, the trusted devices and their
revocations (node-held, ADR-0061/0081), usage counters from the node's own files. **What is truly lost** with a standby that lagged: hub-produced events newer than
the standby's last pos (client-made board edits, task changes, chat messages the hub stored, approvals' audit rows, `NodeLink` history), within the RPO below.
Board and lease state are projections of hub-stored events (ADR-0039/0040): within the window they roll back, and ADR-0055 resync re-claims leases that running
agents hold; leases and tasks that existed only in the lost window are gone and are shown to the owner as such. This is to be proven by the drill of step 4, not
assumed.

**Numbers (to be validated by step 4's measurements, then fixed):**
- **RPO:** node-produced events 0; revocations, device removals and approval decisions 0 while a standby is attached; other hub-produced events at most **5 s**
  of log (a stream) and the small tables at most **30 s** (the feed). Without any standby: the last export, which is the owner's choice.
- **RTO (manual):** the owner's decision plus about **30 s**: the standby opens `standby.db` as `hub.db`, issues its CA and server certificate, publishes the record;
  nodes follow within one redial (their backoff is bounded, ADR-0070 silence deadline 25 s) so the window shows every machine within about **60 s** of the
  owner's click, minus machines that are themselves off. A node that was off follows when it returns, from its stored record.
- **RTO (automatic, section 4):** detection 90 s plus the same 60 s: about **2.5 minutes**. Shorter is possible and flaps on Wi-Fi and laptop sleep; not
  recommended.

## 3. The CA key
The hub's CA key is the trust root of every machine: whoever has it can issue a client certificate the hub accepts. A standby that can issue certificates
needs either that key or its own. Options, with what each does to the threat model:

| Option | Key on how many machines | What it costs |
| --- | --- | --- |
| A. Copy the key to each standby, sealed by the OS (DPAPI, Keychain, 0600 file) | all hub-capable machines, always online-readable | Simplest, nothing changes for nodes. Every machine that can become hub is a root-key theft target at all times; a compromised laptop mints enrolments for the whole fleet. Widens T3 and the "hub breach" row for every standby. |
| B. Copy it encrypted to a key the owner holds or derives (a passphrase, Argon2id/scrypt) | all, as ciphertext | The standby cannot use it until the owner types the passphrase at promotion: theft of the machine yields ciphertext. Needs a KDF dependency (a decision of its own, AGENTS.md) and a passphrase the owner must not lose; the owner's signed device key cannot decrypt it (a browser's non-extractable key signs but does not decrypt). Automatic takeover is impossible (a human types the passphrase). |
| C. Split it (2-of-3 threshold shares across the three machines) | no machine alone | No single machine holds the key at rest, but the live hub has it whole in memory; two machines together restore it. Real implementation cost (own GF(256) code or a dependency), shares must be refreshed when machines change, and a takeover needs two machines awake. Good fit for exactly three machines, awkward for 2 or 5. |
| D. A takeover CA signed by an offline root | root on none (owner keeps it offline), intermediates on hubs | Strongest chain, but nodes pin today's self-signed hub CA, so the first migration needs the same owner-signed re-pin as option E, plus the owner keeps and uses an offline root: an operational burden for a one-person setup. |
| E. **A new CA on every takeover, moved by an owner-signed record (recommended)** | exactly one (the serving hub's); never copied | The key never leaves the machine that made it. Cost: every takeover rotates the CA and certificates (below), and an automatic takeover needs a pre-signed grant. |

**Recommended: E.** How it works, with the pieces that already exist:
- Nodes already trust **the owner's devices** (`device_trust.rs`), independently of the hub, and verify their signatures. The hub record (section 4) is signed
  by such a device and carries the new CA certificate (public) and its fingerprint. A node verifies the signature against its trusted devices, then **adds** that CA to
  its pins (pinned set, not a single CA; `HubLink.ca_pem` already holds PEM text and the connector loads every certificate in it) and starts trusting the new hub's server certificate.
- A node must then hold a client certificate the new hub accepts. The new hub accepts, for a bounded transition (90 days, the certificate lifetime), client
  certificates of the **previous** CA **only if their serial is in the replicated `devices` table and not revoked**: the old CA is trusted as a list of known serials, not
  as an authority, so a stolen old CA key cannot mint new access. Each node, on its first connection to the new hub, renews through ADR-0085's `RenewRequest` over that
  authenticated link and receives a certificate of the new CA. ADR-0085 therefore becomes a dependency of this one (sequence in section 8).
- A node that is off during the takeover keeps working with its old certificate (still on the serial list) and follows when it returns; one that is off past the
  transition joins again with the owner's approval (ADR-0085 point 4).
- Browsers and the desktop app never see the CA (the loopback forwarder checks it), so they are unaffected.
- **Threat-model consequence:** the "secrets never leave the node" invariant is about agent and API secrets and is untouched; the hub's own CA key is the hub's business.
  Under E that key is on **one** machine at a time, not on every standby, which is the main reason to choose it. What E trades: (a) takeover needs an owner-signed record
  (also required by section 4 for fencing, so no extra step), (b) the hub record's signing device becomes the root of "who is the hub", so loss of **all** owner devices
  during a hub loss means re-enrolling (as today when the owner loses everything), and (c) a compromised standby holds the **replicated tables** (users, device public keys,
  audit, board data: metadata, no secrets) but not the key.
- If the maintainer prefers a simpler first release, **A is the fallback with an explicit, documented widening** of T3, not the recommendation.

## 4. How a takeover is decided; split-brain prevention
**The hub record.** A new, append-only message (schema change, so this ADR must be accepted first), `HubRecord`:
`{ hub_group_id, epoch, hub_name, addresses [ordered], ca_cert_pem, previous_epoch, previous_record_hash, issued_at_ms, signer_device_id, signature }`.
- `hub_group_id`: a random id created at first `hub run`; a record with another group id is from a different installation and is never followed.
- Signed by an **owner device key** (ADR-0020's device signing), over a fixed message built like `endorse_message` in `devices.rs`. Nodes, standbys and clients verify it
  against trusted devices (node-side `DeviceTrust`; hub-side the replicated `user_devices`). The hub itself cannot make a record.
- **What stops an attacker who takes a machine from claiming a higher epoch:** a machine alone cannot sign; only a trusted owner device can. An attacker with a standby
  machine can run its dormant hub and serve it, but no node follows it (no valid record), which is the same position as an attacker who stands up a rogue hub today.
  An attacker who has an owner device key can already approve anything (ADR-0020); a takeover is one more thing that key can do and is audited as such.
- Rules (nodes and clients): follow the **highest valid epoch seen**; ignore lower; two different valid records at the same epoch are **both refused** and shown to the
  owner as a conflict (the only tie-breaker is the owner); a record whose `previous_record_hash` does not chain to what the node saw is accepted only if its epoch is
  higher and it is owner-signed (a node that missed an epoch is not stuck). Nodes store the record next to the link state and keep it across restarts.

**Manual takeover first** (step 3): "Make this machine the hub", on the machine's own panel (the loopback takeover endpoint, section 1) or `agora hub takeover`.
The owner signs on a device (a browser or the desktop app via the panel, as for approvals). The standby verifies the signature against the owner devices it replicated
and, as a second check, this machine's own node trusted-devices file; it then opens its replica as `hub.db`, makes a CA (section 3), issues its server certificate for the
addresses in the record, bumps the epoch, signs-in the record and starts serving. **Who may promote:** an owner, from an owner device already trusted on that machine's node;
members and viewers never (ADR-0049). Every promotion is an audit row on the new hub and a notice in the window.

**Fencing the old hub.** Every node and client connection states the highest epoch it has seen (new field in the handshake messages, additive). A hub that is told
of a higher epoch than its own stops serving, marks itself superseded, drops to standby and replicates from the new hub; its divergent log suffix (events after the fork)
is **kept aside**, not merged (below). A node refuses a hub whose record has a lower epoch than its own.

**Partitions: what is possible.**
- *Two machines.* A standby cannot tell a dead hub from a cut link. No automatic takeover is safe; only the owner, by signing, breaks the tie. The owner is the quorum.
  Manual takeover in a partition is allowed and is exactly the case the fencing exists for. State plainly: until the old hub learns of the new epoch (any node or client that
  has seen both tells it), both can serve; nodes that have not seen the new record keep following the old hub. This is a bounded window the owner causes on purpose.
- *Three or more hub-capable machines (the maintainer has three).* A **hub lease**: the serving hub keeps serving only while it has heard from a majority of the
  hub-capable machines (itself included) within 60 s, measured on its own monotonic clock (not wall time; after the process is suspended and resumes, as ADR-0070 already
  detects, it re-checks before serving). A standby may propose automatic takeover only after **90 s** of silence from the hub *and* agreement of a majority of the others
  that the hub is unreachable *and* a **pre-signed grant** by the owner for this exact promotion (the owner signs once, e.g. every 30 days, "machine S may take over from
  hub H at epoch n+1 when a majority agrees"; the grant expires, is revocable, and is checked like any signature). With the 60 s lease shorter than the 90 s wait, the old
  hub in the minority has stepped down before the new one starts. This depends on clock rates, not clock agreement; a VM paused for longer than the margin is the known
  hole, and fencing by epoch is the second line. **Not recommended before the manual path has run in real use** (open question 1).
- Order of standbys (which one proposes first): by the owner's list in the record; a tie-break by the longest replicated pos, then the machine id.

**What the old hub did during the partition.** It may have accepted approvals, leases, board edits and chat. After demotion:
- **Approvals** already verified and acted on by a node are facts at that node (the node verified the signature; ADR-0020) and stay in that node's own log; the old hub's
  suffix keeps their audit rows in a read-only "superseded hub" section of the audit view. Approvals still pending or not yet acted on are **void**: the node, not the old
  hub, holds the request and the new hub gets it again (section 6).
- **Leases and board changes** made on the old hub after the fork are **void unless re-confirmed**: the new hub does not apply that suffix; the owner sees a list ("3 board
  changes and 2 leases were made on Linux after Mac became the hub") with "apply again" per item, which becomes an ordinary action on the new hub; leases are re-claimed by the
  agents that actually hold them through ADR-0055 resync, which is the fact source.
- The old suffix is kept for the owner as a file (`<hub dir>/superseded-<epoch>.db`) and removed only by the owner.

## 5. How nodes and clients find the current hub
- **Nodes.** The record holds an ordered list of addresses (a hub's LAN address(es), and a stable name if the owner has one: open question 3). A node tries the address of
  its stored record; on failure it tries the others; then LAN discovery (ADR-0065 `AGORA-DISCO1`: a **serving** hub answers, an owner-approved standby answers with
  "standby, not serving" so a node can tell the difference; a quiet hub is still invisible unless open for pairing, so for already-enrolled machines this adds a signed
  **"announce" reply that carries the record** instead of requiring pairing mode); every candidate must present a record with epoch at or above the node's and a
  valid signature, otherwise it is ignored. Redial uses the existing backoff (ADR-0070).
- **Server name.** A node checks the hub's certificate name against the address it dialed; the new hub's server certificate is issued with the record's addresses
  (`issue_server`, `ca.rs`), so a changed machine means a new certificate by the new CA, not a pin change by hand.
- **`agora panel`, the TUI, the desktop app.** They run on machines that have a node (or the standby), hence a stored record: they ask "who is the hub" the way a node
  does and point the loopback forwarder (`panelcmd.rs`, same fixed port) at the current one; when the epoch changes the forwarder reconnects, the page's WebSocket
  reconnects (existing "Retry now" and ADR-0070 silence deadlines). The browser's origin is `127.0.0.1:<port>` **on that machine, whichever hub is behind it**.
- **Honest answer on "sign in on any machine":** through `agora panel` the origin does not change across hubs, so the browser's device key stays valid: the key's
  public part is in the replicated `user_devices`, the session is a replicated token hash, so a takeover does not ask for a new sign-in. **A browser pointed directly at a hub's
  own address** (a machine without a node, or a bookmark) is a different origin per address: a new device and a new sign-in approved from a trusted device (ADR-0065), once per
  hub address. Passkeys bind to the relying-party id (`passkey.rs` `rp_id`, the host): they are per host name, so a passkey registered through one address does not exist on
  another; with a stable name (open question 3) they would survive, without it they do not. This is a limit, not hidden.

## 6. What a takeover does to what is running
- **In-flight approvals.** A pending request is held by the **node** (`approvals.rs`, minted id) and was only announced to the hub. After a takeover the node's resync
  announcement (ADR-0055) names its pending approvals; the new hub shows them as new cards. A card the owner signed but the old hub never relayed is simply not executed
  (the node saw no signature) and is asked again. **Replay protection is node-side** (the nonce and expiry checks in `grants.rs`, persisted per node), so a signature
  captured on the old hub cannot be replayed to a node via the new hub. Hub-held pieces (sign-in codes, pairing sessions, passkey challenges) are not replicated and
  are void; people retry. (To be verified in step 3: that the node re-announces pending approvals today; if not, step 3 adds it.)
- **Running agents.** Nothing. They never see the hub (ADR-0002, ADR-0079, ADR-0075).
- **Boards and leases.** From replicated hub events up to the RPO, plus ADR-0055 resync as the fact source: an agent that still holds a file keeps its lease; one
  that existed only in the lost window is shown as lost (section 2).
- **Terminal streams.** Clients resubscribe (the node keeps the terminal; the new hub fans out from the node's next frame); a hub never stored bytes.
- **Audit continuity.** The audit table is append-only by trigger but **not** a hash chain today; this ADR does not claim a chain exists. The takeover writes an audit row
  "hub takeover" (epoch, who signed, from which machine), and the superseded suffix is kept apart. If the maintainer wants proof of continuity, the hub's audit gains a
  hash chain (each row carrying the hash of the previous) as its own change before step 4; with it, a takeover row links the chain across the epoch. Without it, continuity is
  "append-only per hub, plus the takeover row".
- **Machines' "online" (ADR-0070).** The new hub writes `NodeLink` as nodes connect; machines it has not heard from show offline with "last seen" from the replica.

## Alternatives
- **Federation (a hub on every machine, connected).** The maintainer offered and did not choose it. It needs boards, leases, approvals and audit replicated and reconciled
  between live hubs (conflict rules for every one), the "hub cannot widen node policy" model redone (which hub's signature does a node accept for which scope), a new
  presence model (ADR-0070 assumes one owner of "online"), and an ADR-0002 rewrite: months of work and the largest risk surface. It buys what this does not: machines that are
  often disconnected from each other each keep a working hub, and **separate trust domains** (a team where nobody owns every machine). The maintainer's setup (three
  machines, one person, one LAN) needs neither.
- **A standby with no takeover (backup and restore only).** What exists in pieces today (`agora hub backup/restore`). Cheap and safe, but a human restores after the
  fact and the window is dark until then. It stays as step 1 of the plan and as the recovery when no standby exists.
- **What this choice does not give:** two machines usable as hubs at once; hub-level work while the machines are partitioned (a minority side has no hub, only its nodes
  and agents, as today when the hub is off); separate trust domains for teams; and, on two machines, automatic takeover.

## 8. Staged plan (small PRs, each useful alone)
Prerequisite: ADR-0085 (certificate renewal) accepted and built, because section 3's rotation uses its `RenewRequest`.
1. **State export and import.** `agora hub export` / `import`: the existing `backup_to` and `restore`, plus the small-table subset and an owner-passphrase-sealed file if secrets
   are included (none by default: `ca.key` and `token` are excluded by construction, section 2; the `--with-ca-key` switch is dropped, so no export can carry the key). Useful alone: a moved hub, a better backup. Tests: the restore drill
   (`store.rs` tests, `hub_link.rs`) extended with a second hub dir.
2. **The signed hub record and following.** Schema (`schema/agora/v1/`, golden fixtures, capability `hub.record`), signing helper beside `devices.rs` messages, node storage of
   the record, multiple pinned CAs, `known_epoch` in the handshakes, the fencing behaviour (a hub told of a higher epoch demotes). No takeover yet: one hub, epoch 1.
   Tests: records with bad signature, lower epoch, same-epoch conflict, unknown group id; a node follows a record to a second address with a real hub process.
3. **Manual takeover** (`agora hub takeover`, the panel's "Make this machine the hub"): standby mode of the service, import of the last export, CA rotation, record publication,
   node pending-approval re-announce if missing. **Partition tests with real processes** (the `agora-fake-agent` scenario style): start hub A, node N, standby B; kill A; take over on B;
   N and a client follow; restart A and show it demotes; cut the link between A and B instead of killing A and show both can serve until N sees epoch 2, then A demotes; an agent keeps
   running throughout (the key assertion); revocation not resurrected.
4. **Continuous replication:** event stream, table feed, semi-synchronous revocations, replication-aware acks, detach on stall. A **takeover drill** like the restore drill, run in
   CI with real processes: measure RPO and RTO and fail the job above the numbers in section 2.
5. **Automatic takeover (three or more hub-capable machines only):** pre-signed owner grant, hub lease, majority agreement, flapping tests (laptop sleep, Wi-Fi switch, paused process),
   off by default. **On Windows, this step ships only after ADR-0077's code signing (SignPath) is in place** (Amendment below): until then a Windows standby may be quarantined
   by Defender, and an automatic takeover would depend on a machine that can silently lose its binary.
- **Per OS** (each is `agora-platform` or `servicecmd.rs`, never `#[cfg(target_os)]` elsewhere): **Linux** a systemd **user** unit `agora-hub` beside `agora-node` (lingering as today);
  **macOS** a second LaunchAgent, and the **Local Network** permission prompt for the hub's listening port and for discovery (an unsigned or re-signed binary is asked again: ADR-0077 and the
  update path must keep the identity stable); **Windows** a second logon task `agora-hub`, the hub's data under `%LOCALAPPDATA%` with a private ACL for any file
  holding a key (`write_private`), and the three points below (measured on a real Windows 11 machine, after this ADR was first written: `log/2026-11-17-1101-windows-hub-measurements.md`,
  `log/2026-11-17-1137-windows-adr0086-windows-review.md` on branch `comms`). The Windows developer supplies the Windows part, as offered.
  - **Firewall: Agora creates and removes its own rule; it never relies on the Windows Security Alert prompt.** On the first listen the firewall creates "Query User" **Block** rules for the
    executable (TCP and UDP, any port) and waits for a person; answered "Allow" (measured: within 2 s, on a machine whose UAC elevates without prompting) they become allow-everything rules
    (any port, Public included); on a default UAC machine the hub stays blocked until someone answers. A takeover with nobody at the machine would wait on, or widen through, that prompt.
    So at the elevated, visible hub-capable step ("Allow this machine to take over", `agora service install`), **before the hub ever listens**, Agora creates one inbound rule scoped to
    **the hub's TCP port and the installed `agora.exe` path**, in the profile of the network the hub actually serves. It is **not** "scoped to the private profile": on the measured
    machine the LAN adapter is in the Public profile (common on home PCs that answered "no" to network discovery), and a Private-only rule leaves the hub unreachable there.
    `agora doctor` reports when the serving network is Public and the rule therefore includes Public, and suggests making that network Private. Uninstall (and the removal of the
    hub-capable grant) removes the rule; so does an update that moves the executable path, which recreates it. This is also why Agora owns the rule: answering the prompt over time left
    12 stale allow rules (Private and Public, any port) for dev builds in deleted worktrees on the measured machine. **A machine without the rule is not offered as a takeover target.**
  - **Defender.** The binary is **not** code-signed yet (ADR-0077 is not built). One run (0 detections in 39 minutes, one machine, non-excluded path) is not evidence that an unsigned
    listening executable is never flagged. ADR-0077's signing is a **prerequisite of shipping step 5 (automatic takeover) on Windows**; until then a Windows standby may be quarantined,
    and the manual takeover (steps 1 to 4) says so in `agora doctor` for Windows machines.
  - **State** reaches a Windows standby only as section 2 says (`VACUUM INTO` / backup API, never the folder; `ca.key` and `token` never carried).

## Amendment (2026-11-17, accepted with this ADR on 2026-11-19): the Windows measurements
After the first text, the Windows developer measured on a real Windows 11 machine (release 0.3.16, hub started non-elevated as a logon task would, 39 minutes) and reviewed section 8.
Four changes, applied in place above and listed here so the history is visible:
1. The firewall rule is scoped to the hub's TCP port and the installed `agora.exe` path in the profile of the network the hub serves, not "to the private profile" (section 8, Per OS).
2. The Windows Security Alert prompt is never relied on; Agora creates its own scoped rule before the hub first listens, removes it on uninstall, and a machine without it is not a takeover target.
3. Hub state is never copied as files or as the folder; `ca.key` and `token` are excluded by construction (section 2; step 1 loses `--with-ca-key`).
4. "The binary is code-signed" was not true and is removed; ADR-0077 signing is a prerequisite of step 5 on Windows.

## 9. Open questions, with the proposed answer
1. **Automatic or manual takeover first?** Manual first (steps 1 to 4); automatic (step 5) only after the manual path has run for a release and only with three machines.
2. **Who may promote?** An owner, signing from a device already trusted on that machine; never a member or viewer.
3. **Hub address strategy: a stable name?** Optional but recommended: a name the owner controls (a LAN name or DNS name that moves with the hub) in the record's address list;
   the record's address list is authoritative and the name is one entry. Without it everything works through the record and `agora panel`, but direct-to-address browsers and passkeys
   are per hub (section 5).
4. **Is the CA key copied?** No (option E). Fallback option A only if the owner asks for simplicity, recorded as a widening of T3.
5. **How many standbys?** As many as machines the owner marks; replication is per standby. Recommended: one or two; automatic takeover needs a majority of the three hub-capable machines.
6. **Standby on every machine by default?** No: installed on request ("Allow this machine to take over"), because it is a security grant; `agora service install` offers it.
7. **Audit hash chain?** Yes, as its own small change before step 4, so continuity across an epoch is provable.
8. **Do sessions survive a takeover?** Yes (replicated hashes); on loss of the replica's last 30 s the person signs in again.
9. **Epoch conflict at the same number?** Refuse both, ask the owner (section 4).
10. **Does a superseded hub's divergent work ever merge automatically?** No: void unless the owner re-confirms each item.
11. **Retention of the replicated log on a standby?** Same as the hub's; measured in step 4.

## Threat-model impact (to be added to `docs/security/threat-model.md` when accepted)
- New rows: a standby machine is stolen or compromised (it holds the replicated tables, never the CA key under option E); a forged or replayed hub record (owner-signed, epoch-ordered,
  group-bound); an attacker serving a rogue hub (no valid record, no follower); a fork after a partition (fencing and void-unless-confirmed); a stolen owner device key can sign a takeover (it can
  already sign approvals; audited; revocable at once through the existing device revocation, which the standby honours).
- Unchanged: the hub cannot widen node policy (ADR-0020); nodes verify every approval; mesh messages stay untrusted; terminal bytes stay out of the store; agents' secrets stay on nodes.

## Consequences
- Adds: a hub record message and a handshake field (additive), a standby mode of the hub service, a replication stream and feed, a takeover command, certificate rotation by owner-signed
  record, per-OS service work. Requires ADR-0085 first and amends ADR-0056 decision 1 and the architecture document (the "hub stays single" statement becomes "one authoritative hub, any
  machine may take over") when accepted.
- Risks, stated: replication and fencing are the parts that are hard to get right and are only trusted after the partition tests of steps 3 and 4 pass in CI on all three operating systems; two
  machines cannot be made automatic safely; a long VM pause defeats the lease (fencing is the backstop); losing every owner device and the hub together is a re-enrolment.
