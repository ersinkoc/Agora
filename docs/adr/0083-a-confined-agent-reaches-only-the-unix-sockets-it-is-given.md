# 0083. A confined agent reaches only the unix sockets it is given
Status: Accepted
Date: 2026-11-16
Accepted: 2026-11-17 (the maintainer's own reply in the merger chat, "1 kabul et geç", to the merger's list that named ADR-0081, ADR-0082, ADR-0083 and ADR-0084 as the decisions waiting for them). The open questions' proposed answers stand.
Origin: issue #547, from the third review of #477: under `sandbox = worktree` a confined agent can reach the user's ssh-agent socket and sign with their keys.


## Context
- **What the sandbox says about sockets today.** Only `worktree-offline`, and only on macOS, restricts them
  (`agora-platform/src/macos.rs`: `(deny network*)`, then the agent's own mesh socket and its private temp folder). The `worktree` profile on
  macOS says nothing about the network (the test `a_profile_that_is_not_offline_says_nothing_about_the_network`). The Linux Landlock ruleset
  (`linux.rs`, `ruleset`) handles file rights only: no network right, no scope. The kernel on the machine this was written on reports ABI 8.
  Windows confinement (ADR-0050) is a restricted token; it is off by default and was not measured for unix sockets.
- **The environment is inherited whole.** `Supervisor::launch` (`supervisor.rs`) starts from the node's own environment, adds the
  manifest's `[process] env`, the git variables (ADR-0078) and the temp folder, and removes only Claude Code session markers
  (`inherited_session_markers`). So `SSH_AUTH_SOCK`, `GPG_AGENT_INFO`, `DBUS_SESSION_BUS_ADDRESS`, `DOCKER_HOST`, `TMUX` reach a confined
  agent whenever the node has them (a node started from a login shell, or a service with the variable set).
- **The real impact, honestly.** With the ssh-agent socket a confined agent can have every key in the agent sign, so it can push and log
  in anywhere those keys are accepted. It cannot read the keys. This contradicts a sentence of ADR-0078 ("a confined agent has no
  credentials to push with anyway"). Scrubbing the variable does not remove the socket: the agent can find it (`/tmp/ssh-*/agent.*` on
  Linux, the launchd listener below `/private/tmp` on macOS: file names stay visible) and connect to it. It is not the only such
  socket, and the others are worse: the session D-Bus (`/run/user/<uid>/bus`) lets a process ask the user's service manager to start a
  program, which runs outside the sandbox (`systemd-run --user`); the docker socket is root on the machine; a tmux or screen server
  accepts keystrokes into the owner's shells; X11 (abstract and pathname sockets) reads and injects input; gpg-agent signs and decrypts.
  Most of these were not tried against a node here; they are what the sockets are documented to do. The one fact checked in this
  repository is that nothing stops the connection.
- **What would break if all unix sockets were refused.** `git` over ssh needs the agent only to use the owner's keys, and a confined agent
  cannot read `~/.ssh`, so key-based ssh from a confined agent works today only through the ssh-agent: that is the thing being closed.
  The agent's own mesh socket (`Sandbox::sockets`) and its private temp folder must stay. On macOS name resolution goes through a unix
  socket (`/var/run/mDNSResponder`), so it must be allowed or every network lookup fails (to be measured by the implementing PR; the
  profile cannot be written blind). On Linux `/dev/log` (syslog, optional), the name-service cache and systemd-resolved's socket are
  used by name lookups on some systems; glibc falls back where they are missing.

## Options
1. **Document only.** Cheap, leaves a hole `worktree` is sold as closing ("reduces what a confused agent can touch").
2. **Scrub the environment only.** Stops accidents, not an agent that looks for the socket. A good first step, not the decision.
3. **Allow-list of unix sockets, deny the rest** (chosen). Per system, with the mechanism each offers:
   - **macOS:** Seatbelt can say it today: `(deny network-outbound (remote unix-socket))` then `(allow ... (remote unix-socket (subpath ...)))`
     for the allow-list, the same forms `macos.rs` already writes for `worktree-offline`. No kernel limit.
   - **Linux:** two Landlock features, neither on every kernel. Scoping of abstract unix sockets (ABI 6, Linux 6.12) stops connecting to
     abstract sockets created outside the agent's own domain (X11's abstract socket, `unix:abstract=` D-Bus). A right for pathname sockets
     exists in a later ABI (9, as I understand the kernel documentation; the machine here reports 8 and cannot test it). On older
     kernels there is **no mechanism**: seccomp cannot read the path in `sockaddr`, and a network namespace would break everything else.
   - **Windows:** a restricted token already refuses a pipe or object whose ACL names no restricting SID; AF_UNIX sockets there were not
     measured. Not decided.
4. **Make `worktree` refuse to start where it cannot enforce this.** Too harsh for older Linux: `worktree` would stop working on kernels
   that run it today. Rejected as the default; an owner may ask for it.

## Decision (proposed)
1. **Scrub, on every system, now.** A confined agent's environment never carries `SSH_AUTH_SOCK`, `GPG_AGENT_INFO`,
   `DBUS_SESSION_BUS_ADDRESS`, `DOCKER_HOST`, `TMUX` or `TMUX_PANE` from the node (listed in `agora-node`, applied through the existing
   `env_remove` patterns; a manifest's own `[process] env` still wins). The owner may let one through with a policy line
   `sandbox.pass_env = NAME, ...`. This is honest as a first step only.
2. **Deny unix sockets outside an allow-list under `worktree` and `worktree-offline`, where the system can say it.**
   - macOS: the Seatbelt form above, in `agora-platform/src/macos.rs` only.
   - Linux: Landlock scoping of abstract sockets where ABI >= 6; pathname-socket control where the kernel reports it; both added
     to the ruleset in `linux.rs`, chosen by the version the kernel reports, as the truncate and refer rights already are. On a
     kernel with neither, nothing changes and `agora doctor` says "unix sockets are not restricted on this kernel (needs Linux X)".
3. **The allow-list.** The agent's own mesh socket; its worktree; its private temp folder; on macOS `/var/run/mDNSResponder` and the
   other system sockets the implementing PR measures a real agent to need; on Linux the same measured set. The owner adds to it with
   `sandbox.sockets = <path>, ...` (absolute or `~/`; relative paths dropped, as for `sandbox.read`). A node's own folders stay closed
   as today, and ADR-0082's token still guards the control socket.
4. **The sockets are fields on the existing `Sandbox`** (`sockets` already exists): no new type, no `cfg` outside `agora-platform`.
5. **Threat model wording** (no new T-id): the T8 row (agent escapes its worktree) gets a sentence saying that under `worktree` a
   confined agent can connect to any unix socket the user can (ssh-agent, gpg-agent, the session bus, docker, tmux, X11), that the
   environment scrub covers only the variables, and which systems enforce the allow-list once this is implemented. T16 keeps naming
   the control socket only. The same sentence is added now, in this PR, as a fact about today (it does not depend on the decision).
6. **`agora doctor`** names per machine: unix sockets restricted (yes / no, and why not) and which socket variables the node would have
   scrubbed.

## Not decided here
- The exact macOS and Linux allow-list: it must be measured with real agents (Claude Code, Codex) by the implementing PR.
- Windows named pipes and AF_UNIX under ADR-0050.
- TCP to `localhost` (a local proxy, an SSH tunnel on a port): `worktree` has no network rules; ADR-0047 `worktree-offline` is the answer.
- Whether `worktree` may one day refuse to start where the mechanism is missing (option 4).

## Consequences
- A confined agent can no longer sign with the owner's ssh-agent or start a program through the session bus, on macOS and on Linux
  kernels that have the rights. `git push` over ssh from a confined agent stops working if it relied on the agent; the owner's way is
  to push from their own checkout, or to list that one socket with `sandbox.sockets`, knowing that it hands over every key in the agent.
- On macOS a missing allow-list entry shows as a broken tool (DNS, a dev server using a unix socket in `/tmp`): the cost of an allow-list.
  The implementing PR measures common tools and ships the list in `docs/security/sandbox-recipes.md`.
- No hot-path impact: the rules are made once per agent start.
- Old Linux kernels keep the hole, and say so in `doctor`.

## Open questions (for the maintainer)
1. Is the environment scrub (decision 1) acceptable as the first step to ship alone? Proposed: yes.
2. Should the allow-list be the default under `worktree` on macOS, or only `worktree-offline` plus an opt-in `sandbox.sockets = restrict`? Proposed: default under `worktree`; the sandbox exists for this.
3. On a kernel that cannot restrict pathname sockets, run `worktree` anyway with the `doctor` note? Proposed: yes (option 4 stays an owner's choice).

## Built
Built on 2026-11-17 (issue #547). Step 1 on every system: `agora-node::sandbox` (`SOCKET_VARIABLES`, `sandbox.pass_env`, parsed by `parse_pass_env`
where one entry that is not a plain name empties the list; `sandbox.sockets` as paths), applied through `env_remove` at a confined terminal start;
an adapter agent's environment is an allow-list that never held these names, and `pass_env` does not change that (a choice this PR made). Step 2 in `agora-platform` (`Sandbox::confine_sockets`, `socket_confinement()`): Linux Landlock scoping of abstract sockets from ABI 6 (tested here, ABI 8). The kernel headers and the `landlock` ABIs available when this was built have no right for pathname sockets, so on Linux those are not restricted and `agora doctor` says so (`sockets-abstract`); nothing pretends otherwise. Open question 3 as proposed: `worktree` still starts on such a kernel. The allow-list is the agent's mesh socket,
its worktree, its private temp folder (macOS), the name resolver and the owner's `sandbox.sockets` (entries under the node's own folders, also through a link, are
ignored). **macOS step 2 is written but NOT switched on** (`agora_platform::macos::UNIX_SOCKET_DENY_VERIFIED = false`): on a real macOS CI runner the rule
`(deny network-outbound (remote unix-socket (subpath "/")))` with the allow-list made a confined `git commit` produce no output within 15 s, and the cause was not isolated;
a security rule that is not measured is not shipped. On a Mac a confined agent gets step 1 only and `agora doctor` says so. The two Seatbelt tests
are `#[ignore]`d: a Mac developer sets the constant to `true`, runs `cargo test -p agora-platform -- --ignored`, measures each rule form and the allow-list
(DNS, git, the agent's own mesh socket and bridge reconnect, other agents' sockets, the ADR-0082 control socket) and only then switches it on.
