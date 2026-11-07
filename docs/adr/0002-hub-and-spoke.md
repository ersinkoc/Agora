# 0002. Hub-and-spoke topology with autonomous nodes
Status: Accepted
Date: 2026-11-06

## Context
The user wants one window over all machines, centrally aggregated. Machines may sit behind NAT, or be
reachable only over SSH. Work must not stop if the central service is down.

## Decision
- Every machine runs a node; nodes connect outbound to one hub. Clients connect only to the hub.
- SSH bootstrap is supported for machines without an installed node service.
- Nodes are autonomous: agents keep running and events are buffered when the hub is unreachable.

## Consequences
- The hub is a coordination point, not an execution dependency.
- Event replay and idempotent event handling are required at the hub.
- Peer-to-peer node federation is out of scope.
