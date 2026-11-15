# Pairing: find a hub, compare a number, connected (ADR-0065, rung a)

Status: design for review. Nothing here is built. ADR-0065 is accepted; this document is the protocol it asks for, written before the code
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

## 3. Finding a hub (UDP)
- A fixed UDP port (`DISCOVERY_PORT`, one constant, chosen at implementation), `std::net` only, no new dependency.
- Request: the 12 bytes `AGORA-DISCO1`, then 16 random bytes, **padded to 128 bytes**. A request shorter than 128 bytes or with another prefix is
  ignored: the reply is never larger than the request, so the hub cannot be used to amplify traffic at a spoofed address.
- Reply, sent unicast to the sender only: a protobuf `HubAnnouncement { name, port, ca_fingerprint, accepts_pairing: true, version }`
  (`ca_fingerprint` = SHA-256 of the CA certificate's DER). Replies are rate limited (10 a second for the whole hub).
- An announcement is a **candidate**, never a trust anchor: nothing is shared with the hub because it answered. The fingerprint is shown to the person
  next to the hub's name and is public information. (A rogue responder can only get itself listed; it still cannot pass section 5.)

## 4. Messages (new, append-only; ids in `commands.proto` / `events.proto` / `client.proto` at implementation)
Machine to hub, on a `wss://` connection, in this order: `PairRequest`, `PairReveal`. Hub to machine: `PairChallenge`, `PairResult`.
Owner to hub (from an owner window): `PairApprove { pair_id }`, `PairDeny { pair_id }`, `PairOpen`, `PairClose`, `PairStatus`.
Hub to windows: `PairRequested { pair_id, kind (machine|browser), name, os, address, number, expires_ms }` and `PairDecided { pair_id, outcome }`.
New error codes: `pair.closed`, `pair.limit`, `pair.expired`, `pair.mismatch`, `pair.denied`, `pair.unknown` (stable codes with parameters;
the sentences live in `locales/`).

## 5. The handshake for a machine
Connection: the machine does **not** know the hub's CA yet. It dials the hub's TLS with a verifier that accepts any leaf certificate and records
`leaf_hash = SHA-256(leaf DER)` as the machine saw it. This connection carries the pairing messages and nothing else; the verifier is used for
pairing only, never for the node link, and is a separate type so no other code can pick it up by accident.

1. **Commit.** The machine makes `nonce_n` (16 random bytes) and its keypair and CSR, and sends
   `PairRequest { commitment, node_name, os, version }` with
   `commitment = SHA-256("agora/pair/v1/commit" || nonce_n || csr_hash)`, where `csr_hash = SHA-256(CSR DER)`.
   The hub checks it is open and the limits (section 7), makes `pair_id` (128 random bits) and `nonce_h` (16 random bytes).
2. **Challenge.** The hub replies `PairChallenge { pair_id, nonce_h }`.
3. **Reveal.** The machine sends `PairReveal { pair_id, nonce_n, csr_pem }`. The hub recomputes the commitment from `nonce_n` and the CSR it
   received; a different value ends the session (`pair.mismatch`, counted as a failure). The CSR is parsed and the proposed node id checked by
   the same rules as enrolment today.
4. **The number.** Both sides compute the same short number from what each of them saw:
   `sas = HKDF-SHA256(salt = "agora/pair/v1", ikm = leaf_hash || csr_hash || nonce_n || nonce_h, info = "sas")`, first 8 bytes as a big-endian
   integer, mod 10^6, shown as `482-193`. The hub's `leaf_hash` is its own certificate's; the machine's is what its verifier recorded.
   The machine displays "Approve on the hub. Number 482-193". The hub sends `PairRequested` (with the same number, the node's name and os,
   and the address the connection came from) to every owner window and every notifier (ADR-0051).
5. **Approve.** The owner compares the numbers and presses Approve (or Deny). Only an owner session may; the pairing must still be pending and
   inside its time limit (5 minutes from the reveal). On approval the hub issues the certificate with the existing `Ca::issue_node` for the node
   id (a name that already belongs to another machine is refused unless the owner explicitly says "replace", exactly as re-enrolment works today)
   and sends `PairResult { approved, cert_pem, ca_pem }` on the same connection, then closes it.
6. **The machine checks before it keeps anything.** (a) the certificate it got is for its own key and node id; (b) the leaf it saw in step 1
   chains to `ca_pem` (verified with the certificate library, with `ca_pem` as the only root) and the address it dialled is one of that leaf's
   names; (c) only then does it store the CA and its certificate in its link folder, as `agora node enroll` does today. A mismatch is
   thrown away and nothing is stored.
7. **Device trust in the same step** (ADR-0065 decision 5, ADR-0061): the machine then asks, on the machine, once, "Trust the owner's devices on
   this machine? [y/N]" with each device's fingerprint, reusing `agora node trust --from-hub`. It is the node's own question; the hub cannot
   answer it.

### Why this resists a machine in the middle
A party in the middle terminates the machine's TLS with its own certificate `C_m` and relays to the hub over a second connection. The machine's
`leaf_hash` is then `H(C_m)` and the hub's is `H(C_hub)`: the two numbers differ except by chance. The attacker sees the machine's *commitment*
before it chooses anything toward the machine, and the machine's nonce and CSR are revealed only after the attacker has fixed its own nonce toward
the machine, so it cannot grind a collision after the fact: it has one attempt for each pairing, a 1 in 10^6 chance. A failed attempt shows up as two
different numbers on two screens, the owner presses Deny, and it counts toward the lockout. The hub is the owner's own machine, so the hub's nonce
needs no commitment.

## 6. A browser signing in
- A browser has no key and no TLS binding it can read, so the number here is **not** a handshake: the hub makes a random 6-digit number, shows it
  on the sign-in page ("Waiting for the owner to approve. Number 731-004") and on the owner's card ("Browser 'Chrome on Mac' (192.168.1.50) wants
  to sign in. Number 731-004"). Its only job is to let the owner tell two requests apart and to know it is the one they mean.
- The request is unauthenticated by nature, so it is the part to bound hardest: at most **4** pending browser requests for the whole hub and **2**
  per address, each lives **3 minutes**, a new one from an address with a pending one replaces it, and a window gets the card once (no repeated
  notifications). It works only for an owner who is signed in somewhere to approve it; a hub with no signed-in window shows "ask the owner".
- On approval the hub makes a session for the polling browser (the poll carries a token the page got with the number and keeps; the session
  cookie is returned to that poll only). Anything else about sessions is as today (ADR-0049, #450).
- Signing in a stranger by approving the wrong card is the one risk this cannot remove: the card names the browser and the address, and approving is
  the owner's decision, never the hub's.

## 7. Limits
- Pending machine sessions: at most **4**; from one address at most **3 requests a minute** and **20 in 10 minutes**.
- A session lives **5 minutes** from the reveal; a reveal must arrive within **30 seconds** of the challenge.
- Failures (commitment mismatch, a Deny, an expiry nobody answered) count toward the existing lockout (`enroll::Lockout`: 5 failures, a minute
  of lockout for that address).
- One connection carries one pairing; the hub closes it after the result and after 30 seconds without a message.
- The state of every session is memory only and ends with its session, with the switch closing, or with the hub restarting. Nothing about a
  pairing is stored except the audit trail and, on approval, the certificate record enrolment already keeps.

## 8. Audit and what is never logged
Audited: the switch opened and closed (by whom); a request (name, os, address); approve or deny (the owner's device); the certificate issued (serial).
Never logged: nonces, the commitment, the number, the CSR.

## 9. Threat-model rows to add when this is built
- **T23, a machine or browser on the network asks to join or sign in without the owner meaning it:** only while open; bounded, short-lived sessions;
  an owner decision for each; the lockout.
- **T24, a party in the middle of a pairing:** the commitment, the number bound to the certificate each side saw, one attempt in 10^6, visible to the owner.
- **T25, discovery used to find or flood hubs:** a closed hub is silent; a request must be as large as the reply; replies are rate limited and
  unicast; the answer holds only public data.

## 10. Order of work (each its own pull request, each security-reviewed)
1. The messages, error codes and golden fixtures (append-only), and this document updated with the ids.
2. The pairing core in the hub, pure and unit-tested (states, limits, SAS, commitment), `agora pair open|close|status`, audit.
3. The command path for approve/deny and `PairRequested` to windows and notifiers; the machine handshake over TLS.
4. The discovery responder (hub) and the finder, `agora join`, with the trust question (node).
5. The window cards wired in (web #610 and TUI #611 already have the layout), then the browser sign-in by approval (section 6).

## 11. Open points for the reviewer
- Is `leaf_hash` the right thing to bind, or should it be the exporter value of the TLS session (RFC 5705)? The hash is simpler and works with
  the library already in the tree; the exporter would also bind the session, but needs support in the client stack.
- The machine's verifier accepting any certificate: the other option is a hub certificate the machine recognises by the discovered
  `ca_fingerprint`, but a rogue responder can announce any fingerprint, so it would add nothing the number does not already give.
