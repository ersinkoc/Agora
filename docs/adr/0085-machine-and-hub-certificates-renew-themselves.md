# 0085. Machine and hub certificates renew themselves
Status: Accepted
Date: 2026-11-17

Accepted: 2026-11-19 (the maintainer's explicit message in the merger chat naming this ADR: "ADR-0085'i kabul ediyorum"). The open questions' proposed answers stand.

Build progress: schema and hub side done (`RenewRequest`/`RenewResponse`, `enroll.renew`, `enroll.renew_refused`; the hub answers a renewal on a certificate connection,
audits it, revokes superseded certificates after 7 days, and replaces its own server certificate daily when under 30 days remain). Not built yet: the node asking
(30 days left, daily retry, atomic write of `cert.pem` and `key.pem`), `agora doctor` days left, the window's "needs to join again" card, and the `agora-platform` atomic-replace test.

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
