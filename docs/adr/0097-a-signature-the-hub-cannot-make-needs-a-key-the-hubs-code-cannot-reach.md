# 0097. A signature the hub cannot make needs a key the hub's code cannot reach
Status: Proposed
Date: 2026-11-21

Touches ADR-0020 (approval authority), ADR-0028 (approval signatures), ADR-0041 (signed commands), ADR-0048 (the desktop app loads
the hub's page), ADR-0061 (device endorsements) and the threat model (T1, new T55). Found while checking an outside architecture
report (comms `log/2026-11-21-0929-linux-review-architecture-report-results.md`, item S1).

## Context
ADR-0020 promises that a compromised hub can neither approve an action nor widen a node's policy: approvals, grants (ADR-0041),
endorsements and revocations (ADR-0061) are signed by a device key and verified on the node.

The web client keeps that key as a non-extractable WebCrypto key in IndexedDB (ADR-0028, `apps/web/src/net/approval.ts`). The desktop
app does the same inside its webview, which loads the hub's own page (ADR-0048 decisions 1 and 4, `apps/desktop/ui/shell.js`). In both
cases the page's code comes from the hub, from the same origin as the key. Non-extractable stops the key from being copied out; it does
not stop code running in that origin from calling `crypto.subtle.sign` with it.

So a hub whose process or machine is taken over can serve a page that signs, with the person's own key and without them seeing it:
- approvals (`apps/web/src/net/approval.ts`) and skip-permission approvals (`skipApproval.ts`);
- grants for `signed_commands` (`grant.ts`), so `signed_commands` does not protect against this;
- **endorsements of new devices** (`devices.ts`): a key the attacker holds becomes trusted on every node the person's key is trusted on,
  for up to 90 days and renewable by use (ADR-0061). That trust outlives the compromise: cleaning the hub does not remove it; only a
  revocation or `agora node untrust` does.

Keys the hub's code cannot reach keep the promise: the terminal client's and the CLI's keys, and the owner's yes on the machine
(`agora node trust`, pairing). The threat model does not say any of this today: T1 reads as if every device signature were out of
the hub's reach.

## Decision
1. **Say it now (no code).** A new threat-model row (T55) and a sentence in T1: a key held in a page the hub serves is only as
   trustworthy as the hub; ADR-0020's guarantee holds for keys outside that page. The web Settings → Devices text names a browser
   device as "signs in this browser, trusted as much as your hub".
2. **The node knows where a key lives.** A trust line and an endorsement carry the key's holder kind: `page` (a page the hub serves:
   browser, today's desktop webview) or `local` (terminal client, CLI, a native desktop signer of decision 4). The owner sees it when
   trusting on the machine; an endorsement says the kind of the key it endorses, and a `page` key can never endorse a `local` one.
   A line without a kind is read as `page` when it was endorsed through the hub, `local` when the owner typed it on the machine.
3. **A policy line keeps hub-served keys from widening trust.** `<data>/policy` `page_keys = full | approve | none`:
   - `approve` (**proposed default for new nodes**): a `page` key may answer approvals and sign grants, but may not endorse a
     device. Endorsement — the one signature whose effect outlives the hub's compromise — needs a `local` key or the owner on the machine.
   - `full`: today's behaviour (the default for a node from before this ADR, so nothing breaks on update).
   - `none`: only `local` keys count; for a person who uses the browser to watch and the terminal client to decide.
4. **The desktop app gets a key the page cannot use (follow-up, its own PR).** The shell keeps a key in the operating system's
   keychain and signs only after its own native window, which the hub cannot draw, shows what is being signed (machine, agent and
   action for an approval; scope and time for a grant; device name and key fingerprint for an endorsement). The page asks; the shell
   decides. This amends ADR-0048 decision 4 (which left keychain keys to a separate decision) and keeps decision 1 (the shell still
   loads the hub's page for everything else).

Not chosen:
- **Bundling the web client in the desktop shell** (reversing ADR-0048 decision 1): the key would leave the hub's origin, but the
  shell would then have to update with every client change, and a page in the shell would still decide what the person is shown
  before signing.
- **Checking the page's code (hash pinning, a service worker that refuses new code):** a browser cannot verify the top-level page it
  loads, and a service worker can be replaced by the server; it would look like protection without being one.
- **Doing nothing but documenting:** leaves endorsements, whose effect outlives the compromise, signable by hub-served code.

## Consequences
- Decision 1 is docs only and can land on its own.
- Decisions 2 and 3 are a schema addition (a holder kind on a trust entry and on `DeviceEndorsement`, new fields only), node
  policy parsing, `agora node trust --kind`, and refusal codes with Fluent text. A browser-only person on a node with the new default
  can still approve and grant, but adding a second device needs the machine or the terminal client once.
- Decision 4 needs Tauri commands in `apps/desktop` (a native bridge that ADR-0048 decision 2 refused for the remote page: here it
  is one call, "sign this", whose content the shell parses and shows itself) and a keychain crate per OS behind `agora-platform`
  or inside `apps/desktop` only.
- T1's promise becomes true for every `local` key and for endorsements on nodes with `page_keys = approve` or `none`.

## Open questions for the maintainer
1. Default `page_keys` for new nodes: `approve` (proposed) or `full`?
2. Is decision 4 wanted now, or after the desktop app is signed per platform (ADR-0048 consequence 2)?
