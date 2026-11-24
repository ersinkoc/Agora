# Pairing: find a hub, compare a number, connected (ADR-0065, rung a)

Status: built in steps (section 10); sections 12, 13 and 14 say what each step decided. The rest of this document is the design it was built from. ADR-0065 is accepted; this document is the protocol it asks for, written before the code
so that the part that decides who may join (a new way to authenticate) is read on its own. Rung (b), the typed number with a PAKE, is not here.

## 1. What it replaces
Today a machine joins with the hub's CA file, a one-time code and a long command (ADR-0015, `crates/agora-hub/src/enroll.rs`): the node
dials `wss://` trusting only the CA it was handed, sends `EnrollRequest { code, node_id, csr_pem }` and gets a certificate. Pairing removes the
two things a person has to carry between machines (the CA file and the code) and asks the owner for one decision on the hub, once, with a number
to compare. The join string and the code stay as the fallback.

## 2. The hub's switch: "open for pairing"
- Closed by default. `agora pair open` (and "Add a machine" in the window) opens it for **10 minutes**; `agora pair close` and the timeout close
  it; `agora pair status` says which. Opening and closing are owner-only and audited.
- While closed: no discovery answer (the hub is invisible), and every pairing message is refused with `pair.closed`.
- Closing ends every pending pairing session at once.
- **One machine per opening.** An approval closes the switch too, whoever gives it (an owner window, `agora pair open`): the hub does it when
  it takes the approval, so the approved machine still gets its certificate and every other request still waiting ends as closed (not a
  failure for anyone; its machine gets `pair.closed`). A refused approval (`pair.state`, `name_in_use`) closes nothing. The `pair.approve` audit
  row is the record of that closing (#942).

## 3. Finding a hub (UDP)
- A fixed UDP port (`DISCOVERY_PORT`, one constant, chosen at implementation), `std::net` only, no new dependency.
- Request: the 12 bytes `AGORA-DISCO1`, then 16 random bytes, **padded to 128 bytes**. A request shorter than 128 bytes or with another prefix is
  ignored: the reply is never larger than the request, so the hub cannot be used to amplify traffic at a spoofed address.
- Reply, sent unicast to the sender only: a protobuf `HubAnnouncement { name, port, ca_fingerprint, accepts_pairing: true, version }`
  (`ca_fingerprint` = SHA-256 of the CA certificate's DER). Replies are rate limited: 10 a second for the whole hub (every reply counts),
  and 3 **searches** per 10 seconds per source address. A search is one set of 16 random bytes: its copies from the same source inside
  that window (the query sent to both broadcast addresses, and the finder's repeats) are not counted again and are answered again at
  most once a second, so a lost reply is made good and a search made again at once is still answered. The finder sends its one query
  again every half second for its whole wait (another hub may answer later), and lists each hub once.
- An announcement is a **candidate**, never a trust anchor: nothing is shared with the hub because it answered. The fingerprint is shown to the person
  next to the hub's name and is public information. (A rogue responder can only get itself listed; it still cannot pass section 5.)

## 4. Messages (new, append-only; defined in `schema/agora/v1/pairing.proto`, field numbers and carriage in docs/specs/protocol.md "Pairing")
Machine to hub, on a `wss://` connection, in this order: `PairRequest`, `PairReveal`. Hub to machine: `PairChallenge`, `PairResult`.
Owner to hub (from an owner window): `PairApprove { pair_id }`, `PairDeny { pair_id }`, `PairOpen`, `PairClose`, `PairStatus`.
Hub to windows: `PairRequested { pair_id, kind (machine|browser), name, os, address, number, expires_ms }` and `PairDecided { pair_id, outcome }`.
New error codes: `pair.closed`, `pair.limit`, `pair.expired`, `pair.mismatch`, `pair.denied`, `pair.unknown`, `pair.state` (a message that is not allowed in
the session's current state, section 5a), `pair.insecure_transport` (a browser request over a transport that cannot carry the session safely, section 6)
(stable codes with parameters; the sentences live in `locales/`). `pair.names_missing` (the hub cannot name its addresses in its certificate, section 15) is sent in answer to `PairOpen`. One more is machine-local, never sent: `pair.name_unverified` (section 5, step 6b).

## 5. The handshake for a machine
Connection: the machine does **not** know the hub's CA yet. It dials the hub's TLS with a verifier that accepts a leaf certificate it cannot chain
to anything, and records `leaf_hash = SHA-256(leaf DER)` as the machine saw it. This connection carries the pairing messages and nothing else; the
verifier is used for pairing only, never for the node link, and is a separate type so no other code can pick it up by accident.

The verifier relaxes **one** check, trust in the issuer, and nothing else:
- It still verifies the TLS 1.3 handshake signature (`CertificateVerify`) against the leaf's public key with the crypto provider's own signature
  check (proof that the peer holds the key of the leaf it presented). It must never return an unconditional "valid" for the signature, and a
  missing or empty leaf, a failed or unsupported signature, or a handshake that does not complete is a refusal. Without this a party that merely
  replays a leaf it saw could pass as that hub.
- Pairing connections are **TLS 1.3 only** (so no extended-master-secret question and no renegotiation), with a fresh connection per pairing.
- The verifier cannot be constructed by the node-link or any other path, and the connection is closed after one pairing (section 7).
- **Session binding.** Immediately after the handshake, and before any pairing message, each side reads `exporter` = the TLS exporter
  (`export_keying_material`, rustls, no new dependency) with label `EXPORTER-agora-pair-v1`, an empty context and length 32 (the construction of
  RFC 9266, with our own label). The hub reads it from its side of the same connection. It is frozen for the session and then used in step 4.

1. **Commit.** The machine makes `nonce_n` (16 fresh random bytes from the system CSPRNG, never reused) and its keypair and CSR, and sends
   `PairRequest { commitment, node_name, os, version }` with
   `commitment = SHA-256("agora/pair/v1/commit" || nonce_n || csr_hash)`, where `csr_hash = SHA-256(CSR DER)`.
   The hub checks it is open and the limits (section 7) **before allocating any state**, makes `pair_id` (128 random bits) and `nonce_h` (16 fresh
   random bytes). The session now owns its tuple (section 5a).
2. **Challenge.** The hub replies `PairChallenge { pair_id, nonce_h }`.
3. **Reveal.** The machine sends `PairReveal { pair_id, nonce_n, csr_pem }`. The hub recomputes the commitment from `nonce_n` and the CSR it
   received; a different value ends the session (`pair.mismatch`, counted as a failure). The CSR is parsed and the proposed node id checked by
   the same rules as enrolment today.
4. **The number.** Both sides compute the same short number from what each of them saw:
   `sas = HKDF-SHA256(salt = "agora/pair/v1", ikm = exporter || leaf_hash || csr_hash || nonce_n || nonce_h, info = "sas")`, first 8 bytes as a
   big-endian integer, mod 10^6, shown as `482-193` (the bias of 2^64 mod 10^6 is below 10^-13). The hub's `leaf_hash` is its own certificate's and
   its `exporter` is read from its side of the connection; the machine's are what its verifier recorded and what its side of the connection derived.
   The machine displays "Approve on the hub. Number 482-193". The hub sends `PairRequested` (with the same number, the node's name and os,
   and the address the connection came from) to every owner window and every notifier (ADR-0051).
5. **Approve.** The owner compares the numbers and presses Approve (or Deny). Only an owner session may; the pairing must still be pending and
   inside its time limit (5 minutes from the reveal). Approve and Deny are **terminal and one-shot**: the first one moves the session out of
   `Pending`, the hub issues at most one certificate per session, and it does so for the frozen tuple (section 5a), never for values taken from
   a later message. On approval the hub issues the certificate with the existing `Ca::issue_node` for the node
   id (a name that already belongs to another machine is refused unless the owner explicitly says "replace", exactly as re-enrolment works today)
   and sends `PairResult { approved, cert_pem, ca_pem }` on the same connection, then closes it.
6. **The machine checks before it keeps anything.** (a) the certificate it got is for its own key and node id; (b) the leaf it saw in step 1
   chains to `ca_pem` (verified with the certificate library, with `ca_pem` as the only root, after the handshake signature was verified during the connection) and the address or name it dialled is one of that leaf's names (section 5b says how the hub's leaf gets that name; the
   check is never skipped, and a failure is the machine-local `pair.name_unverified`, the machine says so and stores nothing; the name half
   needs no CA, so it is also checked as soon as the handshake ends, before step 1, and a leaf that does not name the dialled address ends
   the pairing there: no request, no number, no card for the owner, nothing issued, so the hub keeps no name for it (#901)); (c) only then does
   it store the CA and its certificate in its link folder, as `agora node enroll` does today. A mismatch is thrown away and nothing is stored.
7. **Device trust in the same step** (ADR-0065 decision 5, ADR-0061): the machine then asks, on the machine, once, "Trust the owner's devices on
   this machine? [y/N]" with each device's fingerprint, reusing `agora node trust --from-hub`. It is the node's own question; the hub cannot
   answer it.

### 5a. Session state: what is frozen and what may happen once
A session is `Awaiting-reveal -> Pending -> Approved | Denied | Expired | Closed`, owned by **one connection**.
- At the commit the hub freezes, per session: the owning connection, `exporter`, `leaf_hash`, `commitment`, the node's source address and
  `deadline`; at the reveal it adds `nonce_n`, `csr_hash` and the proposed node id. None of these changes afterwards.
- `pair_id` is a lookup key and **not a bearer capability**: a message naming a `pair_id` is honoured only on the connection that owns it. Another
  connection naming it gets `pair.unknown`, gains nothing, and counts as a failure for its own address.
- A second `PairRequest`, a second `PairChallenge`, a second or replayed `PairReveal`, or a reveal out of order is `pair.state`: it ends the
  session (not a retry), counts as a failure and never replaces or edits the stored tuple.
- Approve and Deny are terminal. A reconnecting machine cannot reclaim a session: it starts a new one (new nonces, new limits check). A result is
  sent once, on the owning connection; if that connection is gone the approval is void and no certificate is issued.
- Every retry is a fresh session with fresh nonces; no nonce, commitment or exporter is ever accepted twice (the hub keeps the commitments of the
  last 10 minutes to refuse a repeat).

### 5b. The hub's certificate names (the default LAN install)
Today `hub_run` issues the hub leaf with the name `localhost` unless `--name` is given, so a hub reached by its LAN address could never pass
step 6b, and the later node link (which verifies the name against the CA it pinned) would fail for the same reason. Two ways to fix it:
- **Provision the names (recommended).** When the hub starts and again at every `agora pair open`, it takes the names it is reachable at: the
  `--name` values, `localhost`, its host name, and the IP addresses of the interfaces it listens on (for a wildcard listen address, each
  non-loopback address of its interfaces, through `agora-platform` since enumerating them is OS-specific). If that set is not covered by the
  current leaf it issues a new leaf from the **same CA** (so the CA fingerprint and the nodes already enrolled do not change) and the listener
  switches to it for new connections. Discovery announces only addresses that are in the leaf. Failure mode: if the interfaces cannot be listed
  or issuing fails, `agora pair open` fails with an error saying which names are missing and suggests `--name`; it never opens with a leaf that
  cannot match. An address that appears later (a DHCP change) is picked up at the next `pair open`; until then a machine dialling it gets
  `pair.name_unverified`, visibly, with nothing stored.
- **Pin the approved CA instead.** After the number comparison, accept the leaf by `ca_fingerprint` / `leaf_hash` without a name match. The
  number already authenticates the leaf, so this is not weaker for the pairing itself. It is **not recommended**: the node link afterwards
  verifies the hub's name against the pinned CA (that is what keeps a stolen-CA-less impostor out), so either that check would have to be
  relaxed for every later connection, or the machine would pair and then never connect.

Decision recorded here: **provisioning**; pinning is rejected as above (the maintainer may overrule: it would need a rule for the node link, which
is an ADR-0015/0062 matter and not decided here). Acceptance cases: a default hub (no `--name`) paired from a second machine by its LAN IP and by
its host name completes and the node link then connects; the same hub dialled at an address that is in no name fails closed with
`pair.name_unverified`; a leaf from another CA with the right name fails (wrong CA); the right CA with a leaf for another name fails (wrong leaf).

**Built:** provisioning, as chosen above, is section 15.

### Why this resists a machine in the middle
A party in the middle terminates the machine's TLS with its own certificate `C_m` and relays to the hub over a second connection. The machine's
`leaf_hash` is then `H(C_m)` and the hub's is `H(C_hub)`, and the two TLS sessions have different exporters: the two numbers differ except by
chance. (The party cannot present the hub's real leaf either: the handshake signature would need the hub's key.) The attacker sees the machine's *commitment*
before it chooses anything toward the machine, and the machine's nonce and CSR are revealed only after the attacker has fixed its own nonce toward
the machine, so it cannot grind a collision after the fact: it has one attempt for each pairing, about a 1 in 10^6 chance **per attempt**. A failed attempt shows up as two
different numbers on two screens, the owner presses Deny, and it counts as a failure (section 7). The hub is the owner's own machine, so the hub's
nonce needs no commitment. This is a reasoned argument, not a formal proof; a reviewer found no offline grinding break when nonces are fresh and a
session is one-shot, and the design depends on exactly those two rules (5a).

**Aggregate bound.** 10^-6 is the figure for one constrained comparison, not for a window. The budget that turns it into a bound: every failure
(mismatch, a Deny, an expiry nobody answered, an abort after the challenge, an invalid transition) counts; **5 failures hub-wide while the switch is
open close the switch** (`pair.closed` until the owner opens it again), and from one address 5 failures lock that address out for a minute
(section 7). Up to four sessions can wait at once, and a failure only counts when it ends, so one open window can put at most about 9 cards in front of the
owner (the 5 failures that close it, plus up to 4 waiting at that moment): the chance that a party in the middle gets a matching number in a
window is at most about 9 x 10^-6, **under 10^-5**, and each failed attempt is a card the owner saw. (An earlier draft said 6 x 10^-6; it did
not count the sessions that wait at the same time.) A connection that was never admitted (a first message that is a reveal) counts against its own
address only, never against the window's 5, so a host cannot close the owner's window by sending such messages (section 12).

## 6. A browser signing in
- A browser has no key and no TLS binding it can read, so the number here is **not** a handshake: the hub makes a random 6-digit number, shows it
  on the sign-in page ("Waiting for the owner to approve. Number 731-004") and on the owner's card ("Browser 'Chrome on Mac' (192.168.1.50) wants
  to sign in. Number 731-004"). Its only job is to let the owner tell two requests apart and to know it is the one they mean. It is a locator, never
  a password, and nothing is authenticated by it.
- **The poll token is the capability.** The hub makes it with the system CSPRNG, at least **256 bits**, and returns it **once**, in the response that
  creates the request, together with the number. The page keeps it in memory only (not in a URL, not in `localStorage`, not in a log, never in
  the card) and sends it in a request header on each poll. The hub stores only its SHA-256 and compares in constant time. It lives as long as the
  request (3 minutes at most) and is void on approve, deny, cancel, expiry, the switch closing and a hub restart. Pairing for a browser is only
  accepted over TLS or from loopback (`pair.insecure_transport` otherwise), because the session cookie is returned to the poll that proves the token.
- **Only the token's holder continues or ends a request.** A request is polled, cancelled or replaced by presenting its token; the address is never
  a credential. A new request without a token from an address whose quota is full is **refused** (`pair.limit`); it does not touch the pending one. So a
  stranger behind the same address (a shared office or home NAT) cannot invalidate the legitimate request. "Replace" means the holder cancels and
  asks again, and that is an admission like any other (the rates below), and it does not raise a new notice while a cooldown is running.
- **Bounds (normative for browsers, as for machines; section 7).** At most **4** pending browser requests for the whole hub and **2** per address, each
  lives **3 minutes**. Creating requests is limited **per address key** (exact address for IPv4, the /64 for IPv6) to **3 a minute and 20 in 10
  minutes**, and for the whole hub to **10 a minute and 60 in 10 minutes**, counted when a request is **attempted**, before anything is allocated or
  anyone is notified. Polling is limited to one poll per 2 seconds per token, 60 a minute per address key and 300 a minute for the hub; a poll over
  the rate gets `pair.limit` and does not extend anything.
- **Notices are coalesced.** The pending requests are state the owner's windows read as one list (at most 4 cards, each shown once). A push
  notification (ADR-0051) for a browser request is sent at most **once a minute for the hub**; requests that arrive during the cooldown are
  summarised in the next one ("2 more sign-in requests are waiting"), and window updates are batched to at most one a second. The trace "one
  legitimate request, then 10,000 attempts to replace it from the same address" therefore produces: the legitimate request still pending, at
  most 3 admissions a minute from that address (the rest refused before any state), and at most one notice a minute, not 10,001.
- Accepted: a flood from many addresses can fill the 4 pending slots (bounded by the rates above) and make a real request get `pair.limit` for up to
  3 minutes; it cannot cancel or replace anyone's request, cannot produce more than one notice a minute, and is visible to the owner, who can close
  the switch. Browser sign-in works only for an owner who is signed in somewhere to approve it; a hub with no signed-in window shows "ask the owner".
- On approval the hub makes a session for the polling browser: the poll that presents the token receives the session cookie **once**, and the token is
  void from then. Anything else about sessions is as today (ADR-0049, #450).
- Signing in a stranger by approving the wrong card is the one risk this cannot remove: the card names the browser and the address, and approving is
  the owner's decision, never the hub's.

## 7. Limits
These apply to **every kind** of request (machine and browser) and are checked before the hub allocates a session or sends a notice.
- Pending sessions: machines at most **4**, browsers at most **4** (2 per address key); from one address key at most **3 requests a minute**
  and **20 in 10 minutes**, and for the whole hub at most **10 a minute** and **60 in 10 minutes**, each kind counted on its own. A refused
  attempt counts as an attempt.
- A session lives **5 minutes** from the reveal (a browser request 3 minutes); a reveal must arrive within **30 seconds** of the challenge.
- **Failures** (commitment mismatch, a Deny, an expiry nobody answered, an abort after the challenge, a message in the wrong state, a `pair_id` not
  owned by the connection) are counted per address key: **5 failures lock that address key out for a minute**. Hub-wide, **5 failures while the
  switch is open close the switch** (the aggregate bound of section 5). These are **new keyed counters** (`PairLimits` in `agora-hub`): a map from
  `(kind, address key)` to sliding-window counts and a lockout time, plus one hub-wide counter, bounded to **1024 keys** and **10 minutes** of
  retention. A full table refuses new keys (fails closed, never evicts a live key to make room), so filling it cannot reset anyone's counters.
- They are **separate from fallback enrolment**. The existing `enroll::Lockout` is one `(count, until)` for the whole hub with no address in
  its interface, so reusing it would either not limit per address or let pairing noise lock out `agora node enroll` for everyone. It is left
  unchanged; pairing failures never touch it and its failures never touch pairing.
- One connection carries one pairing; the hub closes it after the result and after 30 seconds without a message.
- The state of every session is memory only and ends with its session, with the switch closing, or with the hub restarting. Nothing about a
  pairing is stored except the audit trail and, on approval, the certificate record enrolment already keeps.

## 8. Audit and what is never logged
Audited: the switch opened and closed (by whom); a request (name, os, address); approve or deny (the owner's device); the certificate issued (serial).
Never logged: nonces, the commitment, the exporter, the number, the CSR, the poll token or the session cookie. Limit refusals are counted and audited as a rate (kind, address key, count a minute), not one line per attempt, so a flood cannot fill the log.

## 9. Threat-model rows
The rows are in `docs/security/threat-model.md`: **T30** (a machine or browser asks to join without the owner meaning it; was T-PAIR-1), **T31** (a
party in the middle, or a second connection naming a `pair_id`; was T-PAIR-2), **T32** (a pairing secret leaks: the number, nonces, commitment,
exporter or CSR; new) and **T33** (the hub's certificate does not name the address dialled; was T-PAIR-4, honest about what is not solved yet).
The discovery row (was T-PAIR-3: a closed hub is silent, a request must be as large as the reply, replies are rate limited and unicast) is **T34**; the
re-issue of the hub's leaf while it runs is **T35**.

## 10. Order of work (each its own pull request, each security-reviewed)
1. The messages, error codes and golden fixtures (append-only), and this document updated with the ids. (Done: schema only, capability `pairing.v1`.)
2. The pairing core in the hub, pure and unit-tested (states, limits, SAS, commitment), audit; the owner's commands, `PairRequested` and `PairDecided` to owner windows, the machine handshake over the hub's TLS and the certificate at approval. (Done: `agora-hub` `pairing.rs` and `pairing_net.rs`; section 12 lists what was decided.)
3. `agora pair open|close|status`, provisioning the leaf's names (section 5b), the push notice (ADR-0051) and the browser path (section 6) on the hub.
   (Done in part: `agora pair open|close|status` and the machine's client, section 13, and provisioning the leaf's names, section 15. The push
   notice (only its once-a-minute limit is in `pairing.rs`, nothing sends it) and the browser path are not.)
4. The discovery responder (hub) and the finder, `agora join`, with the trust question (node). (Built: provisioning the leaf's names, the responder and the finder, section 15;
   the trust question too: `agora join` shows the hub's fingerprint and asks whether it matches what `agora pair open` shows, `docs/getting-started.md`.)
5. The window cards wired in (web #610 and TUI #611 already have the layout), then the browser sign-in by approval (section 6). (Built: the cards
   in the web window and the terminal client, section 14; the browser sign-in by approval is not.)

## 11. Decisions taken after the first review, and what is still open
- **Exporter and leaf both.** `leaf_hash` binds the endpoint certificate; the exporter (TLS 1.3, RFC 9266 style, 32 bytes, label
  `EXPORTER-agora-pair-v1`) binds the TLS session. Both go into the number. Cost: the pairing connection is TLS 1.3 only and the node's client
  stack must expose `export_keying_material` (rustls does; no new crate). Gain: the number depends on the TLS session, not only on the endpoint
  certificate. The reviewer found no break in the hash alone; this is chosen as the stronger and cheap option, and can be dropped to the hash if a client stack cannot expose the exporter (then that stack cannot pair).
- **Provision the leaf's names, do not pin** (section 5b). Open for the maintainer only if pinning is wanted instead: it would change the node link
  rule (ADR-0015/0062), which this document does not decide.
- **The verifier accepts an unchained leaf but not a bad signature** (section 5). The alternative, a hub certificate recognised by the discovered
  `ca_fingerprint`, adds nothing: a rogue responder can announce any fingerprint.
- **Open: the numbers** (3 a minute, 20 in 10 minutes per address key; 10 and 60 for the hub; 5 failures per window) are proposals for the
  reviewer. No ADR-0065 decision is changed: it asks for a bounded, owner-approved exchange; these are its bounds written out.

## Tests this design requires (before the code is integrated)
Machine: a party in the middle with its own certificate gets different numbers; a replayed leaf without its key fails the handshake signature; a
TLS 1.2 or empty-certificate server is refused; a second `PairRequest`, a second or replayed `PairReveal`, a reveal before the challenge, and a
reveal with another `nonce_n` or CSR are `pair.state`/`pair.mismatch` and change nothing; another connection naming the `pair_id` gets `pair.unknown`
and no authority; approve twice and approve-after-deny issue one certificate at most, none for a changed tuple; a reconnect cannot reclaim an
approval; the numbers use the exporter (two sessions with the same leaf and nonces differ); default hub (no `--name`) paired by LAN IP and by host
name succeeds and then connects; an address in no name, a leaf of another CA and a leaf for another name each fail with nothing stored; the aggregate
budget closes the switch at the 5th failure.
Browser: a stranger behind the same address cannot cancel, replace or poll the legitimate request (the legitimate token still works); a 10,000-request
churn from one address stays within the admission rates, leaves the legitimate request pending and produces at most one notice a minute; many
addresses fill at most 4 slots and no more; a token is single-use for the cookie, void after deny, cancel, expiry, close and restart, never appears in
a URL or a log, and a plain-HTTP non-loopback request is `pair.insecure_transport`; pairing failures do not lock `agora node enroll`, and its failures
do not lock pairing; a full counter table refuses new keys without resetting the others.

## 12. The hub core, as built (choices the design left open)
Built in `crates/agora-hub/src/pairing.rs` (the state machine, pure, every method takes the time) and `pairing_net.rs` (connections and the CA).
What this document did not say, and what the code does:
- **Who may pair.** A hub offers pairing (`pairing.v1`) once `hub.pairing().enable(leaf_hash)` was called with the SHA-256 of its own certificate's
  DER (`hub_run` does it); until then every pairing message is `protocol.capability_missing`. The leaf hash a session freezes is the one set at that
  moment, so a leaf that is swapped (section 5b, not built) must be set at the same time as the listener switches.
- **`node_name` is the node id.** It is checked at the reveal by the rules enrolment uses (`ca::valid_node_id`), and at the request only for size
  (1 to 64 bytes, no control characters; `os` and `version` the same).
- **An approval closes the switch** (section 2, #942): `Pairing::approve` ends every other session that is still waiting for its reveal or for the
  owner (`PairDecided` `closed` for the ones with a card, not counted as failures) and sets the switch closed, but leaves every approved
  session alone (this one, and one from an earlier opening that its connection has not collected yet), unlike `close`, which also voids an
  approval nobody collected. The window's later `PairStatus` says closed; discovery stops within half a second as for any closing.
- **Results.** A deny is `PairResult { approved: false }` with empty fields (the schema says so); an expiry is `pair.expired`; the switch closing
  is `pair.closed`. `pair.denied` is not sent by the hub core (the browser path may use it).
- **Errors for the wrong order.** A second request on a connection is `pair.state` naming the session's state (`awaiting_reveal` before
  the reveal, `pending` while the owner decides); a second reveal or any other message while waiting is `pair.state` naming the state.
  A reveal after the session ended is answered with how it ended (`pair.expired` or `pair.closed`); another message that arrives just as
  the session ends can instead get `pair.state` naming `expired` or `closed`. A first message that is a reveal (a connection naming a
  session it cannot own) is `pair.unknown`, and a repeated commitment, exporter or nonce is `pair.mismatch`; each ends the session and counts as a failure.
- **Approving a taken name.** If the node id belongs to an enrolled machine and `replace` is false, Approve is refused with `pair.state`
  (`state = name_in_use`) and the request stays pending, so the owner can approve again with `replace`. The same check runs again inside the
  issuing transaction. `replace` revokes the old certificates, as enrolling again does. The same refusal applies while another request for that node id is
  already approved and not yet issued (or being issued), with or without `replace`: only one of them could get the certificate, so the
  second stays pending until the first connection has ended.
- **Idle.** Before the reveal the 30-second rules apply. While a request waits for the owner, the connection is held for the session's 5 minutes
  and answers pings; a machine need not send anything (the 30-second silence rule would cut a request the owner has not seen yet).
- **How windows receive cards.** Two new frame kinds on the events stream, sent only to owner sessions: `PAIR_REQUESTED` (5) carrying `PairRequested`
  and `PAIR_DECIDED` (6) carrying `PairDecided`. A window gets the cards waiting when it subscribes, then each change, batched to at most one push
  a second. Memory only: never an event, never stored, the number never logged. Older clients ignore frame kinds they do not know.
- **Audit.** `pair.open`, `pair.close`, `pair.approve`, `pair.deny` (who is the owner's user), `pair.request` and `pair.issued` (who is
  `pair:<address>`, node the machine's name, outcome `ok`, a stable code, or the certificate serial) and `pair.refused` (a rate: count since the
  last report, once a minute). The number, nonces, commitment, exporter and CSR are never written.
- **The number's function** lives in `agora_hub::pairing::sas` with a test vector computed outside the code; the node side will move it to a shared
  crate or reimplement it against the same vector.
- **Which failures close the switch.** Only failures of an admitted session (mismatch, deny, expiry, abort, wrong order) count for the window's 5.
  A connection whose first message is a reveal was never admitted and has no session: it is `pair.unknown`, counts for its own address's
  lockout only, and cannot close the window. The admission counters count an attempt first against its own address and only an attempt that
  passed it against the hub's share, so one host that hammers cannot use up everyone's share (it gets `pair.limit` from its own counter).
- **Clock.** Every deadline (10-minute window, 5-minute session, 30-second reveal) runs on a clock that only goes forward (`Instant` from the
  start of the process); the wall clock is used only to show a time to a window and in the audit trail, so setting it back cannot keep a
  window or a request open.
- **Text on the card.** `node_name` must already be a valid node id at the request (so no audit row, card or counter holds anything else);
  `os` and `version` are at most 64 bytes with no control character and none of the bidirectional controls, zero-width characters or the byte
  order mark (U+200B to U+200F, U+202A to U+202E, U+2066 to U+2069, U+FEFF).
- **Not in this step:** the push notice itself (`Pairing::take_push` decides when, nothing sends it), provisioning the leaf's names, discovery,
  `agora pair`, the node's verifier, and the browser path.
- **Follow-up (needs a schema addition, so not done here):** the card does not say that the node name already belongs to an enrolled machine; the
  owner learns it from a refused approval (`pair.state`, `name_in_use`). A `name_in_use` field on `PairRequested` would let a window warn before
  anyone ticks Replace, which revokes a real machine and installs the requester.

## 13. The CLI and the machine's side, as built (choices the design left open)
Built in `crates/agora-node/src/pairing_client.rs` (the machine's protocol and checks), `crates/agora/src/joincmd.rs` (`agora join`), `paircmd.rs` (`agora pair`)
and `frontdoor.rs` (choice 3). Tests: `crates/agora/tests/pair_node.rs` (the client against a real hub listener, every refusal path) and `pair_cli.rs` (real processes).
- **The client is a state machine plus a connection.** `Exchange` (no sockets, no clock, no randomness) builds the commitment, validates the challenge (16-byte
  `pair_id` and `nonce_h`), computes the number and takes the answer once; any message out of order ends the session (`Unexpected`). `pair()` is the connection around it.
  The number function is reimplemented on this side with `ring` and checked against the same vector as the hub's (`497259`).
- **The verifier.** `UnchainedLeaf` is private to the module: it records the leaf, refuses an empty one, refuses TLS 1.2 signatures and verifies the TLS 1.3
  handshake signature with the provider's own check (a peer that cannot sign with the leaf's key fails: tested with a certificate and another key's signature).
  The configuration offers TLS 1.3 only; a hub that only speaks TLS 1.2 is refused with a sentence of its own, nothing stored.
- **What is checked before anything is kept (step 6).** The delivered CA is exactly one certificate (a bundle is refused: what is pinned is the one CA the hub's
  leaf chain used) that is a CA with the signing usage and valid now. The leaf chains to it and the dialled host is one of its leaf's names, else `pair.name_unverified`. The delivered certificate chains to that CA (client-auth verifier), is valid now, starts no more than the 10-minute clock allowance ahead of the machine's clock and lasts at most 366 days, has the client-authentication usage and is
  not a CA, its common name is the node id, and its public key is the one in this machine's own CSR. Each check has a test against a certificate wrong in just that way
  (`pairing_client.rs`, `check_delivery`), and each test was seen to fail with its check removed. The CA's fingerprint is printed in the same format as
  `agora hub fingerprint`. The module stores nothing; `agora join` stores only what `pair()` returned.
- **The name is checked twice: first before anything is sent** (#901, real-machine run RM-07). Right after the handshake, before `PairRequest`, the machine
  checks that the dialled host is one of the leaf's names (`pairing_client::leaf_names`: rustls' own name check, the one the final check runs, without the chain,
  which needs the CA delivered at the end). A leaf that fails it is `pair.name_unverified` at once and the connection is closed: no number is shown, the owner gets
  no card, the hub opens no session and issues no certificate, so it keeps no name a later join would have to replace. Before this the owner could approve, the
  hub issued and recorded the certificate, and the machine then threw it away, leaving a name the hub refused without "replace" although no machine had joined.
  Passing the early check proves nothing about the hub: the whole of step 6 still runs after the approval. Tested over real TLS in `pair_node.rs`
  (`an_address_that_is_in_no_name_of_the_hubs_leaf_fails_before_the_owner_is_asked_and_leaves_no_name`) and in `pairing_client.rs`. What is not solved: a join
  thrown away **after** the approval for another reason (the leaf does not chain to the delivered CA, a delivered certificate that fails its checks, a
  confirmed CA fingerprint that differs, the machine stopped or could not store) still leaves the certificate the hub recorded, and the owner's next join of that
  name must choose Replace. The hub cannot tell that the machine threw it away: `PairResult` is the last message and nothing answers it. Releasing such a name
  needs either a new message from the machine (a schema change) or a rule for certificates that never connected (a decision on section 12's taken-name rule),
  so it is left to an ADR.
- **A leaf that does not chain to the delivered CA is reported as `pair.name_unverified`** too (step 6b lists both under that code), with the address as the
  parameter. The sentence tells the person to ask the owner to start the hub with `--name ADDRESS`.
- **The address.** `host`, `host:port`, `[v6]:port` or `wss://host:port`; the default port is 7443 (the front door's). The host is both what is dialled and the name
  checked; for an IP number the leaf must carry that IP (a hub with only `localhost` fails closed, as the acceptance cases say). The hub address is stored as
  `wss://<what was dialled>` in `link/url`.
- **Storing** is `hubcli::HeldLink::store`, shared with `agora node enroll`: the same files, every one through `write_private` (a temporary file made new and renamed over the target, so a planted link is replaced, not followed, and a stop never leaves half a file). `node-id` is removed first and written last, so a
  stop at any point (tested with an injected failure between every pair of writes) leaves a folder that does not look joined, also when joining again with `--replace`. Every save holds the folder's guard (`link/lock`, an OS file lock opened with `open_lock_private`), so two saves that overlap (two `agora` runs enrolling at once, a `--replace` beside them) never mix one's key with the other's certificate (audit F1000). Whoever decides from `node-id` decides under that same guard: `agora join` looks again after the owner approved and stores in one step, so a link another save completed meanwhile is seen and kept (audit F1032); every enrollment with a code (`agora`'s first one with its own hub, and `agora node enroll`) holds the guard from its look through the hub's answer to the end of its save, so two of them ask the hub and save in the same order and the certificate saved last is the one the hub kept (audit F1030 and its Codex review). A join cannot hold the guard while the owner decides, so after approval it keeps a link of the same hub under the same name whose certificate ends no earlier than the one just delivered, also with `--replace`: the hub revoked the delivered one when it issued that newer one (certificates issued within the same second are not told apart: the one in the folder is kept, and the join says it cannot tell which one the hub kept and exits 1: whether the hub replaces a name in use is the owner's choice when approving). A waiter waits as long as a working holder can hold the guard (the enrollment deadlines: 25 s for the connection, 15 s for the answer, and a 10 s margin), then fails changing nothing. The node reads the five files under the guard too, through the folder it holds, so it never starts with one link's id and another's key (audit F1031); no folder is no link, and a folder that is there but cannot be resolved or entered (a link to an unmounted volume, no permission) is an error; where the guard cannot be had at all (a read-only folder) it reads at once without it, and where another command held it past the whole wait it reads nothing and ends, to be started again by its supervisor. `agora join` on a machine that already has a link refuses (exit 2) unless `--replace`. When the machine has no `machine.toml`
  it writes the settings of a node-only machine, so plain `agora` starts the node; an existing one is never touched.
- **The number is shown once.** `join` prints one line on standard output (`Number 482-193. Approve this machine on the hub...`) as soon as the reveal is sent.
  `Number`'s `Debug` prints `Number(..)`; the nonces and the exchange have no `Debug`. A test greps the captured output and every file of the machine (and the node's
  logs) after the flow: the number is in that one line and nowhere else.
- **Timeouts.** Connect and handshake 10 s each; the challenge 15 s; the owner's decision 5 minutes 30 seconds (the hub's session is 5 minutes from the reveal).
- **`agora pair open` is the owner's screen at a terminal (the window's cards are section 14), and only a person at a terminal can answer it.** It opens the switch (the
  hub's 10 minutes; `--minutes N` with N from 1 to 10 makes this command close it earlier, which only works while the command runs, because `PairOpen` carries no
  duration and the schema is append-only), stays in the foreground, and for each machine card prints the name, system, address and number and asks `y/N`.
  **The answer is read from the terminal only** (`agora_platform::read_confirmation`): when stdin is not a terminal the command refuses to start (exit 2, "needs you at a
  terminal", nothing opened), so a pipe, a file or a script (`yes | agora pair open`, a loop of `echo y`) can never approve. On a terminal, what was typed, held or sent before the
  question is discarded when the read starts (`tcflush` on Unix, `FlushConsoleInputBuffer` on Windows), a first key that arrives sooner than 700 ms after that is ignored (three
  such tries, then the request is denied), the line is at most 256 bytes, and nothing is echoed. Only `y` (or the locale's yes word) then Enter approves; any other line,
  including an empty one, denies. A taken name (`pair.state`, `name_in_use`) asks once more whether to replace. After one approval the hub has closed the switch itself
  ("one machine per opening", section 2): the command asks `PairStatus`, says the hub is closed again and exits, sending no `PairClose` at all. It never closes an
  opening that is not its own: it keeps the `closes_at_ms` its `PairOpen` was answered with (each opening has its own deadline), and only a
  hub that still shows **that** opening open after an approval, which only a hub from before #942 does, is closed with `PairClose` (the legacy
  fallback; reading the status and closing are two commands, so on such an old hub an opening made between them would be closed too: a known
  limit). An opening a window made meanwhile (the owner approved there and opened a new round while this command fell behind) is left open
  and the screen says so; a status that cannot be read closes nothing and says to run `agora pair status`. The `--minutes` deadline closes
  only this command's own opening by the same rule (Codex review of #1042); when the terminal goes away it leaves the switch open for its time and exits. **An owner's window
  may answer first** (#942): while it asks, the command keeps reading the hub's notices (`agora_platform::read_confirmation_until`, the same
  terminal-only discipline, read straight from the terminal and checked every 50 ms), and when the request it asks about ends anywhere else the
  question ends, nothing typed counts, and it says how: approved in a window (then it says the hub is closed again and exits, as after its own
  approval), denied in a window, or ended (ran out, the machine stopped, the hub closed), after which it waits for the next machine. The replace
  question ends the same way. **A question that comes without a keystroke** (the one before ended elsewhere, so the owner may have been typing an
  answer to it) is set apart: a separator line ("The last request ended without your answer here. A different machine is asking now...") comes
  before its card, which names the machine and its number, and an answer counts only from **2 seconds** after the question (`NEXT_THINK`,
  instead of 700 ms). A key sooner than that is thrown away with everything typed before it, said once ("That key came before you could read
  the new request..."), and the question and its wait start again, as often as it happens: it never counts toward the three tries, so a burst
  meant for the last request can neither approve nor deny this one. The ordinary first question keeps the 700 ms and three-try rules; an answer
  typed here makes the next question ordinary again. Tested in `pair_cli.rs` (a `y` typed as the window denies the first machine and 1.2 s
  after the next question, twice, approves nothing; a `y` after the wait approves it). Ctrl+C leaves the switch open for the rest of its time (run `agora pair close`).
  Browser cards are ignored here: they wait for a window. The owner's windows (section 14) answer without this command.
- **How `agora pair` reaches the hub.** The switch and the cards are memory of the running hub, so these commands connect to it as the owner with the client token in
  `<hub dir>/token` (the same path as `agora tui` on the hub's machine) over the TLS the hub already serves; `agora_tui::client::Session` now keeps the
  `PAIR_REQUESTED` and `PAIR_DECIDED` frames it receives (at most 64) for `take_pair_notes`, and exposes `command_info` (the error parameters).
- **Not in this step:** the owner's devices trusted in the same step (item 7 above). `PairResult` has no field for a device list, so the machine would have nothing to
  verify; it needs a schema addition and its own pull request. `agora join` says nothing about devices, and `agora node trust --from-hub` still works afterwards.
  Also not built here: the push notice and the browser path. (Provisioning the certificate's names and discovery were built next: section 15.)

## 14. The owner's cards in the web window and the terminal client, as built (choices the design left open)
Web: `apps/web/src/net/pairing.ts` (pure: the notices, the state, the rules), `usePairing.ts` (the connection), `components/PairRequest.tsx` (the card),
`Pairing.tsx` (the window switch and the recent lines). Terminal: `crates/agora-tui/src/pair.rs` and `p` in `agora tui`. Tests: `pairing.test.ts`,
`pairingCards.test.tsx`, `pair.rs`, and `crates/agora/tests/pair_window.rs` (a real hub, the real `agora join`, an owner window using the owner commands).
- **Memory only.** Cards live in client state (React state, `Book` in the terminal client). Nothing is written to `localStorage`, `sessionStorage`, a file or a log,
  and the number is never in the page title or a notification (the title count and the notifications read approvals only; tests render everything with storage and
  console spied on).
- **Who may see a card.** The hub sends the notices to owner sessions only; a client also does not trust that. It asks `PairStatus` once its link is live (and every
  20 seconds in the web window, which also keeps the switch's time current): `ok` means the session is the owner's, `auth.forbidden` means it is not (what was held is dropped
  and later notices are ignored), `protocol.capability_missing` means this hub does not offer pairing. Cards are shown only after `ok`.
- **Reconnect.** Every new connection starts with a `reset` (the web) or `Book::reset` (the terminal client): the hub sends the cards waiting at subscribe, so a card
  that is not sent again is stale and gone. A hub that restarted has none.
- **Where the card is.** A request is a "needs you" item: a row in the fleet console's list (it counts in the header strip and the status bar) with one button, Review.
  The row never has Approve and never shows the number. The full card is in the right-hand Needs you list (on a phone, at the top of the list), and Review brings it into
  view. Away from Home, the first card floats over the window. On a hub with no machine yet the row is above the empty state.
- **Two presses to approve.** The first press (labelled **Numbers match…** since the My machines redesign) asks "Do the two numbers match exactly?" and only "Yes, the numbers match" sends it; Deny is one press. The question has a place of its own, so the buttons do not move when it appears, and the yes is disabled for 500 ms (`MIN_THINK_MS`) after it appears, so a double-click on Approve cannot reach it. `pairClicks.test.tsx` presses the real buttons in a DOM (mutations of each rule fail it). Replace is shown only after the
  hub answered `pair.state name_in_use` to an approval (the frame has no field for it, see section 12), starts unticked, and choosing it starts the two presses again.
  While the link is down, or the hub's expiry has passed, both buttons are disabled with the reason. The countdown is the frame's `expires_ms` against the window's clock.
- **Peer text.** Name, system and address are stripped of control, bidirectional and zero-width characters when the notice is read (`peerText`), shown isolated (`bdi`), and a name placed inside a sentence goes between Unicode isolates, because the web formats messages without Fluent's isolation.
- **Name, system and address** have a row each; the name and the system are isolated with `bdi` and the address is left-to-right; the number is two groups of three in large type.
- **Open the window.** On a fresh hub's empty state the owner can press "Allow a new machine to join" (`PairOpen`: 10 minutes), sees the time left, and can Close it (`PairClose`).
  A `PairDecided` `approved`, for any request and from wherever it was approved, shows the switch closed at once (the hub closed it, section 2), instead of at the next 20-second `PairStatus`.
- **Terminal client.** `agora tui` reads `p`: the first request as a card (name, system and address in rows, the number in two groups). `a` then `y` approves; any other key after
  `a` goes back; `d` denies; `r` chooses Replace on a taken name; Esc leaves it for later; nothing is sent once the request has run out or a command found the link gone. Keys typed before the card was shown are discarded, and a key sooner than 500 ms after it is
  shown is not an answer (the same rule as `agora pair open`). The footer says a machine wants to join, and counts it in "need you".
- **Not built:** browser sign-in by approval (the HTTP path and its card text), trusting the owner's device from a card, and a push notice for a request that arrives while no window is open.

## 15. Names and discovery, as built (choices the design left open)
Built in `crates/agora-hub/src/names.rs` (which names), `leaf.rs` (issuing and replacing the leaf), `discovery.rs` (both halves of the UDP exchange), `tls.rs`
(`TlsSlot`), `crates/agora-platform` (`interface_addresses`), `crates/agora/src/hubcli.rs` (wiring) and `joincmd.rs` (the finder's screen).
Tests: unit tests in the first three files, `crates/agora/tests/pair_node.rs` (a real hub with a real listener and the machine's real client) and `pair_cli.rs`.

**The leaf's names (section 5b).**
- Which names: the `--name` values (kept first, in the order given, lower-cased, an IPv6 literal with or without brackets), `localhost`, the host name (only
  when a certificate can carry it: letters, digits, `-`, `.`, `_`), then addresses. A listen address that is a specific address names that address alone, even
  a loopback one (the owner chose it), and an IPv4-mapped one (`::ffff:a.b.c.d`) names its IPv4 address too, the one machines dial; `0.0.0.0` names every address of every interface that is up, IPv4 only; `::` names IPv4 and IPv6 (a dual-stack socket
  serves both). Always left out: loopback, unspecified, multicast, broadcast, IPv4 link-local (169.254/16), IPv6 link-local (fe80::/10, which needs a zone a
  certificate cannot carry) and IPv4-mapped IPv6. Order: owner names, `localhost`, host name, IPv4 ascending, IPv6 ascending (so the same set is the same
  list; a temporary (privacy) or deprecated IPv6 address is not named: Linux says so through `/proc/net/if_inet6`, Windows through the suffix origin and
  lifetime, macOS does not say through `getifaddrs`, so it is not filtered there). At most 16 names; an owner name is never cut (more than 16 `--name` values, or a name a certificate cannot carry, refuses with `pair.names_missing`),
  addresses past the cap are cut from the end.
- Extra names (ADR-0067 rung 3): `agora remote tailscale` writes the machine's tailnet name and addresses to `<hub dir>/remote-names` (checked names only, at most 4). The hub reads it at start (they are then owner names) and every 3 seconds while it runs, and at every `PairOpen`; a name that is new makes it issue a new leaf from the same CA exactly as below (`Leaf::with_extra`). No message from the network supplies a name.
- When the leaf is replaced: at start, and at every `PairOpen` (a window's or `agora pair open`'s), **only when a wanted name is not in the leaf**. An address
  that went away does not force a new leaf, and the old names stay until the next re-issue. The new leaf is made by the same CA (its fingerprint and every
  enrolled node are untouched), written to `<hub dir>/server-cert.pem` (atomically and owner-only, `agora_platform::write_private`; **before** it is switched on, so a file that cannot be written
  refuses the open with `pair.names_missing` reason `write` and changes nothing), and put in a `TlsSlot` that the accept loop reads for every new connection. A
  connection that is already open keeps the certificate it started with. The slot also carries the leaf's hash, so a pairing session freezes the hash of the
  certificate **its own connection was shown** (`Pairing::request_on`), not the hub's current one: a leaf replaced between a machine's handshake and its
  request cannot make the two numbers differ.
- Failure: at start a system that will not list its interfaces does not stop the hub (it carries the names it knows, and says so); at `PairOpen` it **refuses**
  with the new error code `pair.names_missing` (`reason` is `interfaces`, `invalid`, `too_many`, `issue` or `write`; `names` is the list concerned), the switch stays
  closed, and the sentence tells the person to start the hub with `--name ADDRESS`. Interfaces are only listed for a wildcard listen address.
- `agora doctor` keeps reading `server-cert.pem`; the hub prints `The hub's certificate names: ...` at start.
- **What `agora pair open` tells the owner to type (#943, RM-07).** Only an address the hub listens on **and** its certificate names, since a machine refuses
  any other (`pair.name_unverified`). After `PairOpen` (which re-issues the leaf first when a name is missing) the command reads the listen address from
  `<hub dir>/listen` and the names from `<hub dir>/server-cert.pem` (`paircmd::join_hint`, no I/O, unit-tested). A loopback listen address: it says the hub
  listens on this computer only, so another machine cannot reach it, and how to make it listen on the network: first on this computer's address on it (the
  private or 100.64/10 IPv4 address of an interface that is up, `agora --listen ADDRESS:PORT`, then `agora down` and `agora`; `--listen` for a hub run by
  hand; with several such addresses it lists them all and asks for the one on the other machine's network instead of picking one, #993), with `0.0.0.0:PORT` as the fallback, said plainly to listen on every IPv4 interface, a public one too (reaching the listener is broader than opening the
  switch, though a join still needs the number). It offers only a join from this same computer (the loopback address it listens on, else `localhost`, whichever
  the certificate names). Addresses are offered only where the hub surely listens: a specific address alone; for `0.0.0.0` the certificate's IPv4 addresses;
  for `::` its IPv6 addresses only, because whether `::` also serves IPv4 depends on the system (the listener does not set `IPV6_V6ONLY`, and Windows binds it
  IPv6-only), never a loopback one. For a wildcard listen address the command lists this computer's interfaces (the only I/O besides the two files; the
  list is passed to `join_hint`), because the leaf keeps the addresses it names while the wanted ones are covered, so an address an interface no longer has
  (a changed DHCP lease) can still be in it (#993): an address of an interface that is up comes first, in the certificate's order; one no interface that is up
  has now is listed apart, as an address that may be old or may lead here through the network (an address in front of a NAT given with `--name`), never as one
  the hub surely answers at. When the interfaces cannot be listed, every address counts as present. The first present address is the one to type, the rest are
  listed. The certificate's DNS names other than `localhost` are listed as names that *may* reach the hub if the network resolves them to this computer, never
  as a promise; with no present address to offer, the first name is suggested with that condition, else the first address no interface has, with the same
  condition. None: it says so and suggests `--name ADDRESS`. It no longer suggests the computer's host name as the address to type.

**Discovery (section 3).**
- Port `DISCOVERY_PORT` = **7446**, UDP, IPv4 only. Not IPv6: it has no broadcast, and multicast needs a group membership per interface; a finder on an IPv6-only
  network gets nothing and uses the address.
- The request goes to the **limited broadcast** address (255.255.255.255) and to **each interface's directed broadcast** address, from a socket bound to that
  interface's address, repeated every half second for three seconds. No multicast.
- The responder (a thread of `agora hub run`) **binds `0.0.0.0:7446` only while the switch is open** and unbinds (and the thread ends) within half a second of it
  closing or timing out; nothing listens while closed. A port another process holds is not an error: the hub stays silent.
- It answers a datagram only if it starts with `AGORA-DISCO1` and is **at least 128 bytes**; the reply is a `HubAnnouncement` of at most 108 bytes (name cut to
  48 bytes, version to 16), so it is never larger than the request. The reply carries the hub's host name, the port of its `wss://` listener, the SHA-256 of
  the CA certificate and the version, and nothing else: no address list, no secret, no certificate, no node or project name.
- **Only an address in the leaf is announced.** The announcement has no address field: the finder dials the address the reply came from. The responder finds
  the address the system would send from toward the asker (a UDP socket "connected" to it sends nothing and only picks a route) and answers only when that
  address is one of the leaf's IP names. A query that reaches the hub on another address (a VPN card the leaf does not name, loopback) gets silence.
- Rate limits: **10 replies a second for the whole hub, 3 per 10 seconds per source address** (a sliding window each; the table holds 32 sources and a full table
  drops new sources, never forgets old ones). The limits are checked before anything else is done.
- The finder lists each hub once, even when it answers from several of its addresses (the first address that answered), and **keeps at most 8 hubs**. It reads
  at most 32 datagrams per socket per pass and looks at its deadline inside the receive loop, so a flood of answers can neither grow the list nor hold it
  past its three seconds (tested with a flood). It ignores anything that is not a well-formed `HubAnnouncement` of at most 128 bytes with accepts-pairing, a
  port, a 32-byte fingerprint and a name and version of plain text (no control, bidirectional or zero-width characters).
- **Discovery is never silently trusted.** `agora join` without an address always asks, at a terminal, even for exactly one hub: it shows the hub's name, its
  address and a short fingerprint of the hub's certificate authority (the first 12 bytes of the SHA-256, 24 hex digits in 6 groups of 4, for example
  `566A D067 41A8 B995 CD07 C464`, one shared function) and says "compare it with what `agora pair open` shows on the hub". `agora pair open` and
  `agora pair status` print the **same** short fingerprint on the hub's screen (tested on real processes). With one hub the person answers yes or no; with several
  the person types the number of the one whose fingerprint matches. **Both questions use the same terminal-only reader as the owner's approval**
  (`agora_platform::read_confirmation`): typed-ahead input is discarded, an answer that comes less than 700 ms after the question is ignored (three tries), and
  nothing is echoed (so a chosen number is typed blind). With no terminal discovery chooses nothing (`pair.discovery_unconfirmed`, exit 2, "give the address").
  When the address is typed, nothing changes: no fingerprint is asked.
- **The delivered CA must be the confirmed one.** The machine keeps the full SHA-256 of the announced CA (`Wanted::expected_ca`) and, after the hub delivered its
  certificate authority and every other check passed, compares the SHA-256 of the delivered CA's DER with it; any difference is `pair.ca_mismatch` (machine-local,
  never sent) and nothing is stored (tested, with one flipped bit). The machine compares all 32 bytes; the person compares 12.
- **What protects, and what does not.** A party on the network can announce a hub of its own, or copy a real hub's public name, port and fingerprint. The number
  (section 5) does **not** protect here: it defends against a relay between the machine and the real hub, but a rogue hub that the machine actually dials is its own
  approver, shows a card to nobody and approves at once. Against a rogue hub the **only** protections are (1) the person comparing the announced fingerprint with
  the one on the real hub's screen (a rogue that announces its own CA shows a fingerprint that is not the real one), and (2) the machine's check of all 256 bits: a
  rogue that copied the announced name, port and fingerprint cannot present the real CA (it does not hold its key, so it cannot issue a leaf that chains to it) and
  fails with `pair.ca_mismatch`. Not protected: a person who confirms without comparing, and the shortness of what a person compares (96 bits: making a look-alike
  CA that matches is an offline search of about 2^96 attempts, out of reach today, which is why the machine still compares all 256 bits).
- **Availability only.** The finder's list is keyed by CA fingerprint, name and port, not by address. A rogue that copies those and answers first can make the
  finder list its address instead of the real one, which ends in a failed join (`pair.ca_mismatch`, nothing stored), or fill the 8 slots with look-alikes; it
  cannot make a join succeed against itself and cannot remove a real hub's answer. It cannot learn anything secret: the
  announcement and the query hold only public data.
- Platform: `std::net::UdpSocket` needs no platform code. `SO_REUSEADDR` and `SO_EXCLUSIVEADDRUSE` are not set (two hubs on one machine: the second stays silent).
  Broadcast reception was measured by hand on Linux (a real hub and `agora join` on one machine, LAN and VPN cards); CI measures the same code over loopback
  (`discovery.rs`, `pair_node.rs`) because runners may block broadcast. macOS and Windows broadcast is **not measured by hand**.
- `agora_platform::interface_addresses` (Linux and macOS `getifaddrs`, Windows `GetAdaptersAddresses` with unicast addresses only) is exercised on all three
  systems by `crates/agora-platform/tests/interfaces.rs`; the Windows code was written without a Windows machine and is checked by CI only: **unmeasured by hand**.
