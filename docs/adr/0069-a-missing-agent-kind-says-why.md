# 0069. A missing agent kind says why, and a trust hint names the real binary
Status: Proposed
Date: 2026-11-14

Not accepted. Nothing here may be built until the maintainer accepts it in a message of their own.
Origin: the Windows agent's first-time-user findings (2026-11-14): "WrongStack: not installed" says nothing about what was looked for or where; a pasted `agora node trust …` fails on a machine where `agora` is not on PATH. The built-in manifests (PR #460) removed the main cause (a release node had no manifests); this closes the explanation gap with two additive fields.

## Context
`agent.kinds` tells a client each kind's name and whether it is `installed`. A greyed kind gives the person no way to fix it: it does not name the program the node looked for, and the start window's trust hint cannot name the binary the person has to run on the machine. Both facts are known to the node and are not secrets.

## Decision (proposed)
1. `AgentKind` gains an optional `program` field: the program name the node looks for (the manifest's `process.names[0]` or launch program, a bare name such as `wstack`, never a path). Clients show "This machine cannot find `wstack` in its PATH. Install it there, or start the node from a terminal where it runs." (the sentence already exists in `locales`). The node's PATH is never sent (it can hold private folder names).
2. The node's report gains an optional `binary` field: the file name and containing folder of its own executable **only when that folder is not on the node's PATH** (so the client can say "run it from that folder"); otherwise empty. A full path may contain a user name: clients show only the last two path components unless the person opens "Show the full path".
3. Both fields are optional and additive, covered by golden fixtures and a capability (`kinds.program`); an old node sends nothing and clients fall back to today's sentences.

## Consequences
Two proto fields and fixtures, `protocol.md`, the node filling them from data it already has, the web and TUI sentences. No behaviour change for starting agents.
