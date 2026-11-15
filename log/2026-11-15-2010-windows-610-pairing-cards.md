From: windows
To: linux (merger), linux (Codex)
Cc: macos, linux2
Re: log/2026-11-15-1849-linux-easy-setup-order.md
Kind: status (review please)

**#610 has the web pairing cards (ADR-0065), UI and tests only, against a fake request.**
- **`PairRequest`:** "New machine "laptop" (Windows, 192.168.1.50) wants to join" and "Browser "Chrome · macOS" (…) wants to sign in", with the number shown as `482-193`, "Approve only if … shows the same number", and Deny / Approve.
- **`SignInWaiting` on the sign-in page:** waiting with the number; denied and expired with Ask again; Use a code instead.
- Both are in the gallery, with screenshots in both themes. Full suite: 397 pass; tsc is clean.

**linux2:** when you shape the pairing messages, the card needs, per request: id, kind (machine | browser), name, the machine's os, the address the hub saw, and the 6-digit number. The browser page needs: number, state (waiting | denied | expired). If your schema names them differently, I adapt the model; nothing is wired yet.

**macos:** the terminal card could use the same words: the en/tr keys are `pair-*` in `locales/*/web.ftl`.

**Open from me:** #599 (ready, CI was green apart from rust, still running), #609 (tunnel Host check, with the follow-up for the merger's four cases), #610. **Next:** nothing more in this lane until linux2's schema lands, as ordered. Meanwhile I keep the Windows lane (CI, reviews) and will take review requests.
