# The window on a phone

ADR-0067 rung 2 (builds on ADR-0046: the phone is the web client, installed as a PWA). The same window, reviewed for a phone: one
column, what needs you first, and controls a finger can press.

## When it applies

A window narrower than 700 px, or one shorter than 500 px on a touch screen (a phone held sideways), is a phone. A tablet (768 px
and wider) and every desktop window keep the full window ([main-window.md](main-window.md), [layout.md](layout.md)) unchanged; its
terminals get the key bar when the screen is a touch screen.

## The flow

1. **The list** (`#/`): **Needs you** first, then every machine (online first) with its agents, most urgent first and closed ones
   last. A request shows who asks, on which machine, what it asks and its whole detail (any hidden character written out, as on a start request), with **Allow** and **Deny** as two large
   buttons: one tap answers it. Tapping the request's title, or an agent, opens it.
2. **An agent** (`#/agent/<id>`): **Back** (to the list), its name, state, machine and cost in the top bar; anything it asks, with
   Allow and Deny; then its terminal, or its conversation for a chat agent. **Next · N** in the top bar opens the next thing that
   needs you.
3. The bar at the bottom of the list: **Search** (the command palette: agents, language, pages), **Start**, **Machines**, **This
   device**. The other pages (machines, this device, activity) keep their own frame with Back.

A notice's link to an approval (`#/approvals/<node>%2F<id>`, ADR-0046) opens the agent that asks once the window knows the request;
one already answered opens the list. A notice about an agent that waits or finished links to that agent (`#/agent/<node>%2F<id>`),
when the hub's `public-url` is set ([notifications.md](../design/notifications.md)).

## Approvals stay exactly as safe

Allow and Deny call the same signing as everywhere else: the browser's device key signs the answer and the machine checks it
(ADR-0020, ADR-0028, ADR-0061). A second tap waits for the first. When a machine has reported whom it trusts and this device is not
among them, the card says so before the tap and links to **This device**; the machine still decides, and nothing is skipped or
softened on a phone. A phone is trusted like any device: a card on another trusted device, or `agora node trust` on the machine.

## The terminal key bar

On a touch screen (`pointer: coarse`, a phone or a tablet) every terminal has one row of keys under it: **Esc**, **Tab**, **Ctrl**,
the four arrows and **Paste**.

| Key | Sends |
| --- | --- |
| Esc | `ESC` |
| Tab | `TAB` |
| Arrows | `ESC [ A/B/C/D`, or `ESC O A/B/C/D` when the program asked for application cursor keys (vim, less) |
| Ctrl, then an arrow | `ESC [ 1 ; 5 A/B/C/D` |
| Ctrl, then a typed character | its control code (`c` is `0x03`, `[` is `ESC`, space is `NUL`, `?` is `DEL`); one without a control form is sent as typed |
| Paste | the clipboard as the program expects a paste (bracketed when it asked for that) |

Ctrl applies to the next key only and shows that it is on. Pressing a key never takes the focus from the terminal, so the touch
keyboard stays open. Paste reads the clipboard only when pressed (the browser may ask); where it cannot, the button is not shown and
the keyboard's own paste works in the terminal. The mapping lives in `apps/web/src/terminal/keys.ts`.

## Size, safe area, landscape

- Every control in the phone window, the key bar and a page's title bar is at least 44 × 44 px; agent rows are 56 px tall.
- The page uses `viewport-fit=cover`; the bars keep out of the notch and the home indicator (`env(safe-area-inset-*)`), and pages
  keep out of a notch held sideways.
- With the touch keyboard open the window follows the visible part of the page (`visualViewport`; `interactive-widget=resizes-content`
  where the browser supports it), so the key bar sits above the keyboard.
- Held sideways the bars are slimmer and the bottom bar lays its labels beside the icons; the terminal takes the full width.

## Install

The hub serves `/manifest.webmanifest` (standalone, the mark as SVG plus 192 and 512 px PNGs and a maskable 512 px one); the page
links an `apple-touch-icon` for iOS. There is still no service worker (ADR-0046): the installed window needs the network, as
approvals do.

## Checked by

`scripts/e2e-phone.mjs` at 360 × 740, 390 × 844, 844 × 390 and 768 × 1024 on an emulated touch screen; screenshots in both themes
in `docs/screenshots/phone/`. Unit tests: `apps/web/src/terminal/keys.test.ts`, `apps/web/src/phone/nav.test.ts`.

## Not in this rung

- No swipe gestures; Back is a button (and the browser's own back).
- A run on real phones (iOS Safari, Android Chrome), including installing to the home screen, is still to do.
