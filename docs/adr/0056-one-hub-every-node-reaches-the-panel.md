# 0056. One hub; every node machine can reach the hub's panel
Status: Accepted
Date: 2026-11-13
Accepted: 2026-11-13 (maintainer's explicit approval in chat: "0056'yı onaylıyorum"). The maintainer did not answer the open questions; the proposed answers below stand.

The maintainer's latest direction in chat: the hub stays single, and every machine that runs a node can open the hub's panel,
even when the hub is on another machine. This replaces the two earlier drafts of this ADR (hub and node on every machine, a node in
several hubs; and full peer-to-peer), which are not pursued.

## Context
To see everything today you open the hub's address in a browser, which means knowing the hub's address and certificate and having
a signed-in device. A person sitting at a node machine has a node but no panel there.

## Decision
1. **The hub stays single.** Boards, leases, approvals and the audit trail stay where they are (ADR-0039, ADR-0040, ADR-0053).
   No replication and no second master.
2. **Every node machine can open the hub's panel locally.** The node already holds the hub's address, its CA and this machine's
   enrollment. A command (`agora panel`, also offered by the desktop app) opens the hub's web app for the person at this machine,
   either by pointing the browser at the hub with the right trust set up, or through a loopback-only forwarder in the node.
   The panel is the same web app; only the way to reach it is added.
3. **The person still signs in.** Reaching the hub from a node machine is not a login. The hub's sign-in code and device approval
   (ADR-0020) apply as they do from any other browser; the node does not hold a session on the person's behalf. A node machine
   used only by agents has no panel running unless asked.
4. **No new listening port by default.** The forwarder, if used, binds to loopback only and runs only while `agora panel` runs.
5. **Without a hub connection the panel cannot work.** The node keeps agents running (autonomy), and `agora agent list` keeps
   showing local agents through the node socket.

## Open questions (proposed answers stand)
- Browser pointed straight at the hub (simple, needs the hub reachable and its CA trusted on this machine), or a loopback
  forwarder in the node (works through the node's existing link, one more component to secure)? Proposed: pointing straight at
  the hub, with `agora panel` printing how to trust the hub's CA on this machine (it changes no trust store itself).
- When the hub is itself on a machine you are not at, and unreachable from here, is a forwarder through the node needed?
- Does the desktop app bundle this, or only the command line?

## Consequences
Adds a small command (and possibly a loopback forwarder) and a trust step; no protocol message change and no change to the
hub's authority. A forwarder would be a new surface and needs the threat model (`docs/security/threat-model.md`) updated.

## Amendment A (Accepted, 2026-11-15): the panel opens through the node, with no certificate to import
Accepted: 2026-11-15 (the maintainer's explicit message in the merger's chat, answering the merger's message that named ADR-0056 Amendment A (#608) and ADR-0077 (#597) as the answers still owed: they said that the answer to everything they owed was the same, approval, and asked for the waiting work to be done). The open questions' proposed answers stand.
Origin: the maintainer, after setting up three machines by hand: "we shouldn't suffer this much on every machine; hub and nodes
should connect easily, and browsers on the network more easily". The chosen answer above (point the browser at the hub and print how
to trust its CA) is the `certutil` / keychain step that every browser on every machine has to repeat.

**Change:** `agora panel` (and the first run's "open the window" on a node machine, ADR-0062) opens the hub's web app at
`http://127.0.0.1:<port>/`, served by a **loopback forwarder in the node** that carries the connection to the hub over the node's
pinned-CA link. The browser talks to a loopback address, so it needs no certificate import; the forwarder is what checks the hub's
certificate, against the CA the node already pinned at enrolment. This is the design the desktop app already uses
(`agora-desktop-tunnel`, ADR-0048), and it is reused, not rewritten.

**Requirements:**
1. **One fixed port per hub, kept across runs** (stored next to the node's hub address). A device key and a sign-in belong to the
   page's origin, which includes the port (ADR-0048 found the same), so a new port each time would be a new device and another trust
   step: the pain this removes.
2. **Loopback only, and the Host header checked.** The forwarder binds `127.0.0.1` only and accepts a request only when its `Host` is
   `127.0.0.1:<port>` or `localhost:<port>`. Without that check, a website the person visits could use DNS rebinding to reach the hub
   through the forwarder. The same check is added to the desktop app's tunnel, which does not make it today.
3. **Not a login.** Reaching the panel this way is not a sign-in. Decision 3 stands: the person signs in as from any browser. With
   ADR-0065 decision 6 that means approving from an interface they are already signed in to, with no code typed.
4. **Only while asked for.** Decision 4 stands: the forwarder runs while `agora panel` (or the window it opened) runs, and no port is
   open otherwise.
5. **Threat model:** a row for the forwarder (loopback listener, Host check, what a local process can reach through it, which is no
   more than it could reach by talking to the hub itself).

**What it does not change:** the hub's authority, the protocol, and the node's policy. The hub still sees a TLS connection from the
node's pinned link; a browser on a machine with **no** node still needs the hub's CA (or the desktop app).

**Together with ADR-0065** (find a hub on the network, approve on the hub, the owner's devices trusted in the same step), this
removes the per-machine steps: no enrolment code to copy, no CA file, no certificate import, and no trust command.

**Built (Amendment A):** `agora panel` (`crates/agora/src/panelcmd.rs`) runs the desktop tunnel (`agora-desktop-tunnel`) from the
node's `link/` folder (hub address and pinned CA) on `127.0.0.1`. Port rule: the first run takes 7480, or any free port when that is
taken, and keeps it in `link/panel-port`; a later run that finds it taken refuses to start (`cli-panel-port-taken`) rather than move.
It first checks that the hub answers and presents a certificate from the pinned CA, so a hub that is down or foreign is said at once.
It runs until Ctrl+C and listens on nothing otherwise. The `Host` check (requirement 2) was already in the desktop tunnel (threat
model T22) when this was built; `Origin` is now checked in the same place (a request with an `Origin` other than the tunnel's own is
refused), which covers both. Threat model T27. A port that cannot be saved is reported (`cli-panel-port-unsaved`).
Built later (roadmap M7.6): the command opens the address in the browser through `agora_platform::open_url` when a person is at a
terminal on a machine with a screen (not without a terminal, in CI, over SSH, on Linux without a display, or with `AGORA_NO_BROWSER`
set), and prints it in any case; `agora web` keeps the same gates except the terminal (a desktop launcher or shortcut runs it with
none), and on a machine that only joined a hub runs the panel. The forwarder's port, and the desktop
app's tunnel port, are bound exclusively (`agora_platform::bind_loopback_exclusive`: `SO_EXCLUSIVEADDRUSE` on Windows; no
`SO_REUSEPORT` on Linux and macOS, checked), so no other program can bind beside it while it runs. Not built: the desktop app's own use
of a stored port, and plain `agora` on such a machine opening the panel by itself (it would hold the terminal for the forwarder).
Known gap (threat model T27): a fixed, predictable port can still be bound by another local user while the panel is not running.
To verify on Windows (compile-checked only here): with `SO_EXCLUSIVEADDRUSE`, Windows may keep the port unbindable while connections
the panel closed are in `TIME_WAIT`, so `agora panel` run again at once after Ctrl+C could report the port taken (and the desktop app
fall back to another port).
