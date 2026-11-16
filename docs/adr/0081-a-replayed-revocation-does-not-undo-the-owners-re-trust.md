# 0081. A replayed revocation does not undo the owner's re-trust
Status: Accepted
Date: 2026-11-16
Accepted: 2026-11-17 (the maintainer's own reply in the merger chat, "1 kabul et geç", to the merger's list that named ADR-0081, ADR-0082, ADR-0083 and ADR-0084 as the decisions waiting for them). The open questions' proposed answers stand.
Origin: evidence audit finding F220 (device trust). A proof run against `main` printed `PROBLEM CONFIRMED`: a revocation dated ahead of the
machine's clock, sent again by the hub after the owner trusted the device again on the machine, removed the device again.

## Context
- ADR-0061 and "Devices" in `docs/specs/protocol.md` promise: "A revocation sent again later never removes trust given after it (an
  endorsement signed later, or the owner trusting it again on the machine with `agora node trust ID KEY`)." The hub resends every revocation
  it keeps each time a node connects, for 400 days.
- The node (`agora-node::device_trust`) keeps this promise by dates. Each trusted line records when its trust began (`since`); a
  revocation removes the line when `since <= issued_at_ms` (`revoke_locked`). `confirm` (`agora node trust`, the yes at `agora node
  enroll` and `--from-hub`) sets `since` to the machine's clock and forgets the device's remembered revocation.
- `issued_at_ms` is the revoker's own claim, and a revocation up to 5 minutes ahead of the node's clock is accepted. A browser whose clock
  runs 4 minutes fast revokes a device at 12:00 (dated 12:04). The owner trusts it again on the machine at 12:01 (`since` = 12:01). At the
  next reconnect the hub sends the revocation again; 12:01 <= 12:04, so the device is removed again. The proof:
  `EXPECTED: replay after the owner's confirmation removed=false still_trusted=true`,
  `ACTUAL:   replay after the owner's confirmation removed=true still_trusted=false`.
  Commit 5a2cfa95 fixed a flaky test that hit this exact case with a 1 ms skew by changing the test, not the code.
- Impact: it only takes trust away (availability; the owner's decision is undone), never adds it. But the hub chooses when, and the owner
  sees a device they just trusted disappear with nothing to explain it.
- The endorsement path does not have the problem: an endorsement whose start is at or before a remembered revocation of the device is
  refused, so a re-trust by endorsement always starts after the newest revocation the node has seen. Only the owner's confirmation can
  start before a revocation the node already received.
- A date fix alone cannot work. The node cannot tell, from the date, a revocation it received **before** the owner's confirmation (a replay)
  from one signed **after** it (a new decision): both can carry a date later than the confirmation. Endorsement dates are capped at the node's
  clock when they arrive (`began = min(issued, now)`); capping revocation dates the same way changes nothing here, because the replay
  arrives after its claimed date and is judged again then.

## Decision (proposed; option (b) of the audit)
1. **The owner's confirmation overrides the revocations of that device the machine had received, by identity, not by date.** A revocation
   is identified by what its revoker signed: `(device_id, revoker_device_id, issued_at_ms)`, the fields of `revoke_message`. Not by the
   signature: ES256 signatures are randomized, a second valid signature over the same bytes can be derived from the first without the
   key (s to n - s), and the hub stores "each signature once", so one decision can arrive under several signatures. The same revoker
   signing the same device at the same millisecond is the same decision.
2. **Only a revocation dated ahead of the node's clock when it arrives needs remembering by identity.** A confirmation made after it
   arrived has `since` later than its arrival; only a revocation dated after its own arrival can be dated after that `since`. Every other
   one is already handled by the `since` rule. So when a verified revocation of a trusted signer arrives with `issued_at_ms > now`, the node
   keeps a line for it in a new file, `<data>/device-revocations-ahead`:
   `<device id> <revoker id> <issued ms>`. Such a line is useless once the clock passes `<issued ms>` and is dropped at the next write.
3. **`confirm` marks them overridden.** Under the existing lock, every line of that device with `<issued ms>` at or after the new `since`
   gets ` overridden=<since>`. When a revocation arrives whose identity matches a line marked `overridden=<S>`, and the device's trusted line
   still has `since=<S>`, the revocation does not remove it: `revoke` returns `Ok(false)` (nothing changes for the hub, which already ignores
   the result) and the node's own log records the outcome `overridden`. An overridden line stays as long as the device's trusted line keeps
   that `since`; once the device is untrusted, revoked, replaced under a new key, or confirmed again (which recomputes the marks with the new
   `since`), the line is dropped at the next write.
4. **A new revocation still applies.** A revocation signed after the re-trust has another identity (a later date, or another revoker), so
   the `since` rule applies to it as today. So does one the machine never received before the confirmation (for example, the node was offline
   when it was signed): the owner overrode only what the machine had seen. A revocation signed by a device the machine no longer trusts
   still fails at the signer check, as today.
5. **`device-revocations` is unchanged in format, and `confirm` no longer deletes the device's line from it.** Overriding a revocation keeps
   the device trusted; it does not lift the rule that endorsements of it and by it, signed at or before the revocation, stay refused (the
   spec's "from then on", and T21's "an endorsement the hub kept from before its endorser was revoked does not land after the endorser is
   trusted again"). Today that rule lapses after `confirm` until the hub happens to replay the revocation; this was seen while reading the
   code for F220 and is not proven by a test.
6. **Bounds.** `device-revocations-ahead` holds at most 256 lines (the existing `MAX_REVOCATIONS`), dropping the earliest-dated first. A
   pending line lives at most 5 minutes. A dropped line fails closed: the replay of that revocation may remove the device again, which is
   today's behaviour. Filling the file takes many revocations signed by a device the machine trusts, and that device could remove the
   device again with a fresh revocation anyway; the owner untrusting that signer makes all its revocations fail the signer check.
7. **Compatibility.** Both files are node-local and owner-only, written atomically under the existing lock file.
   - An older node never reads `device-revocations-ahead` and reads `device-revocations` as before: after a downgrade, the replay can undo a
     re-trust again (today's behaviour, narrowing only).
   - A newer node starting without the file behaves as today until a revocation dated ahead arrives. Stale `overridden=` lines left by a
     downgrade and upgrade are dropped when the device's `since` no longer matches them.
   - No protocol message, field, capability or error code changes. The hub keeps replaying as it does.
8. **What the owner sees.** When a confirmation overrides at least one revocation, `agora node trust` (and the enroll and `--from-hub`
   paths) prints one more line, a new key in `locales/en/cli.ftl` and `locales/tr/cli.ftl` with the device id and the revoker id as
   parameters (for example: "Device web-new was revoked from web-old a moment ago. It stays trusted here even when that revocation is sent
   again."). The node's log gets the outcome code `overridden`. The window changes nothing: it already shows which machines trust a device.

Out of scope, considered: the check that refuses an endorsement signed by a device revoked after it compares two signer-claimed dates
(`issued_at_ms` of each), so a fast clock on either side shifts the boundary by at most 5 minutes; it only matters once the endorser is
trusted again, and it never lets the hub add trust. A revoker whose clock is behind dates its revocation before a recent confirmation, and
the `since` rule then ignores it; that is the signer's claim as the spec defines it. Neither is changed here.

## Consequences
- The spec's promise holds under every accepted skew. The audit's F220 proof becomes a
  regression test in `device_trust.rs`, with cases for: a new revocation by the same revoker after the re-trust (applies), one by another
  revoker inside the 5 minutes (applies), one never received before the confirmation (applies), an overridden replay (does not), a full
  file (fails closed), a missing or old-format file, and the endorsement block still holding after a confirmation.
- One new node-local file and one change to `confirm`. No schema change, no hub change.
- `docs/specs/protocol.md` ("Revocation") and T21 in `docs/security/threat-model.md` say how a re-trust overrides a revocation the machine
  had received, in the implementation PR. One new CLI line in en and tr.
- Cost: one more small file read and written on revocation and confirmation only; neither is a hot path.

## Alternatives considered
- **(a) `confirm` sets `since` just after the newest remembered revocation of the device.** One-line change, no new file. Fails open: a
  real revocation signed by another device within 5 minutes after the confirmation, dated before that moved `since`, is ignored. Rejected.
- **(c) Refuse, or cap at the node's clock, revocations dated ahead.** Refusing fails open for every browser whose clock is fast (its
  revocation never applies, and the owner thinks it did). Capping does not fix the replay, which arrives after the claimed date. Rejected.
- **Identify a revocation by its signature or a hash of it.** Signatures are randomized and can be changed into another valid one without
  the key, so the hub could present the same decision as new. Rejected for the signed fields.
- **Store the identity of every revocation, or extend the lines of `device-revocations`.** Every other revocation is handled by the date
  rule, and an older node parses a longer line as invalid and skips it, losing that revocation on downgrade (fails open). Rejected for a
  separate file holding only revocations dated ahead.
- **Do nothing and document it.** The owner's on-machine decision is the strongest one in ADR-0061; the hub undoing it, even only within 5
  minutes of skew, contradicts the spec. Kept as the fallback if the maintainer prefers no new state.

## Open questions (for the maintainer)
1. Is (b), as above, the right trade-off against (a)'s simplicity? Proposed: yes; (a) can ignore a real revocation.
2. Should `confirm` keep the device's line in `device-revocations` (point 5)? Proposed: yes; it is what T21 already says.
3. Should the CLI say anything (point 8)? Proposed: yes, one line, only when a revocation was overridden.

## Built
Built in `agora-node::device_trust` (the `device-revocations-ahead` store, the overrides in `confirm` and `revoke`), `agora` (`trustcli`, one CLI line in en and tr), `docs/specs/protocol.md` and T21. `confirm` writes the overridden marks before it trusts the device, so a failed write leaves the old state. The CLI line does not say "a moment ago" (a revocation can be dated up to 5 minutes ahead). F220 is a regression test in `device_trust.rs`.
