# Approvals queue

- Reachable from the top-bar indicator and as banners in the center panel.
- Each item: agent, project, node, requested action (command, file write, network, install), the exact
  command or diff, risk label from policy, time waiting.
- Actions: Allow, Deny, and **Allow always** when the agent offers it (below); Deny and stop agent is not built. Every
  decision is signed by the device key (ADR-0020).

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
  is interrupted or started again; a resumed agent starts without it (Agora replays nothing). The conversation's record of such a
  permission reads "Permission given; the agent remembers it".
- **Terminal client:** in the box that shows a request, `A` allows it always (`y` once, `n` denies); in a tab or chat, **Alt+A** does,
  only while the agent in front has a request that offers it (otherwise Alt+A is the agent's key). Never in the list, where `a` is Ask:
  Enter opens the agent's tab, or the commands (Alt+K) name it. The hints say Alt+A or `A` only when offered; `A` on a request that does
  not offer it says "This request can only be allowed once." and sends nothing. The agent's line says `[asks less]`.
- Bulk actions only for identical low-risk requests.
- Mobile layout first: approvals are the main mobile use case.
- Expired approvals show as expired; the agent is informed.
