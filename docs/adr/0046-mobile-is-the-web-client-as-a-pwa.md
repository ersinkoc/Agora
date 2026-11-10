# 0046. Mobile is the web client, installable as a PWA
Status: Proposed
Date: 2026-11-10

## Context
Open question Q14 asks whether mobile is a native app or a PWA. The approvals UX (`docs/ux/approvals.md`) calls approvals "the main
mobile use case", and ADR-0020/0028 already require a device key per client. The web client is mobile-first in layout, signs
approvals with a non-extractable WebCrypto key (ADR-0015/0028) and is served by the hub over TLS. The hub already has a notifier
(ntfy and webhook) that tells the owner when something waits. What is missing is the decision, and the small pieces that make the
web client installable and the notification reach the approval directly.

## Decision
1. **No native mobile app in Phase 7.** Mobile is the existing web client, installable to the home screen. A native app is
   reconsidered only if a platform limit below blocks a stated need, with its own ADR.
2. **Installable:** the hub serves `manifest.webmanifest` (name from the locale strings, `display: standalone`, theme colours from the
   design tokens, icons derived from the existing mark) and the page links it. No service worker in this step, so nothing is cached
   and a stale client cannot outlive an update; the page works online only, as today.
3. **Notifications reach the approval:** the notifier's message for a waiting approval carries a link to `/#/approvals/<id>` on the
   hub's public address (the owner sets it once; without one the message has no link, as today). Opening it shows the approval and
   asks for the device key as any approval does. A notification is never an approval: tapping a button in it cannot allow anything.
4. **Web Push is not part of this ADR.** It needs a push service, VAPID keys and stored subscriptions on the hub (new state, a new
   outside party); if ntfy proves too weak it gets its own ADR.
5. **Device keys on phones:** where the browser cannot keep a non-extractable key (some in-app browsers), the page says so and
   refuses to sign instead of falling back to an exportable key.

## Consequences
- One client to build, test and translate; mobile quality is the web client's quality, judged in the same screenshot run (narrow width).
- Without a service worker there is no offline use and no push on iOS before the user installs the page; acceptable for approvals,
  which need the network anyway.
- The hub's public address becomes configuration the notifier needs; a wrong one only breaks the link, not delivery.
- Reversible: the manifest and the link are additive.

## Alternatives considered
- **Native apps (iOS and Android):** two codebases and store review for a signing key that WebCrypto already provides.
- **Tauri mobile:** young for mobile, and the same web client inside a shell with extra build targets.
- **Web Push now:** the best phone experience, but new hub state and a dependency on browser vendors' push services.
