From: linux-audit (Claude Code, the evidence-audit session on linux, run by the maintainer)
To: linux (merger)
Cc: all (windows, macos, linux2, linux (Codex))
Kind: note (handoff; from the maintainer)

**The maintainer asked me to hand over my work and exit. This session stops after this message.** Everything below now belongs to the merger. I release every claim I made on comms.

## Open PRs (all mine until now, now the merger's)
| # | What | Head | CI | Next step |
|---|---|---|---|---|
| 648 | ADR-0081 text, **Accepted** by the maintainer ("0081'i onaylıyorum, kabul vs") | 856d83dc | green | docs only; merge before #657 |
| 656 | F290 (High): an ADR-0057 helper grant outlived its signer's trust. It is now re-checked on use and when listed, through #649's `vouches` | e769639e | green (1 job was still running) | Codex (security) |
| 657 | ADR-0081 implementation (node: `device-revocations-ahead`, overrides set in `confirm`, `confirm` keeps the device's line in `device-revocations`; one CLI line in en and tr) | 69684e03 | green (2 jobs were still running) | after #648; Codex (security). It changes the signatures of `device_trust::confirm` and `approvals::trust_device`; agora-tui's tests were not run |

Already merged by you: #640–#647 and #649. The maintainer also accepted #649's cost (about 4.9 µs per routed command while a grant is live; no mtime-first shortcut).

## Closed without a PR
- **F291 (not reproduced):** narrowing a device's limits only in `<data>/policy` does not leave live grants covering commands. The node reads `device.<id>.projects` only at start (`daemon.rs` ~1491-1494), and grants and helper grants are kept in memory only, so the restart that applies the change also ends them.
  - **Observation, not a finding:** an edit to `<data>/policy` does nothing until the node restarts, while trusted-devices edits apply at once. The threat model (ADR-0049 paragraph) and protocol.md (Devices) do not say this. It is a candidate for a docs note, or for a decision to reload it live (which would then need a trust-generation bump).

## Unproven leads from the audit (no claim, for whoever picks them up)
- **control.rs / uplink_client.rs:** a command with an idempotency key may run twice if the uplink retries (after 25 s) while the first run is still going. The entry is stored only after the command finishes.
- **users.rs `remove`:** it leaves a removed passkey's id on the vouched list. #641 blocks the reuse when a passkey is registered; the root is still there.
- **spawn.rs `start_resolved`:** the event records the signed grant count even when the node now declines the grant (#656). This affects only the audit log; for authority it fails closed.
- **platform unix.rs:** `kill` racing with the reap, and `openpty` descriptors leaking into a spawn made from another thread before FD_CLOEXEC is set.

## Evidence
All ledgers, proofs and verifiers (F210–F291) are in `~/Codebox/agora-audit-evidence/2026-11-16/<scope>/` on this linux machine. My worktrees are removed; the PR branches are on origin. Nothing else is in progress.
