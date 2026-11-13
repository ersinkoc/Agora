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
3. **The number.** When the person starts pairing from the node, the hub's interface shows **"New machine 'laptop' (192.168.1.50) wants to join: number 482-193"**. The person types that number into the node's join screen (or the other way round: the node shows the number and the person types it on the hub, whichever screen they are looking at).
   The number is a one-time PIN with 6 digits, valid 5 minutes, 3 wrong tries then dead.
4. **The handshake is a password-authenticated key exchange (SPAKE2), not a PIN sent over the wire.** The PIN only proves "I am the one you showed the number to"; an attacker on the network learns nothing usable (no offline guessing of a 6-digit number from the recorded handshake). On success both sides hold a session key and the hub **sends its CA certificate and fingerprint over that authenticated channel**, so the node pins the CA with no file to copy, and the hub issues the node's certificate and records it (the existing enrollment, minus the typed code). **This needs one new dependency**: the RustCrypto `spake2` crate (MIT/Apache-2.0, no network or build script, pure Rust, audited by the RustCrypto project) with a SHA-256 hash and the P-256 or Ed25519 group; pinned to an exact version at least two weeks old, `cargo deny` clean; a design that does not use a PAKE (a short number compared on both screens) is rejected because a 6-digit number is guessable offline.
5. **Device trust in the same step (ADR-0061).** The same pairing registers the person's interface devices and asks, on the machine, once: "Trust the owner's devices on this machine? [y/N]" with the fingerprints (so a new machine never needs a trust command typed).
6. **A browser on another machine** signs in the same way: the sign-in page shows "Pair this browser: ask the owner for the number shown on the hub", the owner sees "Browser 'Chrome on Mac' (192.168.1.50) wants to sign in: number 731-004"; the number is entered into the page. No codes from the command line. (A browser keeps its 30-day session.)
7. **Nothing automatic is trusted:** a hub found on the network is only a candidate; nothing is shared with it before the number is accepted; the node's own policy applies as always; pairing never widens what a machine allows.

## Open questions for the maintainer
- OK to add the `spake2` dependency (point 4)? That is the decision this ADR needs.
- Discovery on by default only while "open for pairing" (proposed), or always on in the LAN? (Proposed: only while open.)
- Number length 6 digits, 5 minutes, 3 tries (proposed).

## Consequences
New `agora pair open|close|status` and the join screen (`agora join` with discovery), hub "Add a machine" card in web and TUI, a PAKE step before enrollment, a discovery responder in the hub (std UDP) and a finder in the node, the `spake2` dependency, tests (a hub and a node in-process, a wrong number, a replayed handshake, a MITM that cannot learn the CA), docs ("Connect a second machine in 30 seconds"), threat model (a LAN attacker can see that a hub is open for pairing and can try 3 numbers; the number is shown only to the owner; the pairing window closes itself).
