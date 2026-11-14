# Agora on one page

Read this first. The details are in `architecture.md`, `glossary.md` and the ADRs; this page says **what each thing is, who owns it, and how you extend it**.
The rule for building: **small rungs, each one usable alone, none depending on a later one.**

## The four layers

```
 Interfaces (clients)      web · terminal (TUI) · desktop · CLI          show state, send commands; own nothing
        │  one protocol (schema/, signed commands, positioned events)
 Hub (one per group)       identity · event store · routing · approvals · mesh broker · serves the web window
        │  node ⇄ hub link (mutual TLS, reconnects by itself, buffers while away)
 Node (one per machine)    THE ENGINE: terminals · agent processes · folders/worktrees · git · files · confinement · ledger
        │  agent kinds: manifest (L1) · ACP (L2) · adapter process (L3)
 Agents                    claude-code · codex · wrongstack · opencode · agy · …   (helpers = agents started by agents)
```

Closing, killing or freezing an interface changes nothing below it. A node keeps its agents running without a hub.

## The nouns, one line each (who owns it)

| Thing | What it is | Owned by | Lives in |
| --- | --- | --- | --- |
| **Machine / node** | one computer's daemon; runs agents | node | its data folder (`~/.agora`) |
| **Hub** | the group's meeting point and memory | hub | its data folder (SQLite) |
| **Interface (client)** | a window onto the hub | nobody (stateless) | browser / terminal / app |
| **Project** | a group of folders/checkouts across machines (identity: `.agent-project.json` or git remote + root) | hub (derived) | events |
| **Workspace** | one folder on one machine an agent works in (shared, subfolder, non-git, or a new worktree) | node | the machine's disk |
| **Agent** | a running coding tool in a workspace: state (working/blocked/idle/done), usage, badge | node | the node's process table |
| **Agent kind** | what tool it is and how Agora talks to it | a manifest/adapter file | `manifests/agents/*.toml` |
| **Session** | the tool's own conversation id, used to resume | the tool; the node remembers it | the node's **ledger** (ADR-0060) |
| **Helper (sub-agent)** | an agent another agent asked to start, with your signed OK (ADR-0057) | node (+ your approval) | shown under its parent |
| **Message / task / lease** | agents talking: messages, shared board, file claims, handoff (the **mesh**) | the mesh broker (hub) + node mesh servers | events |
| **Approval** | an agent's request that needs a person, signed by your device and checked on the machine | node verifies, hub routes | your device key |
| **Device** | a browser/terminal key you sign with; a machine trusts it once (ADR-0061) | the machine's trust file | `trusted-devices` |
| **Launch option** | a named way to start a kind, e.g. "don't ask" (ADR-0059) | manifest + node policy | manifest |

## Where you extend it (each is data or a small file, never a core change)

| To add… | You write… | Level |
| --- | --- | --- |
| a coding tool that has a terminal UI | a manifest (how to launch, how to read its state from the screen, resume, MCP) | L1 |
| a tool that speaks ACP | a manifest line (`[structured] acp = {argv}`) | L2 |
| a tool with its own protocol | an adapter program speaking the adapter protocol | L3 |
| a new way to look at things | a client of the hub protocol (the web window, TUI and desktop are the same kind of thing) | client |
| a new rule on a machine | the node's policy file (what it allows; the hub can never widen it) | node |

## Rules that keep it simple

1. **One owner per fact.** If two places can disagree, one of them is wrong by design; the table above says which.
2. **Clients are replaceable.** Nothing may exist only in a client.
3. **The node never needs the hub to keep working.** The hub never acts for you: sensitive actions are signed by your device and verified on the machine.
4. **Messages from agents are data, never orders.**
5. **No user text in code:** everything a person reads is in `locales/` (en, tr).

## The build ladder for "type `agora` and it works" (ADR-0062)

Each rung ships alone, is usable on its own, and has its own tests. Do not start a rung before the one above is merged.

| Rung | What a person can do afterwards | Needs |
| --- | --- | --- |
| **0 (exists)** | start a hub and a node by hand; use any interface; agents survive any interface | — |
| **1 front door** | type `agora` on a fresh machine: it starts a hub **and** a node here (loopback), detached, and opens the interface signed in | machine.toml, detached start, a lock |
| **2 join** | on another machine paste ONE string (`agora join <string>`) printed by the hub: it joins, no CA file, no flags | join string, CA pin |
| **3 honest environment** | tools like `wstack`, `claude`, `codex` are found; a missing one says why | login-shell PATH captured |
| **4 coming back** | after a restart the agents that were running are resumed | ledger (ADR-0060) |
| **5 autostart** | it all starts when you log in (systemd user / LaunchAgent / logon task) | per-OS service |
| **6 care** | `agora status` / `down`, logs, restart-on-failure, version mismatch hints | — |
| parallel | trust without typed commands (ADR-0061), a proper terminal UI (ADR-0063) | — |
