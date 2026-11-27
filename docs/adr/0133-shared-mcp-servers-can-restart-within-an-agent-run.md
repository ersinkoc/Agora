# 0133. Shared MCP servers can restart within an agent run
Status: Proposed
Date: 2026-11-27

## Context

ADR-0122 distributes signed, scoped skills and MCP definitions. Its Amendment A gives a local program
server a secret through a node launcher and a single-use launch ID. A crash or transport reconnect
can leave a live agent unable to restart its server. A holder-backed agent can also outlive the node
that issued the launch ID. URL servers with secret headers currently fail closed. The node needs a
way to handle those lifecycles while preserving the signing, policy and secret boundaries.

## Decision

1. Keep an immutable launch binding to the exact verified item version, project, kind and agent run.
   A node-operated launcher requests a server instance over the run's own local endpoint. The node
   authenticates the endpoint, rechecks the run and binding, and rechecks currently trusted signer,
   scope, node policy and secret availability before each instance. A replacement item version
   requires an agent restart. The grant for one item never authorizes another item or run.
2. Replace the single-use launch ID with a run-scoped reference to that binding. The node admits at
   most one live instance for an item in a run. It issues a fresh instance only after a clean close
   or confirmed exit, with a five-second minimum between failed starts and at most six starts per
   minute per binding. A concurrent request, exhausted budget or uncertain liveness has a stable
   refusal. On node restart a holder-backed run may recover its binding only after the node proves
   continuity of that exact run and reconciles the prior instance. If the platform cannot prove that,
   the agent must restart. Keep binding and liveness metadata in node-owned state with no secret
   values. Remove it when the run ends.
3. Keep the approved program, arguments and secret names under the signed item and node policy.
   Resolve values on the node, deliver program secrets only to the approved process environment,
   and apply the existing environment restrictions. A process running under the same OS identity
   retains the access that ADR-0122 Amendment A documents. On observed removal, loss of signer
   trust or policy refusal, reject new instances and close any instance the launcher still controls.
   A remote removal becomes effective when the node receives and verifies it. When the node
   cannot refresh trust, it can keep a running server but refuse a fresh secret-bearing instance
   until it can verify the current item. Existing agents and unrelated tools continue to run.
4. For a URL server that needs secret headers, use a node-owned MCP stdio-to-HTTP bridge as a separate
   implementation step. The signed HTTPS endpoint is the sole destination. The node locally opts
   into secret-bearing URLs by exact origin and separately allows the secret names. Resolve headers
   on the node. Forbid redirects, forwarding credentials to another origin, prohibited destinations
   and secrets in configuration, command lines, diagnostics or protocol recordings. Reapply outbound
   policy after name resolution and bound connection time, response size and in-flight requests.
   Apply the original agent's outbound restrictions as well. Keep the bridge unavailable for an
   offline confinement profile or where the platform cannot enforce the applicable restrictions.
5. Record launch, refusal and termination outcomes on the node with item, run and reason, without
   values or resolved headers. The agent's existing started-with list describes its launch. Add
   any live status fields only after a separate schema decision. Preserve the existing per-kind
   manifest mechanism. Verify each additional kind's configuration path on a supported release
   before enabling it. This decision succeeds the single-use and secret-URL refusal passages of
   Amendment A as the respective implementation steps ship.

## Alternatives

- Keep a single-use ID and restart the agent after any server failure. This has a small state surface
  and forces an unrelated session interruption.
- Pass secrets in native client configuration or flags. Those locations outlive the launch or
  expose values to other readers. A node-owned launcher keeps the credential use scoped.
- Use a hub proxy. That would move credentials or the privileged outbound connection across the
  node boundary.

## Consequences

The node gains a bounded per-run lifecycle record and a local audit trail. Program restarts can
ship first. The URL bridge requires its own networking review and platform evidence before enablement.

## Verification

Use fake local program and HTTPS servers and generated secrets. Check concurrent requests, expired
bindings, crash and retry, same-run holder recovery, replacement runs, removal, policy change,
recovery without a fresh trust check and denial when instance liveness is unknown. Search launch
arguments, user configuration, logs, audit records and hub frames for the generated values. Exercise
redirect and destination refusals, bounded requests and bridge shutdown with a synthetic endpoint.
Run the launcher cases on Linux, macOS and Windows for each supported lifecycle path.
