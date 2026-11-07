# 0019. Updates and version skew
Status: Accepted
Date: 2026-11-06
Accepted: 2026-11-07

## Context
Nodes on many machines will run different versions than the hub. Updating a node must not kill running agents.

## Decision
- The hub supports the current and previous protocol generation; nodes and clients advertise theirs.
- The hub distributes signed node binaries; nodes self-update when the user approves (or by policy).
- **Live handoff on update:** the old node process passes terminal handles (PTY master file descriptors on
  Unix via `SCM_RIGHTS`; duplicated ConPTY handles on Windows) and serialized runtime state to the new
  process, then exits. If handoff fails, the node falls back to restart with native agent resume.
- Release channels: stable and preview.

## Consequences
Version negotiation and handoff need dedicated tests; Windows handoff may ship later than Unix.

## Additions
- Node binaries are signed with the official release key, which nodes pin. The hub only distributes them and never signs;
  a compromised hub cannot push a malicious binary (consistent with ADR-0020).
- The first release ships restart with native agent resume; live handoff is added behind the same interface once tested.
