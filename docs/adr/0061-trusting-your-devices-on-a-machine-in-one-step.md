# 0061. Trusting your devices on a machine in one step
Status: Proposed
Date: 2026-11-13

Not accepted. Nothing here may be built until the maintainer accepts it in a message of their own.
Origin: the maintainer, after using the window end to end for the first time: other machines are connected, but starting an agent on any of them stops at
"This device is not trusted by that machine yet", and every machine needs a command typed on it.

## Context
Sensitive actions (starting an agent, approvals, grants) are signed by the person's device and verified on the node (ADR-0020), so the hub cannot act for the person. For a
node to verify a signature it must know the device's public key, and today the only way is `agora node trust DEVICE_ID PUBLIC_KEY_HEX` run **on that machine**, per
machine and per browser. With three machines and a new browser profile that is three commands with a 130-character key each; in practice a person never gets to start
an agent on another machine. The window's first screen offers "Start an agent" and the first real use ends in a dead end (the start window now at least shows the command with a Copy button).

## Decision (proposed)
1. **Devices are registered at sign-in.** When a browser signs in (code or passkey) it sends its public key and a name to the hub, which keeps a per-user list of registered devices
   (id, name, public key, created, last used). The hub's list is **only a list of candidates**: it never makes a node trust anything by itself.
2. **A node trusts the owner's devices at a moment the owner is at the machine.** `agora node enroll` (the owner types the enrollment code on that machine) and a new
   `agora node trust --from-hub` fetch the registered devices over the node's pinned-CA link and print each one as *name · fingerprint (first 8 bytes in groups) · registered when*,
   then ask on the machine's own terminal: "Trust these devices on this machine? [y/N]". `--yes` skips the question only for devices whose fingerprints are given on the
   command line. The consent is the owner's, given on the machine; a hub that lies about a key is stopped by the fingerprint the owner compares with the one the window shows
   under "This device".
3. **A new device later is endorsed by a device that is already trusted.** In the window: "Add this browser: approve it on another device of yours". The trusted device signs an
   *endorsement* of the new device's key (new device id, key, scope = this user's machines or a list, expiry, the endorser's id); the hub only carries it. A node accepts an endorsement
   only if it verifies against a device it already trusts and is not expired; it then trusts the new device for the endorsement's scope. The hub cannot forge an endorsement.
4. **Trust only narrows from the hub.** A revocation of a device (the owner revokes it from a trusted device, signed) is applied by nodes at once; the hub can never add trust that
   was not signed by a trusted device or confirmed at the machine. `agora node untrust` and the existing trust file keep working.
5. **The window says what is missing.** "This device" shows which machines trust it (a node reports its trusted device ids; the window compares), and the start window offers
   "Ask another device to approve" instead of a command when an endorsement route exists; the command with Copy stays as the fallback.

## Open questions for the maintainer
- Endorsement lifetime (proposed: 90 days, renewable by use) and scope (proposed: all of the user's machines, the owner may narrow).
- First device on a first machine: proposed answer 2 above (confirm at enrollment on the machine).
- Should the passkey (WebAuthn) be the device identity instead of a WebCrypto key (ADR-0020 chose WebCrypto)? (Proposed: keep WebCrypto; revisit separately.)
- Show names or only fingerprints in the machine's prompt? (Proposed: both, fingerprints are what is compared.)

## Consequences
Schema (device register message and list, endorsement message, node-reported trusted device ids, capabilities), hub store (devices per user, endorsement relay), node trust store
(endorsement verification, revocation, expiry), CLI (`node enroll` prompt, `node trust --from-hub`, `node untrust`), web (This device, Add this browser, start window), threat model
(a malicious hub offering a fake candidate; a stolen trusted device can endorse more devices: mitigated by expiry, revocation and the node's audit of endorsements).
