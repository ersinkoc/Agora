# 0022. Performance budgets are requirements
Status: Accepted
Date: 2026-11-07

## Context
The product must never feel slow, regardless of the number of agents, machines, projects, or clients.
Performance that is not measured degrades with every feature.

## Decision
- The budgets in `design/performance.md` are product requirements with the same weight as correctness.
- Every budget has a benchmark scenario; CI fails on regressions above 10% in hot-path scenarios.
- A change that cannot meet a budget needs an ADR that explicitly changes the budget.
- Phase exits include measured results for every budget relevant to that phase.

## Consequences
- A benchmark harness with simulated nodes, clients, and fake agents is built in Phase 0/1, before features.
- Some features will be designed around budgets (for example local-first clients, direct data path).
