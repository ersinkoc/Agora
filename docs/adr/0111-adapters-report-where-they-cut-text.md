# 0111. Adapters report where they cut text, so redaction sees every edge
Status: Proposed
Date: 2026-11-24

## Context

A node replaces its own secret values (stored tokens, keys) in a structured agent's saved transcript and in what it sends live
(ADR-0089, ADR-0104, #1103, #1157). It matches a whole value, and it also masks a fragment of a secret (4 bytes or more) at every
place where text was cut, because a cut can leave only part of a secret behind, which no longer matches as a whole. The node knows
the edges it makes itself, the bridge's `[redacted]` marks, the `…` the bridge puts where it cut, and the boundaries between turns and
stream pieces ([threat model T61](../security/threat-model.md)).

It does not know the cuts other parts make before the text reaches it (#1123, recorded on #1150):

- an L3 adapter that shortens a long tool output, argument or plan text before it sends `chat/append`;
- the ACP bridge's `fit_line`, which halves every long string of an item whose line would pass the node's 4 MiB line limit
  (`crates/agora-adapters/src/acp.rs`);
- the bridge's stderr tail, where the home path is written `~`.

At such a cut, a piece of a secret shorter than the whole value can remain in the transcript. The adapter protocol
([adapter protocol](../specs/adapter-protocol.md)) has no way to say where a cut is.

## Decision

1. **A cut list on `chat/append`.** A `chat/append` item may carry `cuts`: a list of `{field, at}`, where `field` names the string
   the cut is in (a JSON pointer into the item's `params`, e.g. `/toolCall/output`) and `at` the byte offset in the string as sent
   where text was removed. At most 64 entries per item; a longer list, an unknown field or an offset that is not on a character
   boundary inside that string is ignored entry by entry (never the item). The field is optional and append-only: an adapter that
   sends none behaves as today.
2. **A capability.** An adapter that reports every cut it makes declares `chat.cuts` in its `initialize` capabilities. For an adapter
   with the capability, an item without `cuts` means it cut nothing.
3. **The node treats each reported cut as an edge** and masks secret fragments there exactly as at its own cuts (the existing
   matcher; no new kind of match). The live stream and the saved transcript both use it.
4. **The bridge reports its own cuts**: `fit_line`'s halving and every head-and-tail cut it makes, and declares `chat.cuts`.
5. **Adapters without the capability:** the node keeps today's behaviour and the threat model keeps the residual for them. Whether
   such an adapter's long strings should also be masked at their start and end, or at every `…`, is left to the build (see Open
   questions); it changes no message.
6. **Docs:** the adapter protocol spec gets the field, its limits and the capability; T61 lists reported cuts among the known edges
   and narrows the residual to adapters without `chat.cuts`.

## Alternatives considered

- **Guess cuts on the node** (every `…`, every string that ends at a round size): masks text that was never cut, and still misses
  cuts with no marker.
- **Refuse to save transcripts from adapters that cut:** loses the conversation after a restart for those agents (the reason ADR-0104
  keeps it).
- **Have adapters redact themselves:** an adapter never receives the node's secrets (adapter protocol rules); sending them would widen
  what an adapter process holds.

## Build order

1. Spec and parsing on the node (bounded, ignored entries logged once per agent).
2. The node uses reported cuts as edges (live and saved).
3. The bridge reports `fit_line` and head-and-tail cuts and declares `chat.cuts`.
4. The fake adapter gains a cutting mode for tests.

## Verification

- A fake adapter cuts a stored secret's value in the middle and reports the cut: neither the live stream nor the saved transcript
  holds a fragment of 4 bytes or more of it.
- The same without reporting, with the capability declared: the test documents the leftover (the contract is the adapter's).
- The bridge halving an item over 4 MiB that holds a secret: no fragment survives.
- Bounds: 65 entries, a bad pointer, an offset inside a character, an offset past the end: each ignored, the item kept.

## Open questions

- For adapters without `chat.cuts`, mask at the start and end of every string longer than a threshold (cheap, some false masks), or
  leave the residual documented?
- Should the ACP spec (upstream) get an equivalent, or does it stay an Agora adapter-protocol field?
