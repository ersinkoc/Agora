# Approvals queue

- Reachable from the top-bar indicator and as banners in the center panel.
- Each item: agent, project, node, requested action (command, file write, network, install), the exact
  command or diff, risk label from policy, time waiting.
- Actions: Allow, **Allow always** when the agent offers it (below), Deny, Deny and stop agent. Every decision is signed by
  the device key (ADR-0020).

## Allow always (ADR-0101)

A structured agent can offer an answer it remembers (ACP `allow_always`); the window shows it only when the request says so
(`scopes` holds ALWAYS, after the machine's policy) and never for a start request or a switch of mode.

- **Where:** the approval card (Inspector), the attention dialog, the banner over a pane and above a chat's composer, and the phone's
  card. The fleet console's one-line rows and the phone's bar keep Allow and Deny; the full card has the third button.
- **Shape:** a secondary button after Allow and Deny (in the attention dialog, between Deny and the primary Allow, which stays last), under
  the same rules as Allow (disabled while the link is down, during the dwell, while an answer goes). Under it one muted line in the
  window's words and, when the agent named its option, "In its own words: “…”" as plain text; that label is never the button's name.
- **Only "always":** a request that offers no single answer shows **Allow always** in the place of Allow, as the primary button, with
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
