# 0134. Agent accounts are configured on the node
Status: Accepted
Date: 2026-11-27
Accepted: 2026-11-27 (maintainer approval in chat).

## Context

ADR-0084 covers named node secrets and kind-scoped environment injection. M3.10 still needs a
decision on login and account selection. Some tools store subscription sessions in a configuration
directory. Others consume a key or have an ACP authentication method. The current agent-account
design sketches several mechanisms with different trust boundaries.

## Decision

1. Start with one explicitly selected account on one node per agent launch. The node manages opaque
   account IDs, kind, an owner-chosen label and an isolated configuration root for kinds whose
   manifest and runtime can enforce that separation. Default launch keeps the existing behavior
   for agents that use their ordinary local configuration. An explicitly chosen account either
   launches in its own root or fails with a stable reason. It never falls back to another account.
2. Provision, authenticate and remove managed accounts using node-local owner commands. Interact
   with a verified native login flow locally, or through an OS-authenticated SSH terminal. For a
   key-based account use the node's existing local secret mechanism and store its reference only.
   Keep provider credentials, device codes, configuration paths and login output off the hub.
   Use the kind's verified configuration isolation mechanism and node-owned metadata. Grant a
   confined agent only the access it needs to its selected account directory under node policy.
   Refuse managed accounts for kinds whose supported releases cannot guarantee isolation.
3. Determine readiness with a manifest-declared, bounded node-side status check. Report an enum
   such as unknown, ready, needs login or unavailable. The check reveals no identifiers from the
   provider and is neither a login nor a guarantee that the next provider request will succeed.
   Cache its result and invalidate it on account or credential changes. A request may omit the
   check when the manifest cannot perform a safe one. A local launch must remain possible without
   the hub.
4. In a later, separately approved schema step, let authorized owner windows read an opaque
   account ID, kind, label and coarse status from a visible node, and choose it on launch. Include
   the selected ID in the existing node-verified signed launch authorization. Require the
   applicable capability and current node policy. Legacy launches keep their prior defaults.
   Team sharing of account metadata and browser-based login require their own access decisions.
5. Removal refuses new launches and resumes using that account. Initially refuse local removal
   while a run uses the account, so the owner can stop it explicitly. After removal, an interrupted
   session that referenced that account requires explicit recovery. It does not silently switch
   identities. Provider-side revocation remains a provider operation. The node audit records
   account ID, kind, action and outcome, with no provider identifier or login details.

## Alternatives

- Send login screens or credentials through the hub. This would extend connection, storage and
  support surfaces to authentication data and would need a separate protocol and threat model.
- Reuse the user's default configuration for every identity. That cannot isolate accounts and
  can silently bill or act under the wrong one.
- Use only environment keys. Keep that supported under ADR-0084 for compatible tools. It does
  not cover subscription sessions or explicit profile selection.

## Consequences

The first implementation can establish real per-kind behavior without changing the hub protocol.
Owners authenticate where credentials are held. A later UI can choose an existing account after
the signing and visibility rules have protocol approval.

## Verification

Use a fake provider and fake manifest kinds. Show that two accounts and the normal launch use
separate state, that one cannot fall back to the other, and that inherited credentials do not
override the explicit selection. Cover denied policy, incompatible kind, failed or timed-out status,
launch without hub, removal with a live or interrupted run, substituted account ID in a signed
launch, and unauthorized enumeration. On each supported kind, a person verifies its documented
login and configuration isolation on a real release before enablement.
