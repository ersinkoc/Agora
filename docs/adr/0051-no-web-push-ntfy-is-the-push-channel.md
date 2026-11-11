# 0051. No Web Push: ntfy and the PWA link are the push channel
Status: Accepted
Date: 2026-11-11

Accepted by the maintainer in the working session on 2026-11-11: "0050 değişiklik, 0051 kabul".

## Context
ADR-0046 made the phone client the installable web app and left Web Push as a possible later step: a notification that arrives with
the page closed. Web Push needs the hub to hold a VAPID key, store each browser's push subscription (an address at Mozilla, Google or
Apple's push service) and send an encrypted message through that service for every event. Today the hub already sends the same
notices to `ntfy` topics and webhooks (`agora-hub::deliver`) and a waiting-approval notice ends with a link that opens the approval in
the page (ADR-0046). The ntfy phone app delivers the notice with the screen off and opens the link on a tap.

## Decision
1. **Do not build Web Push now.** Phone push is ntfy (self-hosted or ntfy.sh), already shipped; the notice links into the page.
2. **Why not:** Web Push adds a persistent store of browser subscriptions, a signing key to protect, a dependency on third-party push
   services that learn when a hub notifies (the payload is encrypted, the timing is not), per-browser quirks (Safari needs the page
   installed), and a second path to test and keep secure, for a result ntfy already gives.
3. **Platform and privacy limits:** self-hosting ntfy does not remove every push intermediary. Instant notifications in the standard
   iOS app need an upstream server connected to Firebase/APNS (`upstream-base-url`); without it delivery may take hours.
   Poll requests expose notification timing and a message identifier to the upstream; the app fetches the content from the ntfy server.
   State this when recommending self-hosting; do not promise third-party-free instant iOS delivery. See
   [ntfy's iOS configuration](https://docs.ntfy.sh/config/#ios-instant-notifications).
4. **Revisit if** users report that they cannot install an ntfy client, or a team deployment needs per-person push; then it needs its
   own ADR with the privacy cost stated.

## Consequences
- The notifications design doc says phone push is ntfy; the roadmap line for Web Push is struck.
- Nothing to build, store or protect.
- A user who wants no third-party service uses in-page notifications while a tab is open, or self-hosted ntfy with a client and
  delivery mode that do not require an external push service. The standard iOS app's instant delivery does not meet that requirement.

## Alternatives considered
- **Web Push with VAPID:** see decision 2.
- **A native mobile app with its own push:** rejected by ADR-0046.
- **Email or SMS:** more third parties and more personal data than a topic URL.
