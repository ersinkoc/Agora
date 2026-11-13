# 0062. `agora up`: one command (or one login) brings a machine back, on every OS
Status: Proposed
Date: 2026-11-13

Not accepted. Nothing here may be built until the maintainer accepts it in a message of their own.
Origin: the maintainer: "regardless of the operating system, when I only start agora, shouldn't all the sessions that ran there and the hub-node connections come back by themselves?"

## Context
What a person must do today after a reboot or a sign-out, on each machine: start the hub (`agora hub run --listen … --hub-dir … --name …`), start the node
(`agora node run --project …`), and re-open every agent. Measured on 2026-11-13: a node reconnects to its hub by itself after a hub restart (verified on Windows and macOS), so the link
is already resilient; agents are not: they are processes of the node and die with it (Unix: they may even survive a node crash and then run unmanaged), nothing restarts a node after a reboot
(the macOS and Windows nodes run as detached background processes), and plain `agora` prints usage. ADR-0060 (accepted) gives a session ledger and a Resume button (`resume = "ask"`), ADR-0056 makes
every node machine reach the hub's panel, ADR-0059 stores launch options. Missing is the glue: a single entry point and autostart that make "turn it on" enough.

## Decision (proposed)
1. **The machine remembers what it is.** `agora node enroll`, `agora hub run` (first time) and the start window's settings write a small `machine.toml` in the Agora folder (owner-only): the roles
   (hub: listen address, names, data folder; node: projects, manifests folder; the hub it is enrolled with), never secrets (certificates and keys stay where they are).
2. **`agora up`** starts every role the machine remembers, in order (hub, then node), and returns when they are up; `agora down` stops them (agents end as they do today). Plain **`agora`** with no
   arguments is `agora up` when a `machine.toml` exists, and otherwise prints a short "get started" (create a hub / join a hub) instead of the usage dump. Same behaviour on Linux, macOS and Windows.
3. **After a start the node brings agents back.** It reads the ledger (ADR-0060): agents that were running when the machine stopped (no clean user stop, no exit by themselves) are resumed
   with their saved session (manifest `[resume]` argv or ACP `session/load`), in their folders, **in the order they were started**, and shown as such in the window ("came back after a restart").
   Agents that cannot resume (no native session id, folder gone) appear as interrupted with Start again. What was started with "Don't ask" (ADR-0059) is re-checked against the node's policy and,
   by default, comes back **asking** (the option is not re-applied silently).
4. **Policy.** ADR-0060's `resume = "ask"` stays the default. `agora up` asks once on first use, on the machine's terminal: "Bring agents back automatically after a restart? [y/N]" and writes the answer
   (`resume = auto` with the kinds and folders the owner currently uses, shown in the question). `agora up --resume ask|auto` overrides for one run. The hub can never turn `auto` on.
5. **Autostart is one command per machine:** `agora service install` registers `agora up` for the user (systemd user unit, launchd LaunchAgent, Windows logon task), `agora service remove`
   undoes it, `agora service status` says what is registered; platform code only in `agora-platform`. Installing it is an explicit act of the owner at the machine (some systems ask for permission:
   the command says what it is about to register before it does).
6. **The hub side needs nothing more:** a hub restarts from its data folder and nodes reconnect (verified); `agora up` on the hub machine starts the hub first.

## Open questions for the maintainer
- Default for the first-use question: proposed **no** (ask on the first restart, offer "always" in the window), or **yes** (auto), since you said all sessions should come back?
- Should `agora up` also start agents that were running when the machine stopped even if the user never resumed them before (the plain meaning of "bring everything back")? (Proposed: yes, with `resume = auto`.)
- Windows: logon task (per user, no admin) or a service (survives sign-out, needs admin)? (Proposed: logon task first.)
- After many restarts, how many agents may come back at once (each costs memory and an agent account's rate limits)? (Proposed: at most 8 at a time, the rest wait.)

## Consequences
New `machine.toml`, `agora up/down`, `agora service ...`, a changed no-argument behaviour, a node start phase that resumes the ledger (ADR-0060), per-OS service registration in `agora-platform`, a window
"came back after a restart" marker, threat model (autostart persistence is the owner's explicit choice; `auto` resume restarts agent processes without a person present: the node's confinement and policy apply).
