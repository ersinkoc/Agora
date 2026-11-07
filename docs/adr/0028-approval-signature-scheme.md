# 0028. Approval signatures: ECDSA P-256
Status: Accepted
Date: 2026-11-07
Accepted: 2026-11-07 (maintainer's explicit approval in chat)

## Context
ADR-0020 requires user-signed approvals verified on the node; ADR-0015 left open where the browser holds the signing key.
`agora-node::approvals` has a `Verifier` trait but no scheme.

## Decision
- Approval decisions are signed with **ECDSA over P-256 with SHA-256** (ES256), the algorithm WebAuthn, WebCrypto and
  secure enclaves (iOS, Android, TPM) all support.
- The signed message is `agora-approval-v1 NUL node_id NUL approval_id NUL verdict` (`approvals::signed_message`).
- Signature encoding on the wire: fixed 64 bytes (r || s). Public keys: uncompressed SEC1 point (65 bytes).
- Trusted device keys reach a node only through a user-signed change (ADR-0020); until then the node trusts none and denies.
- Whether the browser key is a WebAuthn credential or a non-extractable WebCrypto key stays a client decision; both produce ES256.

## Consequences
One verification routine on the node covers browser, desktop and mobile. Ed25519 stays the scheme for registry signatures (ADR-0018).
