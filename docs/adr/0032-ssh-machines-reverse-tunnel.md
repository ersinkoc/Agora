# 0032. Machines reachable only by SSH: the hub keeps a reverse tunnel
Status: Accepted
Date: 2026-11-08
Accepted: 2026-11-08 (maintainer's explicit approval in chat)

## Context
Many machines can be reached with `ssh` from where the hub runs but cannot dial the hub: no inbound route to the hub, a
corporate network, a server without a public name. ADR-0011 says the SSH bootstrap "uses the same framed protocol over the SSH
channel's stdio", and `agora node stdio` exists for that. But it bridges the node's local control socket, which is the
command protocol of the machine's owner, not the uplink a node speaks to the hub (events up, acks, routed commands, terminal
relays, mTLS identity). Making the hub speak the control protocol over stdio would mean a second way to be a node: a second
identity check, a second event path, and a hub that dials out instead of being dialed.

## Decision
Keep one way to be a node: it dials the hub with its certificate (ADR-0013, ADR-0015). For an SSH-only machine the hub makes
that dial possible by holding a **reverse tunnel** open:

`ssh -N -T -R 127.0.0.1:<port>:<hub listen address> <destination>`

The node on that machine is enrolled and run as usual, with the hub address `wss://localhost:<port>/`. Everything else (enrollment,
mTLS, events, acks, routed commands, relays, autonomy while the tunnel is down) is unchanged and needs no new code on the node or
new message in the protocol.

- **Where it lives.** A small dialer in `agora-hub` reads `<hub dir>/ssh-machines`, one machine per line: `<name> <ssh
  destination> [remote port]` (default port 17443; blank lines and `#` comments skipped, bad lines counted and ignored like
  `notify` and `trusted-devices`). `hub run` starts one supervised `ssh` child per line and restarts it with backoff
  (1 s doubling to 60 s, reset after a minute of uptime). It uses the system `ssh` binary, so keys, agents, `ProxyJump` and
  `~/.ssh/config` behave as the user already set them up. No SSH library is added.
- **Options the hub always passes:** `BatchMode=yes` (never prompt), `StrictHostKeyChecking=yes` (the host must already be in
  `known_hosts`; the hub never trusts a new host silently), `ExitOnForwardFailure=yes`, `ServerAliveInterval=15`,
  `ServerAliveCountMax=3`, `ClearAllForwardings=no`. The destination is passed after `--`, and is refused if it starts with `-`
  or contains whitespace, so a line cannot inject an option.
- **Bootstrap.** `agora hub ssh add NAME DESTINATION` appends the line and prints the two commands to run once on the machine
  (`agora node enroll --hub wss://localhost:<port> --ca <ca> --code <code> --id NAME`, then `agora node run`); the CA certificate
  is public. Installing the `agora` binary on the machine, and enrolling remotely through the tunnel, are separate follow-ups
  and out of scope here.
- **Status.** The dialer's state per machine (connecting, up, down with reason) is shown by `agora hub ssh` (list) and logged;
  whether the node itself is online remains what the node's own events say.

## Security
- The forwarded port is bound to the machine's loopback only. Another local user on that machine can reach it, but a connection
  without a valid, unrevoked client certificate can do nothing except try an enrollment code, and enrollment attempts are
  already rate limited and locked out hub-wide (ADR-0015). The threat model gets this row.
- The hub's server certificate must name `localhost` (it does by default; `--name` adds others), otherwise the node's pinned
  verification fails, which is the safe direction.
- The SSH account on the machine needs no shell privileges beyond being allowed to forward a port; the hub holds no password and
  never stores a key (it uses the user's agent or key files as configured).
- Nothing here widens node policy or lets the hub act without a signed approval (ADR-0020).

## Consequences
- No protocol, schema or node change; one hub module, one hub-dir file, two CLI commands (`hub ssh`, `hub ssh add`).
- ADR-0011's bootstrap sentence stays true for what it described (`node stdio` is kept for tools that want the control protocol over
  SSH), but the hub's SSH support uses a tunnel instead of stdio. This ADR does not supersede ADR-0011; it records that the
  stdio bridge is not the hub's path.
- Tests use a fake `ssh` (a script that records its arguments and listens on the forwarded port) so CI needs no SSH server;
  one manual check against a real host is listed in the PR.

## Alternatives considered
- Hub speaks the node control protocol over `ssh host agora node stdio`: second identity and event path, and the hub would need
  to invent acks and resume for a protocol that has neither. Rejected.
- An embedded SSH client library: a large dependency with its own key, agent and config handling that users would have to learn
  again. Rejected for the system `ssh`.
- Always require an inbound route (VPN, public name): fine where available and still the default; this only adds the SSH-only case.
