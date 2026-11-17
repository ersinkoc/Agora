# Security policy

Agora controls processes that execute code on your machines. We treat security reports as the highest
priority.

## Supported versions

Only the latest release receives security fixes. Update with `agora update` (or download the latest release) before
reporting, and say which version you tested.

## Report a vulnerability privately

Use GitHub private vulnerability reporting on this repository (the Security tab, "Report a vulnerability"). Do not open
public issues, pull requests or discussions for security problems.

Please include:

- the version (`agora --version`) and the operating systems of the hub and the node involved;
- what you did, what you expected, and what happened, with the smallest steps that reproduce it;
- what an attacker gains (read, write, run code, cross a machine or an approval) and what they need first;
- the output of `agora doctor` if it is relevant (it redacts secrets; check before you paste).

## What to expect

These are goals, not promises: we aim to acknowledge a report within 72 hours, to tell you whether we consider it a
vulnerability within about 7 days, and to ship a fix for a confirmed one as fast as its severity warrants. We will keep you
informed, credit you in the advisory if you wish, and ask that you keep the details private until a fixed release is out.

## Scope

In scope: the `agora` binary (hub, node, terminal client), the web window it serves, the desktop shell, the protocol and
the release process. Of particular interest are the things the threat model says must hold: secrets never leave a node, the
hub cannot widen a node's policy, approvals are signed by a device the node trusts, mesh messages are untrusted data, and a
confined agent stays confined.

Out of scope: an attacker who already has the owner's account on the machine, an agent the owner chose to run without
confinement doing what it was allowed to do, denial of service by a user the owner already trusted, and findings that need a
modified binary.

Design-level security and the known limits are in [`docs/security/threat-model.md`](docs/security/threat-model.md).

## Verify a release

Releases are signed. Download the binary with its `.sha256` and `.sig` files and check both with the key in
[`docs/security/release-key.pub`](docs/security/release-key.pub), from the folder that holds them:

```sh
f=agora-x86_64-unknown-linux-gnu   # or aarch64-unknown-linux-gnu, aarch64-apple-darwin, x86_64-apple-darwin, x86_64-pc-windows-msvc.exe
sha256sum -c $f.sha256
ssh-keygen -Y verify -f docs/security/release-key.pub -I agora-release -n file -s $f.sig < $f
```
