# PR #609: tunnel Host enforcement cross-review

Reviewed head: `7e649632e1e58d87c1254891bca6e7d8f869d8c4`.
Verdict: **APPROVE** for the stated browser DNS-rebinding mitigation. No blocking defect found.

## Transport evidence

The review-only tests call the real `open` path over loopback TCP and TLS, with a generated fixture CA and recording TLS upstream. No external hub, agent, account or credential is involved. The upstream counts TCP accepts before its TLS handshake; rejected heads must leave that count at zero. A valid request to the same listener then proves the counter/listener are live.

All three added transport proofs pass:

- **14 rejected heads** receive exactly the empty-body 421 and produce zero upstream TCP accepts: attacker name, trailing-dot localhost, another loopback IP, IPv6 spelling, wrong/missing port, missing Host, duplicate same/mixed Host, whitespace before the colon, a folded fake Host without a valid one, absolute-form target with a good Host, CONNECT authority-form target, and invalid UTF-8. A valid TLS control afterward returns the expected response, preserves the request exactly, and increments the count to one.
- **Byte preservation and limit:** a body containing `0xff`, NUL and CRLF passes unchanged; an exactly 16 KiB head passes unchanged; a 16 KiB + 1 head closes without dialing upstream. A first valid request plus a pipelined second request with another Host passes unchanged, confirming the documented first-request-only boundary rather than hiding it.
- **Timeout and independence:** an incomplete head receives no response and expires near 10 seconds without dialing upstream; a valid parallel request completes in under 3 seconds and is the only upstream accept.

The deliberate negative control bypasses only the Host guard, temporarily and against the fake loopback upstream. The first attacker-name case then reaches that upstream and receives 200 instead of 421; the regression fails immediately (exit 101). The source is restored in `finally` for both modes. This shows the rejection proof depends on actual enforcement, rather than an unreachable test listener.

## Existing coverage and checks

- Existing tunnel suite: **11 passed**, none failed or ignored, including the split-header, wrong-CA, IP/localhost certificate-name and post-upgrade bidirectional-byte tests.
- Added review transport suite: **3 passed**. Negative control: **1 expected failure**.
- Formatting, docs checker and all-target tunnel Clippy with warnings denied pass on restored product source.
- Reported required GitHub checks pass at this head, including native Rust and desktop shell CI on Linux/macOS/Windows. Local execution is Linux; no native Windows/macOS runtime or built-webview click/navigation test is claimed.
- TLS verifier/CA behavior and the post-initial-request `copy_bidirectional` remain unchanged. The new work is per-connection header setup, bounded by the head-size and timeout checks; it is not added to terminal/frame byte handling. No new dependency or public API change.

## Threat-model boundary

The first-request-only decision is intentional and tested. The [Fetch connection-pool algorithm](https://fetch.spec.whatwg.org/#concept-connection-pool) keys reuse by origin as well as partition and credentials; this supports the stated assumption for a direct browser HTTP connection. A handcrafted local program can send an accepted Host, and can pipeline another afterward. That remains outside the protection provided here and is explicitly retained in T22. No claim is made that this authenticates a local process or fixes T17's loopback sign-in exemption.

Rejecting absolute-form is appropriate to this direct webview tunnel: [RFC 9112 section 3.2.2](https://www.rfc-editor.org/rfc/rfc9112.html#section-3.2.2) gives the request target's authority precedence over Host. Treating it as a path would invalidate a Host-only decision. The parser is not a general HTTP validator; downstream HTTP parsing remains the server's responsibility. Browser-level DNS rebinding was not executed here; these are actual transport/parser rejection and preservation proofs.

## Reproduction

```sh
cargo test -p agora-desktop-tunnel
python3 docs/reviews/0609-run-transport-proof.py
# Deliberate bypass control: expect exit 101 on the first wrong-host case.
python3 docs/reviews/0609-run-transport-proof.py --bypass-guard-control
git diff --exit-code -- crates/agora-desktop-tunnel/src/lib.rs
```

Only review evidence is committed. Production source and existing tests are byte-for-byte restored after instrumentation. No merge is performed by this reviewer.
