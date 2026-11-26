# The window on a phone

ADR-0067 rung 2 (builds on ADR-0046: the phone is the web client, installed as a PWA). The same window, reviewed for a phone: one
column, what needs you first, and controls a finger can press.

## When it applies

A window narrower than 700 px, or one shorter than 500 px on a touch screen (a phone held sideways), is a phone. A tablet (768 px
and wider) and every desktop window keep the full window ([main-window.md](main-window.md), [layout.md](layout.md)) unchanged; its
terminals get the key bar when the screen is a touch screen.

## The flow

1. **The list** (`#/`): the same stat tiles as Home, two to a row; **Needs you** first, then every machine (online first) with its
   agents, most urgent first and closed ones last. Agents are kit rows: the agent's avatar, its name and folder, its state as a pill.
   A request is the one request component ([approvals.md](approvals.md#one-request-three-sizes)) in its row size, stacked: the icon
   chip, the avatar and the ask as the title ("claude · teal wants to change a file"), the machine, the action as one chip and its whole
   detail (any hidden character written out, as on a start request), then **Deny** and **Allow** under it, each as wide as it can be.
   When the agent offers an answer it remembers (ADR-0101), **Allow always** sits between them, with the note and the agent's own words;
   a request that offers only that shows it in Allow's place. The arrow at the end of the title, or an agent row, opens the agent. An
   agent that waits with no request open is its row on the needs tint.
2. **An agent** (`#/agent/<id>`): **Back** (to the list), its name, state, machine and cost in the top bar; anything it asks, with
   Allow and Deny; then its terminal, or its conversation for a chat agent. **Next · N** in the top bar opens the next thing that
   needs you. Above the terminal, **N files changed** opens the review screen ([review.md](review.md)) and **Files** the file
   browser, each full-screen with Close; a closed agent's summary offers **Review the changes** and **Files** the same way.
   **What needs you stays in reach of the thumb**: a bar above the bottom bar (alone at the bottom of an agent) shows the first thing that
   needs you and how many there are. A plain yes or no that can be read whole there (the fleet console's rule, `decidableInline`: one
   short line each of action and detail, no hidden character, never a request to start an agent, never one that offers only an answer the
   agent remembers, ADR-0101) is drawn as the same request component in its row size (who, where, the ask as one chip) with **Deny**
   and **Allow** (the primary, last, as on every request) as 52 px buttons inside the safe area, and its card above does not repeat them. Everything else (a request to start an agent, long or
   structured text, an agent that waits, a machine asking to join, which keeps its two-press card, ADR-0065) has one **Review and
   decide** button that opens the agent or brings the full card into view; nothing is answered from the bar on text that was cut.
   The list's cards and the bar keep T44 as every other surface does, with the shared guard (`useAnswerGuard`, #1048,
   [approvals.md](approvals.md)): Allow and Deny do nothing for 600 ms after anything the card or the bar shows changed (a request that
   has just replaced another, a card that came in above, the trust notice, the link row), after the person came back to the window and
   after the link came back. A request is answered once for the whole window, also from one phone screen to the next: held while its
   answer goes, **Answer sent** once it went (Allow and Deny stay off while it is listed), answerable again after a fresh dwell when it
   failed. The bar also answers one request at a time: while its answer goes, the next request it shows waits. It is announced politely
   (a status, not an alert) and a focus that was on its button moves to the bar. It uses the fleet console's order (a request with no expiry never takes the bar from one about to lapse),
   and a tab, no-break or wide space makes a request not plain.
   A phone has no card over the window (that is the desktop's, [main-window.md](main-window.md#needs-you-loudly)); this bar is its way of keeping Needs you in reach.
   While the link is down Allow and Deny are disabled everywhere on the phone and say why. The state of the link has a row of its own
   under the title bar (it no longer pushes the buttons off a 390 px screen); that row is the one banner for the link, and the bar adds only the reason next to its disabled buttons.
3. The bar at the bottom of the list: **Search** (the command palette: agents, language, pages), **Start**, **Machines**, **This
   device** and **More**, a menu of the side panel's deep subjects (rows a finger can press): **Shared tasks and files** (the board,
   `#/board`), **Team chat** (`#/messages`) and **Usage** (`#/usage`), each a screen of its own with Back that draws what the side
   panel draws (visual pass 2026-11, finding 15). The other pages (machines, this device, activity) keep their own frame with Back. **Edit theme** (palette or This
   device) opens the theme editor ([themes.md](themes.md)) as a screen of its own (`#/theme`) with Back.

A notice's link to an approval (`#/approvals/<node>%2F<id>`, ADR-0046) opens the agent that asks once the window knows the request;
one already answered opens the list. A notice about an agent that waits or finished links to that agent (`#/agent/<node>%2F<id>`),
when the hub's `public-url` is set ([notifications.md](../design/notifications.md)).

## Approvals stay exactly as safe

Allow and Deny call the same signing as everywhere else: the browser's device key signs the answer and the machine checks it
(ADR-0020, ADR-0028, ADR-0061). A second tap sends nothing (the request is held until the first settles). When a machine that asks has
reported whom it trusts and this device is not among them, one notice at the top of the requests names every such machine before the
tap and links to **This device** (each card names its machine); the machine still decides, and nothing is skipped or softened on a
phone. A phone is trusted like any device: a card on another trusted device, or `agora node trust` on the machine.

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

- Every control in the phone window, the key bar and a page's title bar is at least 44 × 44 px (the widget kit's controls too, on
  any phone-sized window, touch or not); agent rows are 56 px tall; the bar's answers are 52 px.
- In an agent's chat on a touch screen, every control in an entry takes a 44 × 44 px finger too. Rows (a group of steps, a step, a
  thought, a change), a code block's header and the "Show all" buttons grow to 44 px. The small ones keep their drawn size and grow an invisible target instead:
  the file chips (`.tap-target`, with their rows set wider apart so two targets never overlap), "Show the request", and a link in a
  sentence (`.tap-inline`: padding above and below each piece, which never moves a line; where it meets the words of a link on the
  next line, the words win). A reply's Copy sits under the reply. A
  narrow window with a mouse keeps the dense sizes. `scripts/e2e-chat-touch.mjs` measures the targets in Chromium (#1073).
- Text uses the type scale only (the screen title is the 16 px title step); no phone file has a pixel font size.
- The page uses `viewport-fit=cover`; the bars keep out of the notch and the home indicator (`env(safe-area-inset-*)`), and pages
  keep out of a notch held sideways.
- With the touch keyboard open the window follows the visible part of the page (`visualViewport`; `interactive-widget=resizes-content`
  where the browser supports it), so the key bar sits above the keyboard.
- Held sideways the bars are slimmer and the bottom bar lays its labels beside the icons; the terminal takes the full width.
- A phone watching a terminal that a bigger window also shows makes it phone-sized for both while it is open (the smallest
  window wins, so the phone is never cut off); the bigger window gets its size back when the phone leaves the terminal. Opening the
  keyboard shrinks the phone's view, and with it the shared terminal, until it closes.

## Install

The hub serves `/manifest.webmanifest` (standalone, the mark as SVG plus 192 and 512 px PNGs and a maskable 512 px one); the page
links an `apple-touch-icon` for iOS. There is still no service worker (ADR-0046): the installed window needs the network, as
approvals do.

## Checked by

`scripts/e2e-phone.mjs` at 360 × 740, 390 × 844, 844 × 390 and 768 × 1024 on an emulated touch screen, and with the High
contrast and Large text themes at 360 and 390 px (44 px targets, the key bar fits); screenshots in both themes
in `docs/screenshots/phone/`; the needs-you bar in `docs/screenshots/phone-needs-bar/` (390 × 844, both themes; 360 px and Turkish too). `scripts/e2e-phone-changes.mjs` opens the review screen and the file browser at 390 × 844 and the
review screen again at 360 × 740 (fits, no sideways scroll). Unit tests: `apps/web/src/terminal/keys.test.ts`, `apps/web/src/phone/nav.test.ts`, `apps/web/src/phone/PhoneDock.test.tsx` (presses the real buttons: never Allow for a start request or a pairing request, disabled while the link is down, one answer at a time), `apps/web/src/phone/phoneGuard.test.tsx` (the cards' and the bar's dwell, once-only answer and window holds, across screens; the row density; the trust notice; no pixel font size in a phone file). Redesign screenshots of every phone screen at 390 × 844 with touch, both themes: `docs/ux/screens/phone-redesign-*.png`.

## Not in this rung

- No swipe gestures; Back is a button (and the browser's own back).
- A run on real phones (iOS Safari, Android Chrome), including installing to the home screen, is still to do.
