# 0089. The chat history of a structured agent survives the node and is read in pages
Status: Accepted
Date: 2026-11-19
Amended: 2026-11-24 (Amendment A below: the maintainer's decision on #1147, live redaction)

Accepted: 2026-11-19 (the maintainer's explicit message in the merger chat: "ADR-0089'u kabul ediyorum"). The open questions' proposed answers stand: 30 days and 32 MiB per agent, the node's own known secret values replaced by a marker (unknown secrets in output cannot be found and the ADR says so), export to Markdown later as its own small PR. It builds on ADR-0090 (Accepted): the new entry kinds and fields are what is stored.

Builds on ADR-0038 (the chat transcript stream; it says "persisting transcripts is a separate decision"), ADR-0060 (the agent session
ledger and resume), ADR-0022 (bulk output stays out of the event store), ADR-0053 (forget closed agents). Touches the threat model
(secrets in output) and the invariant "Secrets never leave the node".

Origin: the maintainer wants the chat of a structured (ACP) agent to read like a real coding tool's history: the whole conversation is
there after a reload, after the node restarts and after the agent is resumed.

## Context
- ADR-0038 keeps the last 500 entries or 256 KiB per agent in the node's memory. A client that subscribes gets that ring as a snapshot.
  A node restart loses it, and older entries are gone for good. With ADR-0060 an agent can be resumed after a restart, but its screen of
  conversation starts empty.
- The agent tool keeps its own conversation (for resume), but it is not in a form Agora can page or search, and Agora must not read
  vendor files it does not own for this.
- The transcript holds what the model said and what tools printed, so it can contain secrets (a command printed an environment variable;
  a file with a key was read). The hub stores none, by invariant.

## Decision
1. **The node writes each structured agent's transcript to its own disk,** append-only, one file per agent under the node's home
   (`transcripts/<agent id>.jsonl`), mode 0600 in a 0700 folder, written through the platform layer, never to the hub. One line per
   `ChatEntry` as ADR-0038 defines it, plus a format version in the first line. Writes are batched (at most once per second or per
   64 KiB) and never block the agent or the stream: if the disk is slow or full, entries stay in the memory ring and the node says
   "history not saved" once (a locale string and an error code), it does not stop the agent.
2. **Bounded, with the owner's choice.** Default per agent: 32 MiB and 30 days after the agent closed; a node policy
   `chat.history_max_bytes` and `chat.history_days` lowers (never raises past a hard cap of 256 MiB per agent, 2 GiB per node) the
   defaults. Over the cap the oldest entries are dropped by rewriting the file in chunks (a rotated segment file, not an in-place
   edit), and the file says it was truncated.
3. **Forgetting an agent forgets its history** (ADR-0053): the file is removed with the closed agent's other traces. `agora down`
   does not touch it. A node's `agora history purge` removes all of it.
4. **Paged reads.** A new command `chat.history { agent_id, before_entry_id, limit }` (limit at most 200 entries and 256 KiB per
   answer) returns older entries, newest first, with a flag `more`. It is routed by the hub like `chat.subscribe` and answered by the
   node; the hub keeps nothing. The existing snapshot of the ring stays as it is (older clients are unaffected); a client with this
   capability asks for pages when the user scrolls up. Entry ids are continuous across restarts: the node continues numbering from the
   file's last id.
5. **Resume continues the same history.** When an agent is resumed (ADR-0060) it keeps its agent id and transcript file, and the
   window shows the earlier conversation above a "resumed" marker. A fresh start (ADR-0060 fresh) begins a new file and the window links
   to the old one as "earlier session".
6. **Search is local and later.** A node-side text search over one agent's file may come in a later ADR; this one only pages.
7. **Secrets.** The file is as sensitive as the terminal. It stays on the node, is never sent unless the user's window asks, and the
   values of the secrets the node itself holds (ADR-0084 and the secret store) are replaced by a marker before an entry is written; no
   existing ADR defines redaction of arbitrary output, so this does not claim to find unknown secrets. The
   threat model gets a row: the transcript file read by another local user (mode and folder checks) and by a confined agent (it is
   outside every confined agent's allowed paths).
8. **Terminal agents are out of scope.** A terminal agent's screen stays a screen (ADR-0022); only structured agents get this.

## Open questions for the maintainer
- Default 30 days and 32 MiB per agent? (Proposed: yes.)
- Redact known secret values before writing, or write raw and rely on the file mode? (Proposed: replace the node's own known secret values, and say plainly that unknown secrets in output cannot be found.)
- Export of a conversation to Markdown from the window (a download made by the node)? (Proposed: yes, later, as its own small PR.)

## Consequences
Schema (a new `ChatHistoryRequest`/`ChatHistoryPage` message pair and a capability; golden fixtures; ADR-gated), the node (writer,
retention, continuous ids, resume wiring, purge command, policy keys), the hub (routing only), the web client (pages on scroll up,
"resumed" marker, "history not saved" notice), the TUI (later), docs (`docs/specs/protocol.md`, `docs/ux`), threat model, tests (a
crash during a write leaves a readable file; a full disk does not stop the agent; ids continue after a restart; the file is 0600;
a confined agent cannot read it; paging returns exactly the entries before an id).

## Build order
1. Schema PR (after acceptance).
2. Node writer, retention and paging.
3. Resume wiring and the purge command.
4. Web: pages on scroll up and the markers.

## Build status

- Decisions 1, 2, 3 (the node's side) and 7, and the seeding of decisions 4 and 5, are built ahead of the schema step, as ADR-0104 C1
  (its build step 4, #931) decides: the writer, the bounds and policy keys (both only lower the defaults), retention counted from when
  the agent ended, `agora history purge`, the node's secret values replaced, continuous ids, and a resume or a take-up that shows the
  earlier turns once (an agent's own replay of a loaded session is not shown again). See ADR-0104 "Build status".
- Not built: the schema step and paging (decision 4's `chat.history`), the window's scroll-back and its "resumed" and "earlier session"
  markers (decision 5; a fresh start already keeps the earlier session as `<id>.earlier.jsonl`), search (decision 6).
- Decision 3's "forgetting an agent forgets its history" is built by ADR-0108 (#1104): a forget at the hub reaches a node that offers
  `agent.forget.local` (at once, or when it next connects), which removes the agent's file and its earlier session's and no longer
  offers it for Resume. An older node still leaves the file to retention.

## Amendment A (2026-11-24, #1147): the node's own secret values are replaced in what it sends live too
Status: Accepted (2026-11-24, the maintainer's explicit message in the merger chat: "Yes, redact live too").

**Why.** Decision 7 replaces the values of the secrets the node holds before an entry is **written**. The live chat stream did not:
the ring sent each entry to `ChatSubscribe` followers as it was, so a value an agent printed reached the hub (in transit, as a relay)
and the window while the file masked it. The invariant "Secrets never leave the node" asks for the same replacement on the way out.

**Decision** (the maintainer's answer above). The node replaces its own known secret values (decision 7's: its secret store and the
environment variables its policy passes to agents, ADR-0084) in everything it sends a follower: each entry of the live stream
(every version of an entry that changes in place), the snapshot a new follower gets, and any page or history of the conversation it
sends, with the same rule and residuals as the file. Paging (`chat.history`, decision 4) is not built yet; when it is, it serves the
same redacted entries. No schema change: a follower gets the same messages, with `<redacted>` where a value was.

**How #1147 builds it** (for the maintainer's review with that PR).
1. The same rule from the same code: (a) whole values, raw and as JSON escapes them, in every text and across the agent's words as one
   stream; (b) every part of 4 bytes or more of a value that touches or crosses an edge. The live path keeps its own stream of the
   agent's words (a carry, its edges), as the writer keeps its own.
2. Never a version unredacted first and redacted later: an entry is redacted before it is first sent, and the agent's words a value
   may still continue wait. Only those wait: with no value held nothing waits; otherwise a piece waits only while the stream ends in
   a part of a value of 4 bytes or more, until the next words show it is not one, until any other entry of the agent (so ids still
   increase on the stream: a client may drop an older new id), until its turn ends, Start again, the ring's bound, or at most
   `LIVE_HOLD_MS` (300 ms, checked on the node's 100 ms tick); a piece sent that way has its end treated as an edge.
3. A new follower's snapshot shows the words as they were sent and leaves out those still waiting; the stream brings them.
4. Residuals: those of the file (a value under 8 bytes, a fragment under 4 bytes, the cuts the node cannot see, JSON forms other than
   `serde_json`'s, a secret it does not hold), and these of the live path: the live view and the file may differ where a part of a
   value stands at an edge by chance (each masks it at its own edges); a value's first 3 bytes or fewer can be sent before the next
   words show the value goes on (the rest is replaced; the file, which holds back more, masks them); text sent before a value was
   added to the store stays as sent on streams already open (every later snapshot has it replaced), and a value added is replaced
   from the node's next read of its values (at most a second later); a value over 16 KiB (only one from the node's environment can
   be) can show in part when its words are sent without what follows.
5. The same values are replaced in the agent's own words the node records as hub events (audit of #1157): an approval card's action
   and detail (before the request is queued, so the queue, the event and the always signature hold the same words), a mesh message's
   body, a task's title, a handoff's summary, and the names, descriptions and hints of the agent's modes, options and commands. Ids
   are kept. Not covered: the body of a mesh message an agent sends to an agent on another machine, which the hub relays as written
   (a question for the maintainer: replacing it changes what the other agent is told).
