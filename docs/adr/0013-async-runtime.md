# 0013. Async runtime
Status: Accepted
Date: 2026-11-06
Accepted: 2026-11-07

## Decision
Use tokio for networking, timers, and orchestration in node and hub. PTY read loops run on dedicated OS
threads that hand bytes to tokio through bounded channels, so a busy terminal cannot starve the runtime.
`agora-core` stays runtime-free.

## Consequences
Hot paths are isolated and measurable; the domain model remains testable without async.
