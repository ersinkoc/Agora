# 0082. A confined agent cannot use the node's control socket
Status: Accepted
Date: 2026-11-16
Accepted: 2026-11-17 (the maintainer's own reply in the merger chat, "1 kabul et geç", to the merger's list that named ADR-0081, ADR-0082, ADR-0083 and ADR-0084 as the decisions waiting for them). The open questions' proposed answers stand.
Origin: issue #440 (T16), found in the ADR-0059 security review; its Windows half was measured on 2026-11-14.


## Context
- The node's control socket (`crates/agora-node/src/control.rs`, `ControlServer::start`, `local_listen` in `crates/agora-platform/src/unix.rs`)
  is owner-only by file mode (0600; a named pipe with an owner-only descriptor on Windows). The accept loop checks nothing about the peer: the
  module comment says "the socket is owner-only, so no further authentication is needed locally". Every connection may send every command
  (`AgentStart` with `launch_options`, `AgentLaunch`, `AgentStop`, `AgentPrompt`, `TerminalInput`, `ApprovalResolve`, ...), unsigned: the
  device-signature check belongs to the hub-routed path only.
- A confined agent (ADR-0047) runs as the same user, so the file mode does not stop it. What stops it depends on the system:
  - **Windows:** measured, nothing to build. The pipe's descriptor is `D:P(A;;GA;;;OW)(A;;GA;;;SY)` (`OWNER_ONLY`, `windows.rs`); the agent's
    restricted token (restricting SIDs: its own, Everyone, Users, logon) and Low integrity each refuse the open. Windows confinement is
    still switched off by default (ADR-0050).
  - **macOS:** done (#475, #477). The Seatbelt profile closes the node's whole home, including a unix-socket connection to the control
    socket, which Seatbelt counts as `network-outbound` (`agora-platform/src/macos.rs`, `deny` and `reopen`).
  - **Linux:** open. The Landlock ruleset (`linux.rs`, `ruleset`) handles file rights only; connecting to an existing pathname socket is
    not a file right on the ABIs we use, so a grant that does not name the node's home still lets `connect(2)` through. `Sandbox::deny`
    is ignored on Linux (its doc comment says so). Measured here: this kernel reports Landlock ABI 8.
- What a confined agent on Linux can do through it, honestly. It cannot leave its file confinement this way: the sandbox level, the
  folders and the policy (`skip_permissions`, limits) are decided by the node's policy file, not by the request, so an agent it starts is
  confined too, and `skip_permissions = forbid` still refuses. What it can do is act as the owner of the node without the owner:
  start agents with a launch option no signed decision covers (the bypass of "helpers never inherit", ADR-0059 decision 5, and of the
  signed approval of ADR-0057/0020), stop agents, and write input into another agent's terminal (`TerminalInput`, by id), which is how one
  confined agent drives another that has a wider folder or another project. Plain terminals are the owner's own unconfined shells
  (ADR-0047); the code takes any terminal id, so an id it learned would be a way out, but `AgentList` lists agents only, and whether a
  plain terminal's id can be learned from the socket was not measured.
- Four ideas were on the issue: keep the socket out of reach, deny it per system, identify the peer (credentials, process ancestry), or
  refuse a node-supervised process tree.

## Options
1. **Move the socket into a private folder.** Useless on Linux: Landlock (ABI up to 8) does not check the path on `connect`, so a folder
   the agent may not read or traverse still lets it connect. Already done where it works (macOS).
2. **Landlock's newer unix-socket rights.** The kernel's Landlock documentation lists scoping of *abstract* unix sockets (ABI 6) and, in
   a later ABI (9, as I understand the kernel documentation; not verifiable here, the kernel reports 8), a right for resolving a
   *pathname* socket. That would close this on new kernels only, and needs the allow-list of ADR-0083. Right as an extra layer, not
   enough alone: kernels in the field are older.
3. **Peer credentials and ancestry** (`SO_PEERCRED` pid, then walk `/proc/<pid>/stat` to see if an agent's process is an ancestor).
   Rejected: the node supervises the owner's own terminals too, so "descends from the node" cannot mean "refuse"; descending from a
   particular agent is defeated by a double fork (the orphan is re-parented to init or a subreaper), and a pid read from the kernel after
   the fact can be reused. `NoNewPrivs` in `/proc/<pid>/status` (the node sets it on confined agents) marks confinement, but a CLI run
   by a service with `NoNewPrivileges=yes` has it too: false refusals of the owner.
4. **seccomp** on `connect`: cannot read the path behind the pointer in `sockaddr`. Rejected.
5. **A secret the confined agent cannot read** (chosen, below). It uses the guarantee the sandbox already gives: the node's home
   (device keys, trust file) is not readable by a confined agent. The same guarantee protects a token kept in it.

## Decision (proposed)
1. **A control token.** At start the node makes 32 random bytes and writes them hex-encoded to `<home>/control.token` (for `--socket
   <path>` without a home: `<path>.token`) through the existing `write_private` (0600 from creation, an owner-only ACL on Windows),
   replacing the file at every start. It is compared in constant time, never logged, never in an event, never in an environment variable,
   never in a command line, never sent to the hub.
2. **The first request on a connection proves it.** A new, append-only command `ControlAuth { token }` is added to `schema/` (a schema
   change: this ADR is the record; the golden fixture is new, none is edited). On a node that enforces the token (decision 3), every other
   command answers `auth.unauthenticated` (existing code) until `ControlAuth` has succeeded on that connection; a wrong token answers the
   same code and ends the connection. `agora-node`'s `Client::connect` reads the token file and sends `ControlAuth` itself, so every
   `agora` subcommand keeps its flags and output.
3. **When it is enforced.** The node enforces the token when its policy confines agents (`sandbox` is not `off`) on **Linux**, the
   only system where nothing else keeps the socket from a confined agent. On macOS and Windows the existing barrier stays, and the token
   is not required (it may be turned on there later by a policy line; not decided here). A node with `sandbox = off` has no confined
   agents and changes nothing for its owner.
4. **The token must be out of the agent's reach, or the start is refused.** `Sandbox::deny` is not enforceable under Landlock (it only
   allows). So when the node confines agents on Linux, it refuses to start an agent whose read or write grants contain the folder that
   holds the node's home or the token file (a node home under `/tmp` with `/tmp` granted): `agent.sandbox_unavailable`, a new `reason =
   control_token_reachable`, as the macOS `protected_path_unresolved` does. `agora doctor` names it before any start.
5. **Platform code.** Only the file creation (already `agora-platform::write_private`) is platform-specific. The token, the handshake
   and the check are in `agora-node` and `agora-protocol`, with no `cfg`.
6. **Wording of a refused open on Windows (the follow-up on #440).** `agora` tells "the node refused the connection" (permission denied)
   from "no node is listening" (not found), with two stable keys in `locales/`. The Windows open that today reads "No node is listening"
   because the pipe refused it is the case.
7. **Landlock's newer rights, later.** When a kernel reports the pathname-socket right, ADR-0083's allow-list is the place to use it; it
   would also protect a node that does not enforce the token. Not part of this decision.

## Not decided here
- Requiring the token on macOS and Windows, or on `sandbox = off` nodes.
- A per-command split (which commands an agent may send). The hub path and the mesh already have their own rules (ADR-0020, ADR-0057).
- Unix sockets other than the control socket (ssh-agent, D-Bus, docker): ADR-0083.

## Consequences
- Compatibility: a new `agora` on an old node: the node does not know `ControlAuth`; the client treats the capability-missing answer as
  "no token needed" and carries on. An old `agora` on a new confining Linux node is refused with `auth.unauthenticated`; the node
  updates in place with its CLI (ADR-0075), so both are the same binary in the normal case. Scripts that talk to the socket without the
  client library must read the token file; the socket stays a local, owner-only interface.
- A same-user program that is not confined (a terminal, a cron job, the owner's editor plugin) reads the file and works as before:
  the token separates *confined* processes from the owner, nothing more.
- One file written per node start, one request per connection: no hot path, no benchmark.
- Residual: a confined agent can still *connect*; it just cannot do anything. A flood of connections is limited by the accept loop's
  thread-per-connection model as it is today (not changed here).
- The threat-model row T16 changes from "open on Linux" to "mitigated by a token" when implemented; the change goes into the implementing PR.

## Open questions (for the maintainer)
1. Token on Linux only when the node confines agents (decision 3)? Proposed: yes.
2. Refuse the start when the node home is reachable (decision 4), even though a home under `/tmp` was usable before? Proposed: yes; tests use a home outside any granted folder.
3. Reuse `auth.unauthenticated` instead of a new code? Proposed: yes.

## Built
Built on 2026-11-17 (issue #440). The command is `ControlAuth` (`CommandRequest` field 101, golden fixture `agora.v1.ControlAuth` added). The token
is made in memory by `ControlToken::generate` (`agora-node/src/control_token.rs`) and written by `ControlServer::start` once `local_listen` holds
the node's lock, before the accept loop; the file is removed when the server stops. Enforcement is the platform constant
`CONTROL_TOKEN_REQUIRED` (Linux) together with `sandbox != off`; it is a constant in `agora-platform`, so there is no `cfg` outside it. Decision 4 is
`SandboxPolicy::token_reachable` (the same by-name-and-by-target check as `git_config`'s `under_a_grant`, failing closed), applied to the final
profile right before the spawn and by `agora doctor` on the static grants. Choices the text left open: the token path for a socket not named
`node.sock` is `<socket>.token`; a node that does not require the token answers `ok` to any `ControlAuth` (so a stale file from an earlier
confining run never locks the owner out); the client retries once on a new connection when the file changed after a refusal (a node starting at
that moment); `agora node stdio` presents the token itself and then carries the proved connection; a routed (hub) `ControlAuth` is refused; the
refused-connection wording of decision 6 was already in place (`cli-node-refused`, the `PermissionDenied` mapping) and is unchanged, plus a new
`cli-node-needs-token` for a client that had no token to show. Verified on macOS and Windows by reading only: macOS `deny`/`reopen` in
`macos.rs` still close the home including the socket, and the Windows pipe keeps `OWNER_ONLY`; neither was run in this change.
