# 0085. Machine and hub certificates renew themselves
Status: Accepted
Date: 2026-11-17

Accepted: 2026-11-19 (the maintainer's explicit message in the merger chat naming this ADR: "ADR-0085'i kabul ediyorum"). The open questions' proposed answers stand.

Build progress: schema and hub side done (`RenewRequest`/`RenewResponse`, `enroll.renew`, `enroll.renew_refused`; the hub answers a renewal on a certificate connection,
audits it, revokes superseded certificates after 7 days, and replaces its own server certificate daily when under 30 days remain). Review changes: only a machine's newest certificate may renew (`superseded` otherwise); refused renewals are audited, rate-limited to one row per machine per reason per hour.
What remains: a thief with the machine's CURRENT certificate and key is indistinguishable from the machine until the owner notices (the audit view shows `superseded`/`too_frequent` signals); revocation (ADR-0015, ADR-0073) is the answer. A machine that lost a renewal's answer asks again with the same request and the hub sends the same certificate again (idempotent: same serial, nothing issued or revoked, audit `enroll.renew_resent`, at most 5 an hour); this works only for the request whose key the certificate was issued for, so a thief with an older certificate gets nothing, and not after the newest certificate has connected. The node keeps its fresh key and request until it has stored the answer (built, step 2). The columns for this are part of schema 13, which is unreleased, so the migration was amended, not extended.

Step 2, the node side, is built: the node renews at a third of the certificate's own life (checked at start and hourly) when the hub's welcome lists `enroll.renew`; a fresh key and its request are persisted (`renew-pending.pem`) before they are sent and kept until the answer is stored; an answer is verified (chain to the pinned CA, same node id, client authentication only, sane end, the request's key) before the pair in use is replaced; the install order and its recovery are in `docs/specs/protocol.md` ("The node side"); the next connection uses the new pair and the old one is given up after one accepted connection. Refusals are never final (a hub can be at fault: a database restored from a backup, a wrong clock): `unknown`/`revoked`/`expired`/`too_old` are remembered (`agora doctor`: join again, the reason, when last asked) and asked again about once a day, `superseded` likewise and logged as security relevant, a success clears them, `too_frequent` waits an hour, anything else backs off; whether the certificate has ended is never written down. Review of #783 changed: a failed key write rolls back to the previous pair, link saves clear the renewal files, a pending request is bound to its certificate, five rejected answers give the request up. `agora doctor` shows the days left of this machine's and the hub's certificate and what a refused renewal needs. The look is hourly; a failed ask backs off from a minute to an hour, and a hub refusal rests about a day. Known and accepted: a node clock far ahead (say 70 days) but still before the certificate's end sees the certificate as due and renews at most once an hour, which the hub's own limit of one certificate per machine per hour bounds; a clock past the end is `Ended` and asks nothing.
Not built yet: the window's "needs to join again" card (the hub's view has no field for it: see the PR that adds the card), and a hub that tells a revoked or unknown certificate why (today such a certificate is refused at the TLS connection, so the node sees only a failed connection and retries slowly; `agora hub machines` on the hub shows it).

Origin: an internal audit. `crates/agora-hub/src/ca.rs` promised "renewal is automatic while enrolled"; reading the code and then running it
(`crates/agora/tests/cert_lifetime.rs`) showed that nothing renews anything.

## Context
- Every certificate the hub's CA issues lasts 90 days (`DEVICE_CERT_DAYS`): a machine's client certificate (ADR-0015, ADR-0027) and the hub's
  own server certificate.
- A machine gets its certificate once, at enrollment, and stores it as `<link dir>/cert.pem`. No message, command or timer asks for a new one.
  `enroll.proto` has only `EnrollRequest` with a one-time code.
- The hub issues its server certificate in memory each time `hub run` starts. A hub that stays up (the intended case: a supervised service)
  keeps serving the same certificate until it ends.
- Proven by `proof_a_machine_whose_certificate_ended_cannot_connect_and_nothing_renews_it` and
  `proof_a_hub_whose_own_certificate_ended_is_refused_by_new_connections`: with a 3-second lifetime, a machine dialing after the end is
  refused by the hub, and machines dialing a hub whose certificate ended refuse it; a restarted hub is accepted again. A connection that is
  already open is not cut at the end (TLS checks validity at the handshake).
- Consequence if left as is: a machine enrolled on day 0 stops connecting after day 90 and the owner must join it again by hand, all machines
  at about the same time. Agents keep running (nodes are autonomous) and events wait in the buffer, but the machine disappears from every window.
- Interim, shipped with this proposal (it changes no behaviour): `agora doctor` and `agora hub machines` warn 30 days before the end, and
  CHANGELOG tells people to join machines again before day 90. That is a warning, not a fix.

## Options

### A. Renew over the existing authenticated link (recommended)
A new, append-only message pair `RenewRequest { csr_pem }` / `RenewResponse { cert_pem, not_after_ms }` on the commands stream of the
already-authenticated mTLS connection (new capability `enroll.renew`; old hubs and nodes never see it). The node generates a fresh key pair
and sends its CSR; the hub checks that the connection's certificate is the machine's current, unrevoked one, issues a certificate for that same
node id, records the new serial in `devices`, and revokes nothing yet (see grace). The node writes the new `cert.pem` and `key.pem`
atomically (write to a temporary file, then rename) and uses them on its next dial; the open connection stays.
A node asks when fewer than a third of the lifetime remains (30 days of 90) and retries daily with backoff, so an outage of the hub of days is survived.
- Pro: no human step, no new trust root, the key never leaves the machine, one small schema addition.
- Con: a stolen certificate and key can renew itself forever until revoked. Mitigation below.

### B. A longer lifetime (a year)
One constant. Fewer events, but it turns a 90-day outage into a 365-day one, makes a stolen key useful for longer, and still ends silently.
Not recommended as the only answer; acceptable as a complement once A exists (A makes the lifetime a tuning knob).

### C. Re-enroll by hand
What happens today. Kept only as the fallback for a machine that was offline past its end.

## Decision
1. **Option A** for machine certificates: `RenewRequest` / `RenewResponse` as above, capability `enroll.renew`, schema added in `schema/agora/v1/enroll.proto`
   with a golden fixture (append-only; nothing existing changes).
2. **The hub's server certificate is replaced without a restart.** The hub keeps the acceptor behind a swappable handle (`ArcSwap`-style with `std`
   `RwLock<Arc<..>>`, no dependency) and issues a new server certificate when fewer than 30 days remain, checked daily and at start. New connections
   use the new one; open ones are untouched. It also writes `<hub dir>/server-cert.pem` so `agora doctor` can read it.
3. **Grace.** The previous certificate stays valid until its own end (not revoked at renewal), so a node that wrote the new file but crashed before
   using it still connects. The hub revokes a machine's older serials 7 days after it first connects with a newer one.
4. **A machine offline past its end** cannot renew (it cannot authenticate). It joins again with the owner's approval through pairing (ADR-0065)
   or a join code (ADR-0027); enrolling again under the same name already replaces the old certificate. The window shows such a machine as "needs to
   join again" with that action; the machine's agents and buffered events are untouched.
5. **Renewal never widens anything.** The new certificate carries the same node id and the same usages (client authentication only). The hub
   ignores any name or extension in the CSR, exactly as at enrollment. Revoking a machine (ADR-0073) revokes its renewals too; a revoked certificate
   cannot renew.
6. **How the owner sees it.** Nothing, when it works. `agora doctor` shows the days left on this machine's certificate and the hub's; the window's machine
   card says "Needs to join again" only for a machine that missed its renewal; the audit log records each renewal (serial, node id, time).

## Threat-model impact
- T3 (rogue enrollment): renewal is not enrollment; it needs a live certificate that the hub still lists as active. A stolen certificate and key
  renew themselves until revoked, which is also true today for the 90 days; revocation (ADR-0015, ADR-0073) remains the answer and is checked on
  every connection (10 s). To limit exposure the hub refuses a renewal from a certificate older than its lifetime and a renewal request more than once an hour.
- A compromised hub already issues any certificate it likes (it is the CA): renewal gives it nothing new.
- Nodes pin the CA, not the server certificate, so replacing the hub's server certificate needs no change on any machine.

## Open questions, with the recommended answer
- Renew at one third of the lifetime remaining? Yes (30 of 90 days).
- Same key or a fresh key at renewal? A fresh key each time (limits the life of a leaked key).
- Keep 90 days as the lifetime? Yes at first; revisit after renewal has run for a release.
- Revoke the old certificate at renewal? No: 7 days of grace, then yes (point 3).
- Rate limit for renewals? One per hour per machine.
- Does the hub tell a machine its certificate is about to end if it has not renewed? Yes: a notice event (`doctor` text only on the machine; the window shows it in the machine's card).

## Tests
- The two existing proofs (`crates/agora/tests/cert_lifetime.rs`) become the starting point: with renewal, a 3-second-lifetime machine connected for
  longer than its lifetime still connects after a redial; a machine offline past the end is refused until it joins again.
- Hub tests: renewal refused for a revoked, unknown, expired or too-frequent request; the new certificate has the same node id and usages; the CSR's
  name is ignored; a golden fixture for each new message.
- Hot reload: a server certificate with a 3-second lifetime is replaced before its end and a new connection after the old end succeeds with no restart.
- Cross-platform: the atomic file replace on Windows (rename over an existing file) is tested in `agora-platform`.

## Consequences
- Adds one schema message pair and a capability; no new dependency.
- Until it is accepted and built, machines must be joined again before day 90 (CHANGELOG, `docs/security/threat-model.md`).

## Amendment A (Accepted, 2026-11-20): a node whose clock is behind keeps its answered request, and accepts a certificate that starts slightly in its future
Accepted: 2026-11-20 (the maintainer's explicit message in the merger chat: "ADR-0094 ve ADR-0085 Ek A'yı kabul ediyorum"). The open questions' proposed answers stand.
Built 2026-11-26 on `main` (decisions 1 to 5). Not built: the window's machine card (open question 2, waits for the "needs to join again"
card). The rest of ADR-0085 stays Accepted and unchanged.

Origin: evidence audit round 10, finding F1340 (#816 describes the proofs). Two Accepted rules of this ADR, each sound alone, contradict each other
when the machine's clock is behind the hub's:
- **Rule 1, node (step 2, review of #783):** "five rejected answers give the request up". An answer is used only when it "chains to the pinned CA, is
  valid now" (`docs/specs/protocol.md`, "The node side"). "Now" is the node's clock, with 10 minutes of slack.
- **Rule 2, hub (review changes):** only the machine's newest certificate may renew; an older one is answered again only for the same request, and
  otherwise gets `superseded`.

### What happens today
1. The hub issues the new certificate B with a start of hub-now minus 5 minutes.
2. A node whose clock is more than about 15 minutes behind sees B as "not yet valid" and rejects the answer (`certificate`). It keeps the request and
   asks again an hour later; the hub correctly resends the same B.
3. While the gap is under about 4.5 hours, one of the hourly asks lands after B's start on the node's clock and renewal succeeds. Above that, the
   fifth rejection comes first: the node deletes `renew-pending.pem`, which holds **B's only private key**.
4. The next due look makes a new key and request. The hub has B as the machine's newest certificate (never connected), so the request from the
   current certificate A is `superseded`, every day, also after the clock is fixed. The node logs it as security relevant ("another holder may
   exist"), which is false.
5. When A ends, the machine must join again by hand. This is exactly the 90-day cliff ADR-0085 exists to remove.

Realistic triggers: a hardware clock kept in local time west of UTC (dual-boot Windows/Linux), a VM restored from a snapshot, a board without an RTC
before NTP syncs, NTP blocked on a locked-down network. The same dead end is also reached by losing `renew-pending.pem` after the hub issued (a
damaged disk); that case is not addressed here (joining again stays the fallback, Decision 4).

Both proofs fail on `main` today: the node side with an injected node clock 6 hours behind (`[Failed x5, Refused("superseded") x7]`, never renewed),
and the hub side against the real `Enrollment::renew` (25 daily new requests, 25 `superseded`).

### Options
- **A. The node keeps a request whose answer was issued for it.** Give a request up only when the answer can never be right for it: another node id,
  another key, the wrong usage, or a chain that does not reach the pinned CA. A rejection for **time** (not yet valid) keeps the request and keeps
  asking hourly; once the clock is right, the resent B is accepted. Alone, this heals only after the clock is fixed.
- **B. The node accepts a start in its own future, bounded.** A renewal answer whose start is at most `RENEW_FUTURE_START` (proposed: 24 hours)
  ahead of the node's clock is accepted when everything else checks out; the chain is checked at the certificate's own start instead of the node's
  now. The node can store it at once because it is not the node that judges its own client certificate: the hub checks validity on its own clock at
  every TLS handshake. Alone, this heals any gap under 24 hours without anyone fixing the clock, but a larger gap still runs into rule 1.
- **C. The hub issues a new certificate to an older one when the newest has never connected.** Rejected: it reopens what the "only the newest may
  renew" review closed. A thief holding the old certificate and key could then renew while the machine's real new certificate is unused (for
  example, the machine is offline), which is exactly the case rule 2 guards.

### Decision (proposed)
1. **A and B together.** B makes the common cases (a few hours of timezone or snapshot drift) work at once with no human step; A guarantees that a
   larger gap never destroys the key, so renewal succeeds as soon as the clock is right, before A ends.
2. **What gives a request up** (replaces "five rejected answers give the request up"): five answers in a row rejected for `node`, `key`, `usage` or
   `chain`. Rejections for `validity` (start beyond the bound, or already ended) and `not_newer` never give it up; they keep the request, rest an hour
   and ask again. A request is still given up when its certificate is replaced by a new link (`agora join`), as today. Today one reason,
   `certificate`, covers "not a certificate, not chained to the pinned CA, not valid now, or too long"; it is split into `chain` (not a certificate,
   not chained, longer than a year) and `validity` (time). Both are stable words shown only as a parameter (`renew-state`, `agora doctor`); a
   remembered `certificate` from an older version is read as `chain`.
3. **The future-start bound is 24 hours** and applies only to renewal answers (`RenewResponse`). Pairing and enrollment keep "valid now" with
   10 minutes of slack: a node that is joining has no certificate the hub has already seen, so nothing is lost by refusing there.
4. **The answer is still checked whole:** same node id, the request's key, client authentication only, at most a year long, newer than the
   certificate in use, chained to the pinned CA. Only the "has it started" test is relaxed, by the bound.
5. **What the owner sees.** `agora doctor` already shows a remembered rejection; a `validity` rejection additionally says "this machine's clock may
   be wrong" (new Fluent text, en and tr). The `superseded` "security relevant" log stays for `superseded` itself; with decision 2 a clock gap no
   longer produces it.

### Threat-model impact
- No new power for anyone. The hub is the CA and could already issue any certificate; accepting one that starts up to 24 hours in the node's future
  gives a malicious hub nothing it could not do with a certificate that starts now.
- Keeping a request longer keeps its private key on disk longer (`renew-pending.pem`, written private). That key is only ever good for the
  certificate the hub issued for it, which the hub already holds; it is the same exposure as the key in `key.pem`.
- Rule 2 is unchanged, so T3 is unchanged: an older copy of the certificate still gains nothing.

### Changes if accepted
- `crates/agora-node/src/link_renewal.rs`: the give-up count only for `node`/`key`/`usage`/`chain`.
- `crates/agora-node/src/cert_renew.rs`: `Rejected::Certificate` split into `Chain` and `Validity`.
- `crates/agora-node/src/cert_renew.rs`: `check_renewed` takes the bound; the chain is verified at `max(now, not_before)` when `not_before` is within
  it. `pairing_client::check_client_certificate` keeps its current behaviour for pairing.
- `docs/specs/protocol.md` ("The node side"): "valid now" becomes "not ended, and started or starting within 24 hours of the node's clock";
  the give-up rule as in decision 2. `docs/security/threat-model.md` T3: one sentence on the bound.
- `locales/{en,tr}/cli.ftl`: the doctor hint for a `validity` rejection.
- Tests: the two F1340 proofs become regression tests (6 hours behind renews at once; 30 hours behind keeps the request, then renews once the
  clock is set right; the hub side unchanged); a `key` rejection still gives up after five; pairing still refuses a future start.
- No schema change, no new message, no new dependency.

### Open questions for the maintainer
- Is 24 hours the right bound? (Proposed: yes. It covers every timezone offset (at most 14 hours) with room; a larger gap is a broken clock that A
  still survives.)
- Should a `validity` rejection also show in the window's machine card? (Proposed: not now; `agora doctor` on the machine is enough until the
  "needs to join again" card exists.)
