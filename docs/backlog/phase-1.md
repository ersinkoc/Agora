# Phase 1 — Protocol and schema

Exit criteria (roadmap): generated Rust and TypeScript types compile; fixtures round-trip.
All tasks depend on P0-01.

### P1-01 Schema layout and codegen pipeline
- **Crates:** `agora-protocol`, `schema/`
- **Read:** ADR-0006, ADR-0009, `specs/protocol.md`
- **Do:** schema directory layout (`schema/agora/v1/*.proto`), Rust codegen at build time without `protoc`,
  TypeScript codegen script into `apps/web/src/gen/`.
- **Done when:** a sample message generates in both languages from one source.

### P1-02 Breaking-change check
- **Crates:** CI, `schema/`
- **Do:** CI job that fails when a published message changes field numbers, types, or names.
- **Done when:** a test PR renaming a field fails CI.

### P1-03 Envelope, streams, and framing
- **Crates:** `agora-protocol`
- **Read:** `specs/protocol.md` §2, ADR-0011
- **Do:** frame type, stream ids, encode/decode, max frame size, transport-agnostic framing trait.
- **Done when:** property tests round-trip arbitrary frames; malformed input returns `protocol.invalid_message`.

### P1-04 Handshake and capability negotiation
- **Crates:** `agora-protocol`
- **Read:** `specs/protocol.md` §1
- **Do:** Hello/Welcome/Resume messages, generation selection, capability intersection.
- **Done when:** tests cover no common generation, unknown capabilities ignored, resume vs snapshot.

### P1-05 Error model
- **Crates:** `agora-protocol`, `locales/en/errors.ftl`
- **Read:** `specs/error-codes.md`, ADR-0007
- **Do:** error type with stable codes and params; every code in the dictionary has an English Fluent message.
- **Done when:** a test enumerates all codes and checks each has a Fluent key.

### P1-06 Event and command messages (initial set)
- **Crates:** `agora-protocol`, `schema/`
- **Read:** `specs/protocol.md` §3–4
- **Do:** define the initial event families and command set; `Unknown` enum fallbacks; optional fields.
- **Done when:** every message has golden fixtures in binary and JSON.

### P1-07 Event sequencing and replay semantics
- **Crates:** `agora-core`, `agora-protocol`
- **Do:** producer sequence numbers, ack high-water mark, idempotent apply, in-memory reference implementation.
- **Done when:** tests cover duplicates, gaps, reorder, reconnect replay.

### P1-08 Adapter protocol messages
- **Crates:** `agora-protocol`, `schema/`
- **Read:** `specs/adapter-protocol.md`, ADR-0010
- **Do:** JSON-RPC envelope mapping and adapter messages; JSON fixtures.
- **Done when:** a minimal adapter written in a second language (TypeScript or Python) passes the fixture tests.

### P1-09 Domain model v0
- **Crates:** `agora-core`
- **Read:** `architecture.md` §3, `glossary.md`
- **Do:** Project (identity from remote + root commit), Node, Checkout, Worktree, Agent, Terminal, Task, Lease
  as plain data with invariants and unit tests. No I/O.
- **Done when:** project identity normalization tests pass for SSH/HTTPS/variant remote URLs.
