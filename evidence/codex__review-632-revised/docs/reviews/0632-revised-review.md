# PR #632 revised design review

Reviewed head: `b2281e204aa3bb76ec3852140c5a37c8015372ed`.
This is review of a proposed protocol design, not implemented authentication or a formal security proof.

Both original P2 findings are addressed. Browser continuation, cancellation and replacement require the poll capability; tokenless callers cannot replace a pending request. Admission/poll budgets, notification coalescing and bounded keyed counters are now explicit. The default LAN certificate problem is addressed by provisioning the actual listen-interface addresses and hostname before opening pairing, while retaining mandatory name/chain checks. Implementation acceptance tests remain required.

Exporter plus leaf, TLS 1.3 only, genuine CertificateVerify verification, immutable connection/transcript ownership, single-use decisions and the proposed rate budgets are reasonable design choices. Keep exporter mandatory for supported clients: section 11's wording about dropping it should not create an unreviewed fallback. No concrete offline nonce-grinding break was found; this does not certify the custom construction. The aggregate success estimate assumes the specified fresh one-shot exchanges and failure counting actually hold.

Two P2 lifecycle contradictions remain:

1. Section 6 invalidates the token **on approval** (line 138), yet requires a subsequent poll to redeem it for the cookie (157–158). Approval between polls therefore has no defined delivery path. Specify `Pending -> ApprovedAwaitingDelivery -> Consumed`, preserving the capability only for one atomic delivery until the original deadline; approval forbids cancellation/replacement. Deny/cancel/expiry/close/restart invalidate immediately. Include approval with no outstanding poll and concurrent redemption controls.
2. Section 7 promises five minutes after reveal but also closes the owning connection after 30 seconds without a message (176). Approval at 45 seconds fails under that rule. Scope the 30-second timeout to awaiting reveal, and define pending transport liveness without extending the fixed approval deadline. Include an approve-at-45-seconds control.

Also clarify that section 5a's same-connection restriction applies to **machine** messages; owner-authenticated approve/deny commands must be accepted from owner connections.

`0632-revised-state-traces.py` is an executable specification counterexample and proposed-state control. It is deliberately not represented as a real server reproduction. It also demonstrates the revised zero-time 10,000-attempt shared-IP churn bound: victim capability preserved, two total admissions due to the per-address pending cap, one notice.

Validation: `sh scripts/check-docs.sh` passed at the reviewed head. The Python traces passed. No Rust suite or network pairing tests were run for this documentation-only review.

Verdict: original findings resolved; **DESIGN CHANGES NEEDED** for the two lifecycle clarifications. Concrete replacement semantics were sent directly on comms; no further maintainer choice is needed. Implementation remains a separate security review.
