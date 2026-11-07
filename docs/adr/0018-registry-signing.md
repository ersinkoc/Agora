# 0018. Adapter registry and signing
Status: Accepted
Date: 2026-11-06
Accepted: 2026-11-07

## Decision
- The registry is a static, versioned index (served over HTTPS, mirrorable) listing manifests and adapter packages.
- Every package is signed with an Ed25519 publisher key; the index pins publisher keys (trust on first use,
  with an official key for first-party packages).
- Hubs can run a private registry and allowlist publishers.
- Nodes verify signatures before installing or running adapters; adapters run with the node's user privileges,
  so installing third-party adapters requires explicit user approval.

## Consequences
New agent support ships without a binary release, and a compromised mirror cannot inject code.

## Additions
- Publisher keys can be rotated and revoked; the index carries a signed revocation list.
- The index is versioned and signed with expiry, and nodes reject an index older than the one they last saw (rollback protection).
