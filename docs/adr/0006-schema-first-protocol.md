# 0006. Schema-first, append-only protocol
Status: Accepted
Date: 2026-11-06

## Context
Node, hub, clients, adapters, and plugins will be written in several languages and released independently.
Incompatible wire changes would break deployed nodes and third-party adapters.

## Decision
- All messages are defined in `schema/` and generated for Rust, TypeScript, Go, and Python.
- Handshake negotiates protocol generation and capabilities.
- Published messages are immutable; changes add new messages or capabilities. New fields are optional;
  enums include `Unknown`.
- The concrete schema format is decided in a follow-up ADR (open question Q2).

## Consequences
- Golden fixtures for every published message; CI fails if one changes.
- Some duplication over time is accepted in exchange for compatibility.
