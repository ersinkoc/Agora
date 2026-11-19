# 0091. The plan limits of subscription agents are shown from the tools' own output
Status: Proposed
Date: 2026-11-19

Not accepted. Nothing here may be built until the maintainer names this ADR in an acceptance message.

Builds on ADR-0042 (usage from agent files), ADR-0021 and ADR-0033 (the catalog and price estimate), ADR-0004 (agents are manifests),
`docs/design/usage-and-cost.md` section 3 ("quota profiles", not built). Touches ADR-0020 (approvals, for the Claude Code reporter), the
invariant "Secrets never leave the node" and the threat model.

Origin: the maintainer: agents on a subscription (Claude Code, Codex and others) have plan limits, so the window must show how much of the
limit is used and when it resets. Dollar amounts are not wanted: people on a plan do not pay per token.

## Context
- Agora measures tokens from the agents' own files (ADR-0042) and shows them in the Usage panel. It knows nothing about plan limits.
- Codex writes, in its own session file under the user's home, `token_count` events that carry `rate_limits` (a primary window with
  `used_percent`, `window_minutes`, `resets_at`, optionally a secondary window, and account fields such as `plan_type` and a credit
  balance). The node already reads that file for tokens. There is no official documentation of the format; it is known from the file
  itself and from community sources, so a reader must tolerate change.
- Claude Code documents (statusline input) `rate_limits.five_hour` and `rate_limits.seven_day` with `used_percentage` and `resets_at`
  for subscription users, only after the first reply of a session, each window possibly absent. Its session files carry no limit
  information. Getting the numbers means Agora installs a status line reporter in the user's Claude settings (or starts the agent with
  its own settings file), which edits something the agent trusts and may displace a status line the user already has.
- A vendor usage endpoint with the user's token exists for some tools. Agora does not use it: it needs the user's credential, it is not
  documented as supported, and the secret must never leave the node.

## Decision
1. **A limit observation is a small neutral record:** `{ tool, window, used_percent, resets_at, source, observed_at }`. `window` is
   `five_hour`, `seven_day`, or `minutes:<n>` for anything else (the label is derived from the window length, never from a field name
   such as "primary"). `used_percent` is 0 to 100 and clamped; `resets_at` is a time the node bounds (not in the past by more than a
   day, not more than 31 days ahead). Credits, balances, plan names, account ids, tokens and anything else the tool reports are not part
   of the record and are never forwarded. A strict allow-list of fields, in code, with a test.
2. **Codex first, from the file the node already reads.** The usage reader of ADR-0042 gets an optional `[quota]` mapping in the manifest
   (JSON paths for percent, window length and reset), so no agent-specific Rust is needed, and a malformed or missing field just means
   "unknown". Nothing is installed, nothing is called, no credential is touched.
3. **Claude Code only with its own ADR-gated step.** The first build rung does not touch Claude's settings. A later rung may start Claude
   Code with a settings file written by the node that adds a status line reporter (the node writes to a private path, the reporter writes
   only the allow-listed record to a socket the node owns), after the maintainer accepts that rung by name in its own ADR amendment. It must
   keep a user's own status line working by chaining to it. Until then Claude Code shows "unknown" for limits, and says why.
4. **Fresh, stale and unknown are different things.** A window is shown with "observed N minutes ago". After `resets_at` passes it is shown
   as "reset" (not as the old percentage) until a new observation arrives. With no observation the window shows "unknown". A limit is
   never invented from token counts or from a profile.
5. **A new message, not a changed one** (the schema is append-only): `AgentQuotaObserved` as an event to the window, with a capability
   (`usage.quota`); `AgentUsageReported` is unchanged. The hub keeps the last record per agent kind and machine in its snapshot (a small
   bounded table), nothing else.
6. **Where it shows:** the Usage panel gets one bar per tool and window, with the reset time and the age of the observation; an agent card
   shows a small chip from 80 percent and a stronger one from 95 percent; the start window shows a hint (it does not block) when the chosen
   kind's window is at 95 percent or more, with the reset time. All text in the UI's own words, from `locales/`.
7. **No money anywhere.** The record has no price field, and the UI shows no currency.

## Open questions for the maintainer
- Show the percentage only, or also a rough "enough for about N more minutes" estimate? (Proposed: percentage and reset time only; an estimate
  would be a guess.)
- Accept Claude Code's reporter rung at all, or leave Claude Code at "unknown" until the vendor offers a supported local source?
  (Proposed: decide when the Codex rung is built.)
- Notify (desktop, phone) when a window passes 95 percent? (Proposed: yes, as a normal low-priority notice, off by default.)

## Consequences
Manifest format (`[quota]`, additive, `min_engine` raised), schema (a new event and a capability, golden fixtures, ADR-gated schema PR), the
node reader with tests (a Codex-shaped file with and without the secondary window, malformed fields, a hostile value out of range,
fields that must never appear in the record), the hub (snapshot table), the web client (Usage bars, card chip, start hint), the TUI later,
docs (`docs/design/usage-and-cost.md` section 3 is replaced by this), the threat model (a local file feeding a number the window shows:
untrusted, clamped; the reporter rung: a settings file the agent trusts).

## Build order
1. Schema PR (after acceptance).
2. Node reader and the Codex manifest mapping.
3. Hub snapshot table and the web Usage bars, chip and start hint.
4. Optional, separately accepted: the Claude Code reporter rung.
