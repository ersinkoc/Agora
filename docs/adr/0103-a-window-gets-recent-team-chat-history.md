# 0103. A window gets recent Team chat history
Status: Accepted
Date: 2026-11-23
Accepted: 2026-11-23 (the maintainer, in the merger chat: "Accept and build")

Origin: issue #1020, found while investigating #1016 (PR #1025). A window that opens or reloads after agents messaged each other
shows an empty Team chat, although the hub holds the messages.

Touches ADR-0006 (append-only schema: one appended field), ADR-0027 (the client snapshot), ADR-0040 (the precedent: the shared boards
in the snapshot), ADR-0008, ADR-0031, ADR-0035 and ADR-0074 (mesh messages, their ids and their copies), ADR-0049 (team mode: a member
or viewer sees only granted projects), ADR-0053 and ADR-0073 (forgotten agents and machines), ADR-0090 (the replay after the snapshot),
ADR-0089 (paged chat history, for its shape only), ADR-0022 and `docs/design/performance.md` (snapshot size, the hub lock),
`docs/specs/protocol.md`, `docs/specs/mesh-tools.md` and the threat model (T4, T13).

## Context

**The bug.** A window learns about a mesh message only from the live `MessagePosted` event. The client `Snapshot`
(`schema/agora/v1/client.proto:130`) has no field for messages, and the hub's view does not keep them: `MessagePosted` "changes no
view" (`crates/agora-hub/src/projection.rs:833`). So:

- a window opened or reloaded after a message was sent never shows it;
- a member or viewer always gets a snapshot, whatever position they resume from (`crates/agora-hub/src/server.rs:877-882`), and the web
  replaces its whole view with the snapshot (`apps/web/src/net/live.ts:350`). Every reconnect of a member's window therefore empties its
  Team chat, including messages it had already shown. The owner's window loses them the same way when it is more than 10 000 events or
  8 MiB behind (`MAX_REPLAY_EVENTS`, `MAX_REPLAY_BYTES`), or after a hub restore.

**The evidence (#1016, #1020, PR #1025, main 63bd193b).** The UI audit saw messages "appear late". PR #1025 measured the path from
`send_message` to the window and disproved the hypothesis that the node holds local posts back: the hub stores a message 0.15 to 0.28 s
after the tool answers and an open window shows it 0.19 to 0.29 s after, with no other event (3 of 3 runs, also at the audited commit
6f6ebe7c). In the full-stack runs, "a third message, sent before the window subscribed, was in the hub and not in the window". The PR's
test (`crates/agora/tests/hub_dial.rs`) has to wait for the window's snapshot before it sends, because "a message the hub took before
the snapshot would be in no snapshot and never come live". That is this bug.

**Where messages are today.**

- *Recorded by nodes.* A node's broker records a `MessagePosted` for a local delivery (`crates/agora-mesh/src/broker.rs:782`) and for a
  post to another machine (`broker.rs:847`). A message between machines is recorded twice, by the sender's node (`to` is
  `<node>/<agent>`) and by the receiver's (`from` is already `<origin>/<agent>`, ADR-0031). A `#channel` message on a shared project is
  delivered by the hub as one copy per receiving agent (`crates/agora-hub/src/board.rs:46`), and each receiving node records its copy
  (body `[#dev] ...`); the hub writes no `MessagePosted` itself.
- *Stored by the hub, for good.* Every event is appended to the hub's SQLite `events` table (`crates/agora-hub/src/store.rs:135`:
  `pos`, `producer`, `seq`, `at_ms`, `payload` BLOB) by `ingest_locked` (`crates/agora-hub/src/hub.rs:362`). Nothing in `store.rs`
  prunes it. There is no index by event kind or project: finding messages means reading payloads. Message bodies are in the event store
  already; ADR-0049 keeps them out of the audit table only.
- *Filtered per person.* A member or viewer receives a live `MessagePosted` only when its sender agent is in a granted project
  (`crates/agora-hub/src/scope.rs:266`: "it is theirs only when the sender is theirs"). The lookup qualifies `from_agent_id` with the
  producer (`qualified`, `projection.rs:281`), so only the **sender's own copy** is ever admitted: a receiver's copy, whose `from` is
  already `<origin>/<agent>`, and so every `#channel` copy, reaches the owner only (the test
  `a_message_between_machines_is_the_members_by_the_senders_own_copy_only`, `scope.rs:632`). `MessagePosted` names no node, so `is_forged`
  never refuses it (`projection.rs:253`); a node can write any `from`, and the web marks a copy that only another machine vouches for
  with `via` (#905).
- *Held by the web.* `View.messages` keeps the newest 50 (`MAX_MESSAGES`, `apps/web/src/net/view.ts:118`), pairs the two copies of a
  message between machines by sender, recipient, text and producer (`view.ts:423-445`), and the side panel shows the newest 5
  (`recentMessages`, `view.ts:697`). Pairing and `via` need the event's producer and time, which are in the `Event` envelope
  (`producer_id`, `at_ms`), not in `MessagePosted` (`events.proto:353`: `message_id`, `from_agent_id`, `to`, `body`).

**What constrains a fix.**

- `MessagePosted` has no project, no time, no producer, and its `message_id` is per node (ADR-0074). Only the hub's log position
  orders and names a stored message hub-wide.
- A body may be 16 KiB (`max_body_bytes`, `broker.rs:36`). A frame, and so a whole snapshot, is at most 1 MiB
  (`DEFAULT_MAX_PAYLOAD`, `crates/agora-protocol/src/frame.rs:30`; the web's `MAX_PAYLOAD`, `apps/web/src/net/frame.ts:24`).
- The hub clones its view under its lock on every subscribe (`hub.rs:768-806`). Whatever the view keeps is copied there.
- Clients do not negotiate capabilities with the hub. The web window is served by its hub and always matches it
  (`docs/specs/protocol.md`, `NodeForget` row); the terminal client and the desktop shell may not. An older hub decodes a request whose
  command it does not know as no command, which `dispatch::route` refuses as `auth.forbidden` (`crates/agora-hub/src/dispatch.rs:50-60`),
  not as a missing capability.
- A viewer sends only the read-only commands listed in `viewer_may` (`crates/agora-hub/src/users.rs:329`).

## Options

### (a) The newest messages in the snapshot

The hub keeps a bounded tail of `MessagePosted` events in its view and appends them to the snapshot as a new repeated field.

- *Size and performance.* Bounded by a count and a byte cap, it is a fixed addition to one frame that is sent once per subscribe. The
  hub's view gains a ring per project; with shared (reference-counted) entries the clone under the lock copies pointers, not bodies.
  Selecting a window's newest entries runs on the cloned view, outside the lock, like the rest of `snapshot_of`.
- *Scope.* The tail is cut with the same `Scope` as the rest of the member's snapshot, by the rule live events use (`scope.rs:266`), so
  the snapshot and the live stream agree.
- *Store.* No new storage: the ring is folded from the events the hub already applies, so `View::rebuild` at start rebuilds it from the
  log, as ADR-0040 does for the boards.
- *Compatibility.* An appended proto3 field: an older window ignores it, an older hub sends none and a newer window shows what it shows
  today. No capability. No race: the tail and the snapshot are one consistent cut at `pos`, and live events continue after it.
- *Limit.* Only recent messages; nothing older can be read.

### (b) A paged request

A hub-answered command, for example `mesh.history { project_id, before_pos, limit }`, returning stored `MessagePosted` events newest
first with a `more` flag.

- *Size.* Unbounded history without growing the snapshot.
- *Store.* The hub would have to find messages in a log with no index by kind: a scan backwards over payloads (under the lock, like
  replay, so bounded per page) or a store migration that adds an index table of message positions per project. It must also apply the
  window's scope and the forgotten-machine rule to every row.
- *Race.* A request sent after the snapshot answers from a later log than the snapshot unless it is cut at the snapshot's `pos`. ADR-0040
  rejected "ask the hub for the board with a command when the window opens" for this window "in which events can be missed".
- *Permissions.* Viewers must be allowed it (`viewer_may`), so a viewer is not left with the empty Team chat this ADR is about.
- *Compatibility.* No client capability exists to learn whether the hub has it. An older hub answers `auth.forbidden`, which a window
  must not show as "not allowed". The web is safe (served by its hub); the terminal client and the desktop shell are not.
- *Latency.* A second round trip after the snapshot before Team chat has content: against the client's first useful paint budget
  (`performance.md`, 1.5 s, 500 ms cached), and a flash of an empty panel.

### (c) Both: a small tail in the snapshot and paging for older messages

(a) for what the window shows on open, (b) for scrolling back. It costs both, and (b)'s costs buy only something no window shows today:
the web holds 50 messages and shows 5, and there is no full Team chat view to scroll yet.

## Decision

**(a) now, shaped so that (b) can be added later without changing it.** Paging is not decided here (open question 1).

### 1. The schema: one appended field

`Snapshot` (`client.proto`) gains, at the next free number (12 today, checked at the schema PR):

```
// ADR-0103: the newest mesh messages this window may see, oldest first, each as the hub stored it (its log position, producer, time
// and `MessagePosted` body). Positions are before `pos`. A client folds them in as it would the live events, and ignores any other body.
repeated PositionedEvent recent_messages = 12;
```

`PositionedEvent` (`pos`, `Event`) is already published in `client.proto`; reusing it carries the envelope that pairing and `via` need,
and the hub sends the stored event as it is. No new message, command, event, error code or capability. A golden fixture is added (a
snapshot with two messages, one pair of copies of a message between machines), none edited.

### 2. The hub keeps a tail per project

- `View` keeps, per project (`<node>/<project>`), the newest **50** `MessagePosted` events and at most **256 KiB** of their encoded
  size, whichever is smaller, dropping the oldest. Over all rings, at most **8 MiB**, dropping the oldest across projects. Entries are
  shared, so the view's clone under the lock does not copy bodies.
- **Placement uses only what the producer can vouch for: its own agents.** A message goes to the ring of the project of
  `<producer>/<from_agent_id>` when that is an agent the hub knows (the sender's own copy, the one `scope.rs:266` admits); otherwise
  to the ring of `<producer>/<to>` when that is a known agent (a receiver's copy of a message between machines, and each `#channel`
  copy); otherwise to one more ring of the same limits that only the owner's snapshot reads. Each entry records whether it was
  placed by its sender (it passes the live rule) or by its recipient.
- **The hub never resolves an already-qualified `from` to another machine's agent.** A node can write any `from` (T13): resolving it
  would let a forged sender into the ring, and the snapshot, of a project its machine does not host. Placing a receiver's copy by its
  own recipient only bounds it per project; it does not make it a member's (decision 3).
- Copies are kept as stored: both copies of a message between machines, and each receiver's copy of a `#channel` message. The hub does
  not pair or deduplicate; the window does, exactly as for live events (#905). (A channel message to many agents can fill the tail with
  copies; that is what the live stream shows today too.)
- It is folded where the view applies events (`projection.rs`, the `MessagePosted` arm that today does nothing), so a hub that rebuilds
  its view from the log at start has the same tail.
- Per project so that one busy project does not push a quiet one's messages out of a member's snapshot.

### 3. What one window is sent

- From the rings the window may see: the owner's shown view (ADR-0073: no sender under a forgotten machine; no sender that is a
  forgotten agent, ADR-0053), a member's or viewer's cut view: only entries placed by their sender, in a granted project, the
  rule live events follow (`Scope::admit`), so a member gets in history exactly the copies they would have got live. Receiver and
  `#channel` copies stay the owner's, as live today; whether members should see them is not this ADR's change (open question 5).
- The newest **50** in log order, then fewer until they encode to at most **256 KiB** together, sent **oldest first**. 50 is the web's
  `MAX_MESSAGES`; 256 KiB leaves the rest of the snapshot three quarters of the 1 MiB frame (16 maximal bodies; real messages are much
  smaller). The limits are hub constants, named in `docs/specs/protocol.md`, not policy.
- The owner's binary-path rule (ADR-0069) and every other per-person rewrite do not touch messages; bodies are sent as stored.

### 4. The window

- `fromSnapshot` folds `recent_messages` into `View.messages` in order through the same code as the `messagePosted` case of
  `applyEvent` (pairing, `via`, the 50 cap), but not through `applyEvent`'s position rule: their positions are before the snapshot's,
  which `applyEvent` would drop (`view.ts:192`). Live events then continue after `pos` as today.
- Bodies are agent text, untrusted (T4): shown exactly as a live message is shown today, as plain text, never as markup. A history
  entry gets no handling a live one does not, and nothing in a message is an instruction to the window. The terminal client does not show
  Team chat today and ignores the field.

### 5. Limits and errors

No error code: the field is filled or empty. A hub that cannot read its rings (none today: they are memory) sends an empty field rather
than failing the snapshot.

## Consequences

- **Fixed.** A window that opens, reloads or reconnects into a snapshot shows the recent Team chat, for the owner, members and viewers
  alike. (PR #1025's test still waits for the snapshot: it asserts a live event. A new test covers send, then open.)
- **Schema.** One appended field and one fixture (ADR-0006); `schema-breaking` stays green.
- **Hub.** The rings in `View`, their fold and rebuild, the per-window selection in the snapshot builder (`snapshot_of` and the scope's
  `filter_view`), tests. Memory: at most 8 MiB of message events; a subscribe clones up to (projects + 1) × 50 pointers under the lock.
- **Snapshot size.** At most 256 KiB more per snapshot, usually a few KiB. A hub with 50 clients that all reconnect at once sends at most
  12.5 MiB more in total. A benchmark of `subscribe` and `snapshot_of` with full rings goes in the hub PR (ADR-0022).
- **Privacy.** No new data leaves the hub to anyone who could not already have received it live, with one exception (open question 2).
  The hub already keeps every message in its log for good; this ADR neither adds nor removes retention.
- **Docs.** `docs/specs/protocol.md` (the snapshot field, the limits, the fold rule), `docs/specs/mesh-tools.md` (where "Channel history
  is not kept" is about the receiver's inbox, a line that the window still shows the recent copies), the threat model's T4 row (history
  is the same untrusted data as live messages).
- **Tests.** Send, then open a window, and the window shows the message (the test #1020 asks for, in `crates/agora/tests/hub_dial.rs`
  and in the web's view tests); a member's snapshot holds only messages from granted projects' senders; a forgotten machine's messages
  are not in the owner's; a rebuilt view has the same tail as the live one; the caps (51 messages, 17 maximal bodies, a busy project next
  to a quiet one); the two copies of a message between machines arrive as one row after a reload; an older-window decode of the new
  snapshot (the fixture) is unchanged.

## Alternatives considered

| Alternative | For | Against |
| --- | --- | --- |
| (b) Paging alone | unbounded history; no snapshot growth | a scan or a store index; a race after the snapshot unless cut at `pos`; no hub capability for the terminal client and desktop, and an old hub answers `auth.forbidden`; a second round trip before the panel has content |
| (c) Tail and paging now | complete | pays for scroll-back no window offers yet; (a) is the first half of it anyway |
| Replay the kept messages after the snapshot, as ADR-0090 does for session config and slash commands (at the snapshot's `pos`) | no schema change; older windows drop them by the position rule | ADR-0090 needed it for events that only **replace**; these **append**, so the window has to tell a history copy from a live event at the same position, and the stream's length after a snapshot depends on chat history, which ADR-0040 rejected for the boards |
| `repeated MessagePosted` in the snapshot | smaller | loses the producer, time and position that pairing (#905), `via` and ordering need |
| A new `MessageSnapshot` message | could add a project field | duplicates `PositionedEvent`; the project is the hub's derivation, not stored data |
| Ask the sender's node (its broker's inboxes) | no hub memory | nodes may be offline; inboxes are per recipient and node-local; the hub already has every message |
| Keep the full history in the window (local storage) | no protocol change | another device or browser still starts empty; text stored in the browser outlives a revoked grant |

## Open questions

1. **Paging (b), later?** Proposed: not now. When a full Team chat view with scroll-back is designed, a separate ADR adds a hub-answered
   request with the cursor as the hub's log position (`before_pos`), cut by the same scope and forgotten rules, allowed to viewers, behind
   a hub capability that clients can see (which needs a way for a client to learn the hub's capabilities first). Its shape could follow
   ADR-0089's `chat.history` (accepted, not built: no schema yet, roadmap M3.3); its store is the hub's, not the node's. Today's next free
   numbers would be request 110 and `CommandOk` 32.
2. **A member granted a project today sees its messages from before the grant**, up to the tail. The board (ADR-0040) and the agents of a
   granted project behave the same way. Proposed: accept, and say so in `docs/ux` where grants are explained.
3. **Messages of a forgotten agent (ADR-0053).** Proposed: left out of the snapshot, like the agent; still in the log.
4. **Limits: 50 messages and 256 KiB per window, 50 and 256 KiB per project, 8 MiB per hub?** Proposed: yes; they are constants and can
   be tuned without a schema change.
5. **Members and receiver copies.** A member never sees a receiver's copy or a `#channel` copy, live or in history, so a message sent
   to their agent from another project's agent is invisible to them. Proposed: unchanged here; a separate issue if the maintainer wants
   it, since it means trusting a receiver's word for who sent it.
6. **Retention of message text in the hub's log**, which keeps it for good today. Out of scope; a separate issue if the maintainer wants
   one.

## Build order (each its own PR, after acceptance)

1. **Schema:** `Snapshot.recent_messages`, the golden fixture, `docs/specs/protocol.md`.
2. **Hub:** the rings in `View` (fold, rebuild, caps, shared entries), the per-window selection with scope and forgotten rules, the
   benchmark of `subscribe`/`snapshot_of` with full rings, the hub tests.
3. **Web:** `fromSnapshot` folds the tail through the `messagePosted` path; the #1020 test in `hub_dial.rs` (send, then open a window,
   and the window shows the message) and the web view tests; a reload in headless Chromium shows Team chat.
4. **Docs:** `mesh-tools.md`, the threat model row, and `docs/ux` for open question 2.
