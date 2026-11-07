# 0010. Wire encoding per surface
Status: Proposed
Date: 2026-11-06

## Context
Node ↔ hub carries high-volume terminal output and events. Client ↔ hub serves browsers. Adapters are
written by third parties in any language and should not need a protobuf toolchain.

## Decision (proposed)
| Surface | Encoding |
| --- | --- |
| node ↔ hub | protobuf binary frames; terminal output as raw byte frames on dedicated streams |
| client ↔ hub | protobuf binary (generated TypeScript); JSON available with a debug capability |
| node ↔ L3 adapter | JSON-RPC 2.0 over stdio, messages from the same schema via proto3 JSON mapping |
| CLI/scripts | JSON over the local socket |

## Consequences
- Adapter authors use plain JSON-RPC, like ACP and MCP, which they likely already know.
- Two encodings of the same messages must stay in sync; enforced by golden fixtures in both forms.
