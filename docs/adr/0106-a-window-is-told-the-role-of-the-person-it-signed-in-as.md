# 0106. A window is told the role of the person it signed in as
Status: Accepted
Date: 2026-11-23

Accepted: 2026-11-23 (the maintainer, in the merger chat: "Accept and build" for ADR-0106).

Origin: issue #1099, the leftovers of #1098 (the terminal client's folder picker for members). PR #1107 fixes what it can without a
protocol change. On 2026-11-23 the maintainer chose to draft this ADR for the rest ("Draft a Proposed ADR").

Touches ADR-0006 (append-only schema: one appended enum and one appended field), ADR-0049 (team mode: owner, member, viewer; every
refusal for a role is audited), ADR-0049 Amendment A (#1049, a member starts agents in granted projects), ADR-0065 (pairing, whose
`PairStatus` is owner-only), ADR-0027 (the client snapshot), `docs/specs/protocol.md` (handshake) and the threat model.

## Context

A window (the web window or the terminal client) does not know whether the person it signed in as is the owner, a member or a viewer.
It finds out from a refusal:

- **Pairing cards and Usage.** The web (`apps/web/src/net/useUsage.ts`) and the terminal client ask `PairStatus`. The hub answers it to
  the owner only (`pairing_net::owner_command`) and refuses everyone else with `auth.forbidden`.
- **The folder picker.** The terminal client learns that the person is confined to granted projects from that same refusal, or from a
  refused `DirectoryList`.

The hub writes every refusal for a role as a row in the owner's Activity (ADR-0049). So finding out the role costs a refused row:

- A member's window writes one refused `pair.status` row for each connection after a lost link. #1107 re-asks after a lost link
  because the hub ends a connection when the person's role or grants change (`server.rs`, checked once a second).
- A viewer is offered "start in a folder" (`f`), and their first try writes one refused `DirectoryList` row.
- A member promoted to owner during a routine reconnect (the terminal client reconnects every 500 ms while events arrive) keeps the
  "not the owner" answer for the pairing cards until a lost link or a restart.

Nothing on the wire carries the role today:

- `Hello` and `Welcome` carry generations, capabilities, `peer_id` and the time. `Hello.role` is the connection kind (node or client),
  not the person's role.
- The `Snapshot` is the scoped view only. A viewer's snapshot is the same as a member's with the same grants, and an owner's can look
  like a member's.
- `Welcome.capabilities` is defined as the intersection of both sides' capabilities, not facts about this session.

## Options

1. **An appended `Welcome.person_role`.** The hub fills it when it answers a client's `Hello`, from the same `access()` it already
   uses. It holds for the whole connection, because the hub ends a connection whose role or grants changed.
2. **A new command any role may ask** (for example `WhoAmI`), never refused and never audited. It is also a schema change, costs a
   round trip, and adds a command whose only purpose is what `Welcome` could say.
3. **A capability string** (for example `role.viewer`) in `Welcome.capabilities`. It needs no new field, but it breaks that list's
   definition (an intersection of what both sides support) and mixes session facts into negotiation.
4. **Keep learning from refusals.** No schema change, and the refused rows stay: one per member connection after a lost link, and one
   per viewer's first folder try.

## Decision

Option 1.

```proto
// ADR-0049: the role of the person a client connection signed in as.
enum PersonRole {
  // An older hub, a node's connection, or a hub not in team mode that predates this field: the client keeps today's behaviour.
  PERSON_ROLE_UNSPECIFIED = 0;
  PERSON_ROLE_OWNER = 1;
  PERSON_ROLE_MEMBER = 2;
  PERSON_ROLE_VIEWER = 3;
}

message Welcome {
  // fields 1-4 unchanged
  // ADR-0106: the role of the person this client connection signed in as, when the connection opened. Unspecified for a node.
  PersonRole person_role = 5;
}
```

1. **The hub** fills `person_role` for every client connection from the access it already resolved for that connection. A hub that is
   not in team mode (one person) sends `PERSON_ROLE_OWNER`. A node's connection gets `PERSON_ROLE_UNSPECIFIED`.
2. **The value holds for the connection.** The hub already ends a connection whose role or grants changed. The client reconnects and
   gets the new value in the next `Welcome`. No event carries role changes.
3. **The role is a hint for the window, never a permission.** The hub keeps checking every command against the person's access, as
   today. A window that shows an action the role does not allow is still refused and audited. A window that hides an action the role
   does allow is a display bug, not a security hole.
4. **Windows use it to stop asking.** When `person_role` is set:
   - the web and the terminal client stop asking `PairStatus` unless the role is the owner;
   - they choose the folder picker by role (owner: the machine's home; member: the granted project folders);
   - they hide `f` and other changing actions from a viewer.
   When it is unspecified (an older hub), both keep today's behaviour, including #1107's.
5. **Docs.** `protocol.md` (handshake), `capabilities.md`, and the threat model: the role is the person's own, already visible to them
   in Settings, so telling their window adds no exposure.

## Consequences

- A member's or viewer's window writes no refused row in the normal course. The owner's Activity shows only real refusals.
- A member promoted to owner sees the owner's view after the reconnect the hub forces; no routine reconnect can miss it.
- One appended enum and one appended field, with a new golden fixture. `schema-breaking` stays green, and older clients ignore it.
- The hub sends two to three more bytes per connection.

## Alternatives considered

- **A new command (option 2):** a round trip and a command that only repeats what the handshake can say.
- **A capability string (option 3):** it changes the meaning of `Welcome.capabilities`.
- **Keep learning from refusals (option 4):** this is what #1107 does. The refused rows it leaves are the reason for this ADR.

## Open questions

1. **Hub not in team mode.** Should it send `PERSON_ROLE_OWNER`, as proposed, or `UNSPECIFIED`? Owner makes the one-person case behave
   like team mode's owner, with no special case in the windows.
2. **Project grants in `Welcome` too?** No: the scoped snapshot already says what a member may see. The role is the only missing fact.

## Build order (each its own PR, after acceptance)

1. **Schema:** the enum and the field, a new fixture, and `protocol.md`.
2. **Hub:** fill it, with a test per role, a node connection, and a role change that reconnects with the new value.
3. **Clients:** the terminal client (picker, pairing cards, `f` for a viewer), then the web (pairing cards, Usage, start actions), each
   with tests that no refused row is written for a member or viewer in the normal course. This closes #1099.

## Build status

- **Built:** steps 1 and 2 in #1139 (the schema, the fixtures in `schema/fixtures/person-role/`, the hub), and step 3 in the clients
  pull request that closes #1099.
- **Open questions answered as proposed.** A hub not in team mode sends `PERSON_ROLE_OWNER`, and `Welcome` carries no grants.
- **Delivery differs from the Decision's wording.** A window sends no `Hello`: it opens with `Subscribe`, and the hub built a `Welcome`
  for nodes only. A window-sent `Hello` would be refused by an older hub, so the hub sends the window an unasked `Welcome` as its
  **first frame**, before the snapshot. That `Welcome` has an empty capability list and an empty peer id, and
  `docs/specs/protocol.md` section 1 describes it. An older window ignores it, as it ignores every control frame it does not use. A
  newer window treats any other first frame as an older hub (`PERSON_ROLE_UNSPECIFIED`).
- **Clients.**
  - Both the web and the terminal client ask `PairStatus` (and the web `UsageReport`) only for the owner, and take the picker's role
    from the `Welcome`.
  - Neither sends a viewer's typing or terminal sizes. Sizes are refused for a viewer too, and the window sends them by itself.
  - The terminal client hides from a viewer the keys that change something (`y`/`n`, `s`, `f`, `x`, `u`, `d`). A key pressed anyway
    says so and sends nothing.
  - The web hides start, new terminal, close, resume, remove, prompts, options and Give.
- **Not built:** a viewer's answer buttons on requests in the web, and a chat tab's prompt line in the terminal client (#1140). The hub
  refuses and audits both, as before.
