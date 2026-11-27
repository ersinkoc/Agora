# Screenshots

The component gallery (`#/gallery`) in both themes, on a desktop and a phone viewport. UI pull requests
refresh these with `scripts/screenshots.mjs` (see its header for how to run it).

| | Dark | Light |
|---|---|---|
| Desktop | ![](gallery-desktop-dark.png) | ![](gallery-desktop-light.png) |
| Phone | ![](gallery-phone-dark.png) | ![](gallery-phone-light.png) |

Each image shows a dark panel and a light panel; both themes appear whatever the system theme is.

The sign-in page (unauthenticated routes redirect to it):

![](signin-desktop-dark.png) ![](signin-desktop-light.png)

From another machine, once the owner set a screen password (ADR-0067), it asks for the password too; the title bar has **Lock**
and This device sets the password and the idle lock (`screen-password/`):

![](screen-password/signin-password-dark.png) ![](screen-password/signin-password-light.png)
![](screen-password/window-lock-button-dark.png) ![](screen-password/device-password-dark.png)

## A chat agent's conversation
An agent without a terminal (ACP or an adapter): what it did reads as steps (folded details, the last three of a run shown), its
changes as coloured diffs, its plan as a list (earlier copies folded) and its answers with code blocks (`chat/`, gallery data).

![](chat/chat-top-dark.png) ![](chat/chat-dark.png)
![](chat/chat-top-light.png) ![](chat/chat-light.png)

## The console (Home)
One window: machines and agents on the left, the terminals of the agents being watched in the middle (several at once), and
what needs you, uncommitted changes, agent messages and spend on the right.

![](console-dark.png)
![](console-light.png)
![](console-phone-dark.png)

Command palette (Ctrl+K), on Radix Dialog:

![](palette-dark.png)

What an agent changed (click "N files changed"):

![](diff-dark.png)

Files of a workspace (the Files button on a terminal):

![](files-dark.png)

A chat agent (no terminal): send it a request; its permission request appears on the right and is answered with this browser's device key:

![](approval-dark.png)

Commit history of a workspace (the History button on a terminal):

![](history-light.png)

A chat agent's conversation (ADR-0038): your requests and its answers, streamed from the node:

![](chat-light.png)

`demo-ltr-dark.png` and `demo-rtl-dark.png` show the same console left to right and with the page forced right to left (no RTL translation ships yet, so the text is English): the layout mirrors, the terminals stay left to right.

## On a phone (ADR-0067 rung 2)
`phone/`, from `scripts/e2e-phone.mjs` with `AGORA_SCREENSHOTS`: the list with a request waiting and one agent with the key bar, at
360 × 740, 390 × 844 and held sideways (844 × 390); a tablet (768 × 1024) keeping the full window with the key bar; This device on a
phone. Both themes, and the High contrast and Large text themes at 360 and 390 px (`*-high-contrast.png`, `*-large-text.png`,
plus the theme editor as a phone screen).

![](phone/phone-390x844-home-dark.png) ![](phone/phone-390x844-agent-dark.png) ![](phone/phone-390x844-home-light.png) ![](phone/phone-390x844-agent-light.png)
![](phone/phone-844x390-home-light.png) ![](phone/phone-844x390-agent-dark.png)
![](phone/phone-360x740-home-large-text.png) ![](phone/phone-360x740-agent-large-text.png) ![](phone/phone-390x844-home-high-contrast.png) ![](phone/phone-390x844-theme-editor-large-text.png)

## Visual regression baselines
`apps/web/visual/` holds the reference images that `scripts/visual-regression.mjs` compares against (gallery, sign-in and the
Settings page on demo data at `#/gallery/settings`, both themes, desktop and phone; the theme packs' three dialogs, `pack-*`, on a desktop). They are the contract for how the shared components look: a pull request that changes them updates the
images with `--update`, and the diff shows in review.

`grant-dark.png` and `grant-light.png` show the permission window a machine's `signed_commands` policy triggers (ADR-0041).
- `adr59-start-dont-ask-confirm-{dark,light}.png`, `adr59-start-dont-ask-chosen-{dark,light}.png`: ADR-0059 start window, "Don't ask" asked about (Keep asking is the main button) and chosen; `adr59-no-ask-badge-tiles-{dark,light}.png`: the "Does not ask" badge on an agent's tile (gallery).

Themes (ADR-0068 rung 2, `docs/ux/themes.md`), from `scripts/screenshots-themes.mjs`: the theme editor in both schemes, High
contrast, Large text, Slate, Sand, and a change refused because text would be hard to read.

![](adr68-theme-editor-dark.png) ![](adr68-theme-editor-light.png)
![](adr68-theme-contrast-warning-dark.png) ![](adr68-theme-contrast-warning-light.png)

One shell watched from a 1600 × 900 window and a 900 × 600 one (`shared-size/`, `scripts/e2e-shared-size.mjs`): both show the
smaller window's size; the bigger window has it at its top left with a hairline where it ends.

![](shared-size/shared-size-big-dark.png) ![](shared-size/shared-size-big-light.png)
![](shared-size/shared-size-small-dark.png) ![](shared-size/shared-size-small-light.png)

## ADR-0066: a start request with a model and an effort

The approval card an agent's `agent_start` with settings produces in the web window: the settings sit next to the folder, the
first prompt is shown in full, and only "this one" can be allowed (`apps/web/src/components/StartApproval.tsx`).

![](adr66-start-card-settings-dark.png) ![](adr66-start-card-settings-light.png)

The start window's pickers (the agent's own default is preselected and sends nothing; a model is a text box, an effort a list the machine gave)
and the same window with a model and an effort chosen, which then says what will start (`apps/web/src/components/LaunchSettings.tsx`).

![](adr66-start-pickers-dark.png) ![](adr66-start-pickers-light.png)
![](adr66-start-confirmation-dark.png) ![](adr66-start-confirmation-light.png)

The eight interface quick wins, before and after, dark and light, on a desktop, a tablet with touch and a phone (`ux-quick-wins/<screen>-<theme>-{before,after}.png`): text size, review colours and first file, a failed agent, the layout menu, touch targets, approval wording.

Home as a fleet console, before and after, in both themes (`fleet-console/`; see `../ux/fleet-console.md`): `before-*` is the window before, `after-*` after, for what needs you (`attention`), a calm fleet (`calm`), a fresh hub (`empty-hub`), machines but no agent (`first-run`), the hub unreachable (`offline`) and the phone (`phone`, `phone-calm`); Large text, High contrast, Turkish and 200 agents are after-only.

The owner's cards for a machine that asks to join (ADR-0065), dark and light, from a real hub and a real `agora join` (`pairing-cards/`): the empty hub with "Allow a new machine to join" (`01`, `02`), the row in what needs you with the card (`03`), the second step (`04`), Replace (`05`), the fleet console with an agent fleet (`06`), the phone (`07`, `08`), the link down (`09`), and the states side by side (`10`, `11`, Turkish `12`).

## The terminal behaves like a real terminal

From `scripts/terminal-fidelity/` (`docs/ux/terminal-fidelity.md`): a realistic screen with a selection (`terminal-*`), find
(`find-*`), a program's clipboard offer and "Jump to latest" while scrolled up (`offer-and-jump-*`), the "Terminal settings" menu
(`settings-menu-*`), a phone with the key bar after a swipe (`phone-*`), a terminal smaller than its window (`letterbox-*`), and a
block of full-block characters drawn by the page, at row height 1.1 and 1.0, and by the graphics card (`blocks-*`, dark only).

![](terminal-fidelity/terminal-dark.png) ![](terminal-fidelity/terminal-light.png)
![](terminal-fidelity/find-dark.png) ![](terminal-fidelity/find-light.png)
![](terminal-fidelity/offer-and-jump-dark.png) ![](terminal-fidelity/offer-and-jump-light.png)
![](terminal-fidelity/settings-menu-dark.png) ![](terminal-fidelity/settings-menu-light.png)
![](terminal-fidelity/phone-dark.png) ![](terminal-fidelity/phone-light.png)
![](terminal-fidelity/letterbox-dark.png) ![](terminal-fidelity/letterbox-light.png)
![](terminal-fidelity/blocks-page-1.1.png) ![](terminal-fidelity/blocks-page-1.png) ![](terminal-fidelity/blocks-gpu-1.png)

## Needs you, loudly
The card for a plain request (Allow, Deny, Later), the card for one that is only reviewed, the banner after Later, and the settings in This device
(`needs-you-modal/`, `scripts/screenshots-needs-you-modal.mjs`):

![](needs-you-modal/card-answer-dark.png) ![](needs-you-modal/card-answer-light.png)
![](needs-you-modal/card-review-dark.png) ![](needs-you-modal/card-review-light.png)
![](needs-you-modal/banner-dark.png) ![](needs-you-modal/banner-light.png)
![](needs-you-modal/this-device-dark.png) ![](needs-you-modal/this-device-light.png)

## Home, terminal and command palette

Home with the Needs you panel kept beside the console list, "Jump to latest" with a link address under it (the link address is added to the page for the shot, as the demo terminal has no history or link), and the new command palette entries. The demo has no hub, so it shows no Lock, no join entry and no project entries.

![](home-terminal-palette/home-needs-you-panel-dark.png) ![](home-terminal-palette/home-needs-you-panel-light.png)
![](home-terminal-palette/terminal-jump-and-link-dark.png) ![](home-terminal-palette/terminal-jump-and-link-light.png)
![](home-terminal-palette/palette-machines-dark.png) ![](home-terminal-palette/palette-machines-light.png)

## ADR-0059 decision 5: a helper that acts without asking

The approval card an agent's `agent_start` with `options: ["skip-permissions"]` produces: how the helper acts and what that means, no
"more like this", and Allow first shows the same confirmation as a start from the window, with Keep asking as the primary button
(`apps/web/src/components/StartApproval.tsx`).

![](helper-no-ask/helper-no-ask-card-dark.png) ![](helper-no-ask/helper-no-ask-card-light.png)

![](helper-no-ask/helper-no-ask-confirm-dark.png) ![](helper-no-ask/helper-no-ask-confirm-light.png)

A chat agent after Stop turn with two messages waiting (#908): the step that ran reads "Stopped" (a still square, no spinner), each
waiting message is above its own answer, and the input no longer says it waits:

![](0908-stop-turn-queue-dark.png) ![](0908-stop-turn-queue-light.png)

## The visual pass (M1.4)
Every main screen in both themes at a phone and a desktop width (`visual-pass-2026-11/`, `scripts/screenshots-visual-pass.mjs`), listed
with the findings in [the review](../reviews/2026-11-26-visual-pass.md).

The repository README shows `home-desktop-dark.png` from this pass; a later pass that replaces it updates the README's link in the same commit.
