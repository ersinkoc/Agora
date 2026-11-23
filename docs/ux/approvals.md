# Approvals queue

- Reachable from the top-bar indicator and as banners in the center panel.
- Each item: agent, project, node, requested action (command, file write, network, install), the exact
  command or diff, risk label from policy, time waiting.
- Actions: Allow, **Allow always** when the agent offers it (below), Deny, Deny and stop agent. Every decision is signed by
  the device key (ADR-0020).

## One request, three sizes

Redesign PR 4: every request that waits on the person is one component (`ApprovalRequest`), with the same needs tint, an icon chip
for the kind of ask, the agent's avatar and the same order of buttons everywhere. Only its size changes:

| Size | Where | Shape |
|---|---|---|
| **row** | Home's queue; the phone's list (stacked: the buttons under the text, each as wide as it can be) | who, where and how long on one line, the ask under it, the buttons at the end |
| **card** | the side panel, above a terminal, above a chat's composer | the ask whole under the title, the buttons under it (at the end of the first line when the card is wide) |
| **full** | the attention dialog | larger; the title is the ask, the machine a tag; Later before the answers |

- **Buttons:** **Deny** (secondary), then **Allow always** where ADR-0101 offers it beside a single answer (secondary), then **Allow**
  (primary, last). A request that offers only "always" has it in the place of Allow.
- **Danger:** an Allow that lets an agent act without asking is the red danger button (ADR-0102): a request to start a helper with a
  permission-skipping option, and a switch to a mode or setting that skips asking. The start still asks for the confirmation first
  ("Keep asking" is the primary, focused answer); the switch keeps its dwell and "Read it first".
- **Title:** the ask in the window's words, "codex · teal wants to run a command" (change a file, read a file, open a web address, delete
  something, start another agent). It is chosen from a closed set of well-known action words, matched exactly; anything else reads
  "… asks permission". The words come from the agent, so the action and detail stay on the line under the title, whole, and that line is
  what is decided on.
- **Never decided on cut text:** where Allow is offered the ask is one mono chip that wraps, never truncated; the agent's whole request is
  one click away when the short form leaves something out. The row and the dialog offer Allow and Deny only for what `decidableInline`
  shows whole; anything else has **Review**.
- **Guards (T44):** every list where a request is answered in place (Home's queue, the side panel, the cards above a terminal and above
  a chat's composer, the phone's list and its bar for what needs you) has the same dwell, from one hook (`useAnswerGuard`, #1045): Allow and Deny do nothing for 600 ms after anything the
  list shows changed or moved (a pane narrow enough to put the buttons under the text included), after the panes were rearranged (a
  move, a split, a resize, a maximize, another tab in front, a pane pointed at or following another agent, a side column opened or
  dragged; a pane taking the focus is not a rearrangement), after the person came back and after the link came back; off while the link is down, with the reason. A start in a
  card goes through the same guard. Each surface's dwell is its own.
- **One answer per request, for the whole window (#1048):** the holds are the window's, not a surface's (`AnswerHoldsProvider` in
  `App`): a request answered, or on its way, from any surface (a card in a pane, the side panel, Home, the attention dialog, a switch's
  card, the phone's cards and bar) is held on every other one, also from one phone screen to the next. Held while it goes, "Answer sent" once it went, answerable again after a fresh dwell on every surface
  that lists it when it failed. Closing a surface (the side panel) drops nothing: an answer that went is forgotten only when its request
  leaves the window's list, one on its way only when it settles. The dialog keeps its own dwell, answered-once and Later; the switch
  card keeps its own Allow dwell and one-click lock, and its Deny has no dwell (main-window.md): it is off only while the surface is busy
  (`busy`: an answer on its way, the link down) or the request is held. The component only draws what the surface decided.
- **Narrow pane:** below 360 px a card in a pane is drawn in the row size with the text across it and the buttons under it, each as wide
  as it can be; nothing of the ask is cut.
- **One copy:** the side panel lists an agent by its request's card (which opens the agent), not again as a row above it; the corner
  banner is hidden while a Needs you list is on screen.
- **The phone (redesign PR 13, #1048):** no longer the exception. Its list draws each plain request in the row size, stacked, with the
  whole detail; its bar is a surface of its own under the same guard, and it still answers one request at a time (while its answer
  goes, the next request it shows waits). A machine that does not trust this device is named once, at the top of the list, not on
  every card.
- **Not yet:** the start and switch cards' bodies (they keep their layout; only their buttons follow the order and the danger rule).
- Screens: `docs/ux/screens/approval-row-*.png`, `approval-panel-*.png`, `approval-terminal-*.png`, `approval-card-narrow-*.png`,
  `approval-narrow-pane-*.png`, `approval-dialog-*.png`, `approval-skip-danger-*.png`, `approval-start-noask-danger-*.png` (dark and light);
  the phone's in `phone-redesign-*.png`.

## Allow always (ADR-0101)

A structured agent can offer an answer it remembers (ACP `allow_always`); the window shows it only when the request says so
(`scopes` holds ALWAYS, after the machine's policy) and never for a start request or a switch of mode.

- **Where:** the approval card (Inspector), the attention dialog, the banner over a pane and above a chat's composer, and the phone's
  card (between Deny and Allow, as everywhere). The fleet console's one-line rows and the phone's bar keep Allow and Deny; the full card has the third button.
- **Shape:** a secondary button between Deny and the primary Allow, which stays last, on every surface (the one order of
  [One request, three sizes](#one-request-three-sizes)), under the same rules as Allow on that surface (disabled while the link is down,
  during the dwell, which every surface has, while an answer goes). Under it one muted line in the
  window's words and, when the agent named its option, "In its own words: “…”" as plain text; that label is never the button's name.
- **Only "always":** a request that offers no single answer shows **Allow always** in the place of Allow (after Deny), as the primary button, with
  a line saying the agent offers no single answer. It is never answered from a one-line row (the fleet console, the phone's bar, the
  attention dialog): those send the person to the full card. Deny is a single answer on every card.
- **Words:** ACP lets the agent decide what the answer covers and how long it keeps it, so "for this session" and "until it ends" are
  said only for a kind known to drop the answer when its session ends (Codex's review of ADR-0101). No machine reports a kind as checked
  yet, so every kind reads **Allow always** with "remembers this answer … which may be longer than this session"; the ADR's session
  words stay in `locales/` under their own ids for the day a kind can be marked so.
- **Signature:** `agora-approval-always-v1`, binding the agent, the action and detail (hashed), the agent's label and the exact option
  id as announced (`net/approval.ts` `signAlways`, `crates/agora-tui/src/device.rs` `always_message`; vectors in
  `schema/fixtures/signing/approval-always-v1.json`). A machine that refuses it says why: single answers only on this machine
  (`approval.scope_forbidden`), the agent does not offer that answer (`approval.scope_unavailable`), or the agent was started by an older
  Agora (`approval.adapter_outdated`).
- **Asks less:** while an agent holds at least one always answer, its tile, pane header, phone row and header and the chat's controls
  show a quiet **Asks less** chip (neutral chrome, never a state colour). Its tooltip and accessible name say how many answers it
  remembers and list their actions (at most 16). An agent that acts freely shows **Acts freely** only. The chip goes when the agent ends,
  is interrupted or started again; a resumed agent starts without it (Agora replays nothing). The chip is Agora's record for that run
  of the agent: it lists only answers the agent was confirmed to have received; one whose delivery could not be confirmed shows as
  cancelled and the agent is ended. What the agent keeps in its own settings is up to the agent, as the note under the button says. The conversation's record of such a
  permission reads "Permission given; the agent remembers it".
- **Terminal client:** "always" is given only in the box that shows the whole request, the agent's own words and the duration
  warning, with `A` there (`y` once, `n` denies). In a tab or chat, **Alt+A** opens that box for the tab's own request, only while it
  offers "always" (otherwise Alt+A is the agent's key); so do the commands (Alt+K) and every other way in. Nothing is signed until a key
  in the box, and the hints and the command say so: "Alt+A allow always (asks first; …)", "Allow the request that waits, always… (asks
  first; …)". Never in the list, where `a` is Ask:
  Enter opens the agent's tab, or the commands (Alt+K) name it. The hints name Alt+A (in a tab or chat: it asks first) or `A` (in the box: it answers) only when offered; `A` on a request that does
  not offer it says "This request can only be allowed once." and sends nothing. Alt+Y, Alt+N and Alt+A in a tab answer exactly that tab's own request, whatever the tree has selected, and nothing if it is gone
  (never another request). Wherever the request is read before it is answered (the request box, the chat's waiting block, the inspector)
  it shows the agent's own name for its option ("In its own words: “…”", one plain line, at most 64 characters) and the warning that the
  agent decides what "always" covers and how long it keeps the answer, possibly longer than this session. The request box takes its
  answer as one deliberate key (a key typed before it appeared, or a paste, never answers). A single Allow or a Deny goes at once only when that
  exact request is in view: its own card whole in the inspector (beside a tab the inspector shows only the selected agent's own request,
  so a command from a tab never answers another agent's request unseen) or whole in its chat tab; otherwise the box opens. "Always" is
  never taken from a box last drawn cut: it says the window is too small and sends nothing. A key that comes with a change of the
  window's size is dropped until the box is drawn again. The agent's line says `[asks less]`.
- Bulk actions only for identical low-risk requests.
- Mobile layout first: approvals are the main mobile use case.
- Expired approvals show as expired; the agent is informed.
