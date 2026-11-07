# 0008. Mesh via an MCP server injected into agents
Status: Accepted
Date: 2026-11-06

## Context
Agents need to coordinate (message, delegate, avoid editing the same files). Injecting keystrokes into another
agent's terminal is unreliable and unstructured. Most coding agents support MCP tools.

## Decision
Each node exposes a mesh MCP server to its local agents and injects it into the agent's MCP configuration at
launch. The hub brokers messages, tasks, leases, and handoffs across nodes.

## Consequences
- Coordination works across agent kinds and machines with one tool surface.
- Agents without MCP support receive messages through their integration level's input path as a fallback.
- Leases must be enforced at the node for file writes routed through the node; agents writing directly to
  disk are only advised, not blocked, unless sandboxing is added later.
