# 0065. Pairing: find a hub, enter a number, connected
Status: Proposed
Date: 2026-11-14

Not accepted. Nothing here may be built until the maintainer accepts it in a message of their own.
Origin: the maintainer: "I do not want to ask for a key with `agora hub login` again and again; it is my machine. Every node must connect to a hub easily, without a million steps: if there is a hub
somewhere on the network the node should find it, or I type its address; I say connect, it asks for the number shown on the hub's screen, I enter it, the handshake runs, done."
Replaces rung 2 of ADR-0062 (the join string stays as the fallback) and supersedes the typed sign-in codes for the owner's own devices.

## Context
Joining a machine today needs the hub's CA file, an enrollment code and a long command; signing in a browser needs a code from `agora hub login` each time (the browser session also ended after 12 hours or a closed tab: fixed in #450).
Both are the same problem: a new party must prove to the hub's owner that it is the one the owner means, without typing secrets across machines.

## Decision (proposed)
1. **The owner is never asked for a code on the hub's own machine.** `agora`/`agora web` open the interface already signed in (ADR-0062); sessions last 30 days and are extended on use (#450).
2. **Finding a hub.** A hub answers a small discovery request on the local network (UDP multicast and broadcast on a fixed port, `std` only, no new dependency): its name, address, port, CA fingerprint and whether it accepts new machines right now. A node's "Join" screen lists the hubs found; if none is found, the person types an address (`host[:port]`).
2a. A hub only answers discovery while it is **open for pairing** (a switch in its interface/CLI: "Add a machine" / `agora pair open`, closing by itself after 10 minutes), so a quiet hub is invisible and a rogue request gets nothing.
3. **Two ways to confirm, the person uses whichever screen is in front of them:**
   **(a) Approve on the hub (the default, no new dependency).** The new machine's pairing request shows up in the hub's interface as a **"Needs you" card and a notification** (the web window, the terminal client, and the push channel of ADR-0051 if configured): *"New machine 'laptop' (Windows, 192.168.1.50) wants to join. Number 482-193. [Approve] [Deny]"*. The same number is shown on the new machine's screen: the owner **compares the two numbers** and approves. (Without the comparison a card could be approved for the wrong request: the number is what ties the card to the machine in front of them.)
   **(b) Type the number (a later rung, needs the PAKE decision).** The hub shows a one-time number (6 digits, 5 minutes, 3 tries) and the person types it into the new machine's join screen, for the moments when no interface is open on the hub.
4. **The handshake.** For (a) it is a *short authentication string* exchange with commitment, built from what the tree already has (`ring`: ECDH P-256, HKDF, HMAC; no new crate): the new machine sends a commitment to its public key and a nonce first, the hub answers with its own, the new machine reveals, and both derive the same 6-digit number from the whole transcript; a man in the middle can make the two numbers match only by luck (1 in a million, and one attempt: a mismatch or a deny kills the pairing). After approval the hub sends its **CA certificate and fingerprint over the channel keyed by that exchange**, so the new machine pins the CA with no file to copy, and the hub issues and records its node certificate (today's enrollment, minus the typed code).
   For (b) a password-authenticated key exchange (SPAKE2) is needed so that a recorded handshake cannot be used to guess a 6-digit number offline: that is the one place a new dependency would come in (the RustCrypto `spake2` crate, MIT/Apache-2.0, no network or build script, pinned, `cargo deny` clean) and it is **decided separately, when (b) is built**.
5. **Device trust in the same step (ADR-0061).** The same pairing registers the person's interface devices and asks, on the machine, once: "Trust the owner's devices on this machine? [y/N]" with the fingerprints (so a new machine never needs a trust command typed).
6. **A browser on another machine** signs in the same way: the sign-in page shows "Waiting for the owner to approve… number 731-004"; the owner sees the card "Browser 'Chrome on Mac' (192.168.1.50) wants to sign in. Number 731-004. [Approve] [Deny]" in an interface they are already signed in to. No codes from the command line. (A browser keeps its 30-day session.)
7. **Nothing automatic is trusted:** a hub found on the network is only a candidate; nothing is shared with it before the number is accepted; the node's own policy applies as always; pairing never widens what a machine allows.

## Open questions for the maintainer
- Approve-on-the-hub with number comparison first (proposed), the typed number later? Or both at once?
- Discovery on by default only while "open for pairing" (proposed), or always on in the LAN? (Proposed: only while open.)
- Number length 6 digits, 5 minutes, 3 tries (proposed).

## Consequences
New `agora pair open|close|status` and the join screen (`agora join` with discovery), hub "Add a machine" card in web and TUI, a PAKE step before enrollment, a discovery responder in the hub (std UDP) and a finder in the node, (later, only for the typed number) the `spake2` dependency, tests (a hub and a node in-process, a mismatched number, a denied card, a replayed handshake, a MITM that cannot learn the CA), docs ("Connect a second machine in 30 seconds"), threat model (a LAN attacker can see that a hub is open for pairing and can try 3 numbers; the number is shown only to the owner; the pairing window closes itself).
