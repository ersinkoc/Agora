# 0004. Integration levels L0–L3; prefer structured state
Status: Accepted
Date: 2026-11-06

## Context
New coding agents appear constantly. Screen-scraping agent UIs breaks with every UI update and with
localized agent UIs. Many agents now speak the Agent Client Protocol (ACP); some do not.

## Decision
Four integration levels: L0 generic terminal, L1 declarative manifest, L2 ACP, L3 external adapter process
in any language. Reported state beats screen-derived state. Manifests and adapters are distributed through
a signed registry, not compiled into the binary.

## Consequences
- Supporting a new agent never requires a core release.
- Two UI modes for agents (native terminal and structured view), selectable per agent.
- The adapter protocol becomes a public, versioned API.
