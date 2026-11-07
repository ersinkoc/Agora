# Protocol schema

Source of truth for every protocol message (ADR-0006). Format: Protocol Buffers, proto3 (ADR-0009).
Generated code is never edited by hand and is not committed. Published messages are never changed; see
`docs/specs/protocol.md`.

## Layout

`schema/agora/v1/*.proto` holds protocol generation 1. A new generation gets a new directory (`v2/`).
Never reuse or renumber a field; add new fields and messages instead.

## Breaking changes

CI runs `agora-schema-check` against the target branch. Renaming, retyping, or removing a field, message,
enum, or enum value fails the build; adding new ones is fine. Run locally:
`cargo run -p agora-schema-check -- <base schema dir> schema`.

## Generating code

- **Rust:** automatic. `crates/agora-protocol/build.rs` compiles the schema with `protox`, a pure-Rust
  protobuf compiler, so `protoc` is not needed. Types are in `agora_protocol::v1`.
- **TypeScript:** `cd apps/web && npm ci && npm run gen` writes `apps/web/src/gen/` (via protobufjs).
