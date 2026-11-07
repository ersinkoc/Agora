# 0015. Identity and device enrollment
Status: Proposed
Date: 2026-11-06

## Decision (proposed)
- **Users:** local accounts with passkeys (WebAuthn), password as fallback. OIDC login in team/cloud mode.
- **Devices:** every node and every client device has its own key pair. A node enrolls with a one-time code
  shown in the window; the hub issues a client certificate (mTLS). Revocation is immediate.
- **User approval keys:** each client device holds a signing key used to sign approvals (see ADR-0020).
- Multi-user from day one in the model; the local hub simply has one user.

## Consequences
No shared secrets in config files; stolen hub credentials do not let an attacker enroll nodes silently.
