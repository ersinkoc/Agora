# 0009. Schema format: Protocol Buffers as the IDL
Status: Accepted
Date: 2026-11-06
Accepted: 2026-11-07

## Context
ADR-0006 requires one schema generating Rust, TypeScript, Go, and Python, with append-only evolution, a
binary form for high-volume node traffic, and a JSON form for adapters and debugging.

Options:
- **Protocol Buffers.** Mature generators in all four languages; field numbers enforce append-only
  evolution; official JSON mapping. Cost: a code generator in the build.
- **JSON Schema.** Excellent for JSON, weak for binary encoding; Rust generators are less mature.
- **Cap'n Proto / FlatBuffers.** Zero-copy, but weaker TypeScript and Python ecosystems.
- **Custom IDL + own generator.** Full control and no external toolchain; months of work and a permanent
  maintenance burden before the first feature.

## Decision
Use Protocol Buffers (proto3) as the IDL. Generate Rust code at build time with a pure-Rust generator so
contributors do not need `protoc`. Use the canonical proto3 JSON mapping for the adapter protocol and debug
mode. Lint schema changes with a breaking-change checker in CI.

## Consequences
- Wire evolution rules come for free (never reuse field numbers, never change types).
- One build-time dependency chain for codegen; justified by four target languages.
- A custom IDL remains possible later because messages are defined declaratively.
