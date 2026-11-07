# 0018. Adapter registry and signing
Status: Proposed
Date: 2026-11-06

## Decision (proposed)
- The registry is a static, versioned index (served over HTTPS, mirrorable) listing manifests and adapter packages.
- Every package is signed with an Ed25519 publisher key; the index pins publisher keys (trust on first use,
  with an official key for first-party packages).
- Hubs can run a private registry and allowlist publishers.
- Nodes verify signatures before installing or running adapters; adapters run with the node's user privileges,
  so installing third-party adapters requires explicit user approval.

## Consequences
New agent support ships without a binary release, and a compromised mirror cannot inject code.
