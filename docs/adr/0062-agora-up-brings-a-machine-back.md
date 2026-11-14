# 0062. `agora`: one command that just works (first run, daemons, interfaces, coming back)
Status: Accepted
Date: 2026-11-13
Accepted: 2026-11-14 (maintainer's explicit approval in chat: "0062'i onaylıyorum"). The open questions were not answered; the proposed answers stand (automatic resume default yes; first run shows the three choices; Windows logon task; at most 8 agents at once). Built in small rungs, each shipping alone (`docs/map.md`). Rung 2 (join) is built as the discovery-and-number pairing the maintainer asked for next (a separate ADR); the join string stays as the fallback.
Origin: the maintainer, 2026-11-13/14: "there is still something raw about the project: I want a system that simply works when I type `agora`. Whether I open a hub or join a hub,
whichever interface I want, one person types only `agora` and the system works: if it is a server there will be a server, a node, a hub, whatever it needs. Then I connect the machines to each other,
and on every machine, in any folder, I open claude, codex, wrongstack, opencode, whatever, and they keep running whether I close or open the interface: my interface must never freeze them or lose them."
This ADR replaces the earlier, narrower draft of 0062 (`agora up`, which stays inside it).

## Context
Today a person must know the parts: start a hub (`agora hub run --listen … --hub-dir … --name …`), copy its CA file and an enrollment code to another machine, run `agora node enroll …` and `agora node run …`,
trust each browser on each machine by hand (ADR-0061 fixes that), open the web page or `agora tui`, and after a reboot start everything again. Plain `agora` prints a usage page. The first real attempt of the
maintainer on three machines took a whole evening and still left two machines unusable (nodes started by agents with a reduced `PATH`, two hubs, a trust command typed on every machine). The pieces are good
(a node already outlives any client: closing a browser or a TUI does not touch its agents, which are the node's children; a hub is a durable store; nodes reconnect by themselves); what is missing is the front door.

## Decision
1. **`agora` with no arguments is the front door**, on Linux, macOS and Windows. It never prints a usage dump; `agora --help` still does.
2. **First run is a short guided choice** (full-screen in a terminal, plain prompts when not interactive), stored in the owner-only `machine.toml` (roles, addresses, names; never secrets):
   - **"Start Agora here"**: this machine becomes a hub **and** a node (its own). The hub listens on loopback only; one question: "Let other machines of yours connect? [y/N]" (yes = listen on the LAN, names/addresses
     detected for the certificate). Done in one step; the interface opens signed in.
   - **"Join an Agora"**: the person pastes **one join string** that the hub prints (`agora hub join-string`: hub address, the **CA fingerprint** pinned in it and a one-time enrollment code, e.g. `agora://192.168.1.246:7444/ca=sha256:…/code=ABCD-1234`).
     No CA file to copy, no separate flags: the machine pins the CA from the fingerprint, enrolls as a node and (ADR-0061) asks on this machine, once, whether to trust the owner's devices. `agora join <string>` does the same without the full-screen.
   - **"Just this machine, alone"**: as "Start here" without LAN listening.
3. **Later runs: `agora` makes sure what the machine remembers is up, then opens the interface.** Missing hub/node processes are started **detached** (they survive the terminal, the SSH session and the
   interface), their state is shown by `agora status`, and they are stopped only by `agora down`. The interface is chosen by the person: `agora` (the terminal client, ADR-0063), `agora web` (opens the browser on this machine's hub or the hub it joined),
   `agora desktop` where installed; `agora --ui web|tui|none` remembers a default. **Closing, killing or freezing an interface changes nothing** for hubs, nodes and agents: they are separate processes owned by no client (a test pins it:
   kill -9 a client with agents running and nothing happens to them).
4. **Agents come back after a restart** by the ledger of ADR-0060: agents that were running when the machine stopped are resumed in their folders with their saved sessions (or offered "Start again"), in the order they were started, at most 8 at a time;
   the survivor check of ADR-0060 runs before any automatic resume; options chosen with "Don't ask" (ADR-0059) come back asking. On the first restart the person is asked once: "Bring agents back automatically? [Y/n]" (proposed default **yes**, the maintainer wants everything back);
   the answer is stored and changeable. `agora up --resume ask|auto` overrides for a run. The hub can never turn automatic resume on.
5. **The node uses the person's real environment.** At `agora` time the node captures the user's login-shell `PATH` and relevant variables (never secrets: the allowlist of ADR "adapters keep the user's folders" applies to adapters; agents started by the node inherit what the user's own terminal would give them), stores it, and uses it for every detached or service-started node, so installed tools (claude, codex, wstack, opencode, agy) are found and the start dialog's "not installed" is honest; it says **why** ("`wstack` was not found in this machine's PATH: …").
6. **Autostart is part of the first run:** at the end, "Start Agora when you log in? [Y/n]" runs `agora service install` for the user (systemd user unit, launchd LaunchAgent with the installing shell's `PATH` and `KeepAlive`/`ThrottleInterval`, Windows logon task); `agora service remove|status` (status says when Login Items or a policy turned it off); platform code only in `agora-platform`. Some systems ask for permission: the command says what it will register first.
7. **Starting agents anywhere is one flow in every interface:** pick the machine, the folder (browse or type; the machine's known folders), the kind (every installed agent, with launch options of ADR-0059), start; the window/TUI lists the same kinds on every machine and explains missing ones. No command is typed on any machine (ADR-0061 makes the first trust a single yes on the machine at join time).
8. **Hardening of the daemon model:** detached processes log to the Agora folder (rotated), restart on failure once with a throttle (a supervisor shim or the service manager), refuse to start twice (a lock), and `agora status` shows pid, version, listen address, linked hub, agent count, last restart reason in one screen; a version mismatch between hub and node is shown with the way to update.

## Open questions (proposed answers stand)
- Default of the "Bring agents back automatically?" question: **yes** (proposed; you said everything should come back) or no?
- First-run order: show the three choices (proposed) or start "Start here" without asking when nothing is configured? (Proposed: show the choices; one keypress.)
- Windows autostart: logon task, no admin (proposed), or a service?
- Maximum agents resumed at once: 8 (proposed), the rest wait.
- Does the join string put the one-time code in the pasted text (proposed: yes; it expires in 10 minutes and works once) or ask for it separately?

## Consequences
`machine.toml`; `agora` front door and first-run flow; `agora join`, `agora hub join-string`, `agora status`, `agora up|down`, `agora service install|remove|status`, `agora web|desktop`, `--ui`; detached process management and locks; the captured login environment and an honest
"not installed" explanation; the ledger-driven resume at start (ADR-0060) with the survivor check first; per-OS service registration in `agora-platform`; the terminal client (ADR-0063) as the default interface; docs (a "Get started in two minutes" page) and the threat model
(autostart persistence is the owner's choice; automatic resume starts processes without a person present, under the node's confinement and policy; the join string carries a one-time secret that expires).
