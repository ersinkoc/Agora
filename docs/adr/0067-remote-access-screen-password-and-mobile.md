# 0067. Remote access: a screen password, Tailscale and Cloudflare Tunnel, and a phone-sized window
Status: Proposed
Date: 2026-11-14

Not accepted. Nothing here may be built until the maintainer accepts it in a message of their own.
Origin: the maintainer: "our next thing is a screen password for the web UI, remote access through a Cloudflare Tunnel and/or Tailscale, and the mobile interface."
Builds on ADR-0046 (mobile is the web client as a PWA), ADR-0051 (ntfy is the push channel), ADR-0049 (team mode), ADR-0020/0061 (device-signed actions), ADR-0065 (pairing).

## Context
The window is reached today by a LAN address with a self-signed certificate (a browser warning), signed in with a code, and nothing is meant to be reachable from outside. Reaching your own hub from a phone or another city needs three things:
a way to get there safely (a network path with a real certificate), a lock on the door (so that an address someone learns opens nothing), and a window that works on a phone. Three properties already protect the *dangerous* part: an agent start,
an approval or a grant is signed by a device key and checked on the machine, so a stolen session token alone cannot start anything (ADR-0020). What a remote visitor with only a session could do is **watch**: terminals, costs, diffs. That is the data this ADR protects.

## Decision (proposed)
Built in five small rungs; each ships alone.
1. **Screen password (rung 1).** The owner sets a password (`agora password set`, or in the window under This device). A new sign-in from a non-loopback address needs it (the owner on the hub's own machine is never asked: ADR-0062). Stored as a salted, slow hash (PBKDF2-HMAC-SHA256 through `ring`, 600 000 iterations, no new dependency); a wrong password is rate limited with a growing lockout per address and globally, and is audited. A passkey (already supported) can replace it per device. **Lock now** in the window ends the session; an optional **idle lock** (off by default; e.g. 10 minutes) locks the window until the password is typed again, which does not touch agents. Sessions stay 30 days, extended on use (#450); a locked or signed-out session needs the password.
2. **A phone-sized window (rung 2).** The same web client (ADR-0046), reviewed on phone widths: a single column (machines and agents -> an agent -> its terminal or conversation), "Needs you" first with Approve/Deny reachable in one tap, big tap targets, a terminal key bar (Esc, Tab, Ctrl, arrows, paste) for a touch keyboard, install as a PWA, ntfy pushes that open the right agent, and safe-area/landscape handling. Signing actions with the device key works on phones (WebCrypto non-extractable keys); a phone is trusted like any device (ADR-0061, a card on another device or at the machine).
3. **Tailscale (rung 3, first remote path: private by default).** `agora remote tailscale` detects Tailscale (`tailscale status --json`, no API keys), adds the machine's tailnet name and IP to the hub's certificate names, can publish the hub with `tailscale serve` (a real HTTPS certificate on the machine's MagicDNS name: no browser warning), and prints the URL for the phone. Only devices of the person's tailnet reach it; the screen password still applies to non-loopback sign-ins. Nodes on other machines can use the same names to reach the hub across networks.
4. **Cloudflare Tunnel (rung 4, public hostname: guarded).** `agora remote cloudflare` helps set up `cloudflared` (the user installs it and logs in; Agora does not hold Cloudflare credentials): it writes/prints the tunnel config that points a hostname at the hub on loopback, and can run `cloudflared` as a supervised child. The hub treats the tunnel as a trusted reverse proxy only for the real client address header, accepts only the configured hostname (Host/Origin checks), refuses sign-in without the screen password and **warns that Cloudflare Access (SSO) in front of the hostname is strongly recommended** (a one-line setup note); public exposure is **off until the owner turns it on** and the window shows a persistent "Reachable from the internet at …" banner while it is on.
5. **Hardening and visibility (rung 5).** Remote sign-ins are audited with address, device and result; "Where am I signed in" lists sessions with a revoke button; per-address and global rate limits on the sign-in and websocket upgrade; a remote session cannot register a new device key or change the password without the password again; **no remote command ever bypasses a machine's own trust**: a remote session can watch, and can start/approve only with a device the machine trusts (ADR-0061), exactly like a local one.
6. **Nothing is reachable from outside by default:** loopback or LAN only, until `agora remote ...` is run on purpose; turning a path off removes it (certificate names, `tailscale serve`, the tunnel).

## Open questions for the maintainer
- Which first: Tailscale (private, simpler, proposed) or Cloudflare Tunnel (public hostname)?
- Password: one for the owner (proposed) or per device? Idle lock default off (proposed) or on?
- Is a screen password *in addition to* passkeys acceptable, or should remote sign-in require a passkey where one is registered? (Proposed: either; both is optional.)
- Do you already use a Cloudflare account/domain and Tailscale? (It decides which helper to build and test first.)

## Consequences
Hub: password store and verification, rate limits, remote-address handling and Host/Origin checks, audit rows, session list; CLI: `agora password`, `agora remote tailscale|cloudflare|off|status`; web: password screen, lock, session list, phone layout, terminal key bar; docs ("Open Agora from your phone in five minutes"), threat model (a remote visitor with only a session can watch but not act; password guessing and lockouts; tunnel misconfiguration; the Cloudflare Access recommendation).
