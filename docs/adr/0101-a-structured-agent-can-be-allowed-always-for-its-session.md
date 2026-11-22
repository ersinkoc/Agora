# 0101. A structured agent can be allowed always for its session
Status: Accepted
Date: 2026-11-22
Accepted: 2026-11-22 (the maintainer, in the merger chat: "bence bu önemli bir özellik acilen yapmalıyız", then "Accept, start now" to the question that summarised the scope button, the separate ALWAYS signature and the policy refusal; Codex's review findings are fixed in the implementation PRs)

Origin: the maintainer's decision of 2026-11-22: structured (ACP) agents must be able to get an "always allow" answer from the person,
so they stop asking for every action. Today WrongStack's `wstack acp serve` (no launch flag that skips asking, `wrongstack-acp.toml`)
asks on every tool call, and the person can only answer each one.

Touches ADR-0006 (append-only schema: appended fields, one enum, one event), ADR-0020 and ADR-0028 (user-signed approvals verified on the
node: a new signing domain), ADR-0041 (grants are not used here, see the alternatives), ADR-0058, ADR-0059 and ADR-0090 Amendment A
(modes, the `skip_permissions` policy, the "Acts freely" badge, session-only scope), ADR-0060 and ADR-0093 (resume and kept agents),
ADR-0024 and `docs/ux/visual-language.md` (UI words, one primary action), `docs/ux/approvals.md` (which already lists "Allow for this
session"), `docs/specs/adapter-protocol.md` (`approval/request`, `approval/resolve`), `docs/specs/protocol.md`, `docs/specs/error-codes.md`,
`docs/specs/agent-manifest.md` and the threat model (T4, T51, a new row).

## Context

An ACP agent asks for permission with `session/request_permission`, offering options of four kinds: `allow_once`, `allow_always`,
`reject_once`, `reject_always` (in ACP, `allow_always` allows the operation and asks the agent to remember the choice; the protocol says nothing
about what the choice covers or for how long, so the agent decides both). The bridge (`crates/agora-adapters/src/acp.rs`, `approval/resolve`, about line 1690) turns the request into an
`approval/request` and, when the node passes the person's decision, picks the agent's option:

- approved: `allow_once`, else `allow_always`, else the first option, whatever its kind;
- denied: `reject_once`, else `reject_always`, else `cancelled`.

The decision itself carries one bit. `ApprovalResolve { approval_id, approved, signer_device_id, signature }` (`commands.proto`) is signed
over `agora-approval-v1 NUL node_id NUL approval_id NUL verdict` (ADR-0028, `agora_node::approvals::signed_message`), the adapter gets
`ApprovalResolveParams { session_id, approval_id, approved }` (`adapter.proto`), and the window sees `ApprovalRequested` / `ApprovalSnapshot`
with `action` and `detail` only. The options the agent offered never leave the bridge.

Three things follow:

1. **The person cannot answer "always".** An agent that offers `allow_always` is never sent it while it also offers `allow_once`, so it asks
   again for every call. For kinds without a launch flag or a bypass mode (WrongStack), the only alternatives are answering every card or
   switching the whole agent to a mode that skips asking (ADR-0090 Amendment A), which is far wider than "stop asking me about `git status`".
2. **A plain Allow can already become "always".** When an agent offers no `allow_once`, the bridge sends `allow_always` for a decision the
   person signed as a plain Allow, and the agent remembers it. Nothing records that, and the signature does not cover it. Today's fallback
   to "the first option" can even select an option of an unknown kind.
3. **The window knows nothing of the options**, so it cannot offer a choice the agent does not have.

Terminal (L1) agents ask on their screen; their prompts are answered by keys the agent's own UI defines, not through approvals. They are
not covered here.

## Decision

### 1. The choice the person sees

When, and only when, the request offers it (decision 2), the approval card shows a third button after **Allow** and **Deny**:
**"Allow always for this session"**. Allow stays the primary button (one primary action, `visual-language.md`); the new one is a secondary
button and obeys the same read-first rule as Allow (a card whose full request must be read first keeps both disabled until it is). Under it,
one muted line in the window's own words, and the agent's own name for its option as quoted, untrusted text:

| Key (`locales/en/web.ftl`, `locales/tr/web.ftl`) | en | tr |
| --- | --- | --- |
| `approval-allow-always` | Allow always for this session | Bu oturum boyunca hep izin ver |
| `approval-allow-always-note` | { $name } will not ask again for what it counts as the same request, until it ends. It decides what that covers. | { $name } aynı istek saydığı şeyler için, sona erene kadar bir daha sormaz. Neyin kapsandığına kendisi karar verir. |
| `approval-allow-always-label` | In its own words: { $label } | Kendi sözleriyle: { $label } |
| `approval-always-only` | This agent offers no single answer for this request: allowing it allows it for the rest of its session. | Bu ajan bu istek için tek seferlik izin sunmuyor: izin verirsen oturumunun geri kalanı için izin vermiş olursun. |
| `agent-always-allowed-short` | Asks less | Daha az sorar |
| `agent-always-allowed-explain` | You allowed { $count } of its requests for this session. It does not ask again for those until it ends. | Bu oturum için { $count } isteğine izin verdin. Sona erene kadar bunlar için bir daha sormaz. |
| `permission-approved-always` | Permission given for this session | Bu oturum için izin verildi |

Error and notice codes (`locales/<lang>/errors.ftl`, `docs/specs/error-codes.md`, decisions 2 and 3):

| Code | en | tr |
| --- | --- | --- |
| `approval.scope_forbidden { approval_id }` | This machine does not let agents act without asking, so only this one request can be allowed. | Bu makine ajanların sormadan iş yapmasına izin vermiyor; yalnızca bu istek için izin verilebilir. |
| `approval.scope_unavailable { approval_id }` | The agent does not offer that answer for this request. Nothing was sent; answer it again. | Ajan bu istek için bu yanıtı sunmuyor. Hiçbir şey gönderilmedi; yeniden yanıtla. |
| `approval.always_only_forbidden { action }` (conversation notice) | The agent asked for “{ $action }” and offered only an answer it would remember. This machine allows single answers only, so it was refused. | Ajan “{ $action }” için izin istedi ve yalnızca hatırlayacağı bir yanıt sundu. Bu makine yalnızca tek seferlik yanıtlara izin veriyor, bu yüzden reddedildi. |
| `adapter.permission_unanswerable { action }` (conversation notice) | The agent asked for “{ $action }” without offering a way to allow it, so it was refused. | Ajan “{ $action }” için izin istedi ama izin vermenin bir yolunu sunmadı, bu yüzden reddedildi. |

- **Where it shows.** Every web surface that shows the request itself: the card (`ApprovalCard`, Inspector), the attention layer
  (`AttentionLayer`), the banner in the centre panel (`ApprovalBanner`) and the phone's card (`Phone.tsx`). The fleet console's one-line rows
  (`FleetConsole`) keep Allow and Deny; opening the agent reaches the card. Start requests (`StartApproval`) and switches to a mode that skips
  asking (`SkipApproval`) never show it (decision 2).
- **A request that offers only "always"** (the agent has no single answer, decision 2) shows **Allow always for this session** in the place of
  Allow, as the primary button, with `approval-always-only` above it, and Deny. The person is never shown "Allow" for something the agent
  will remember.
- **The terminal client.** `A` allows always in the request view (the full request on screen, where `y` and `n` answer today), and
  `Alt+A` does so in a tab or a chat, only while the agent in front has a request that offers it (otherwise `Alt+A` stays the agent's key,
  as `Alt+Y` and `Alt+N` do). Not in the list: there `a` is Ask and `Alt+Y` is already case-folded (`Alt+Shift+Y` allows once), so a
  one-letter case difference must never be the gap between a single and a remembered answer; from the list, Enter opens the request. The
  build checks that neither key folds into an existing one. The hints grow the choice only when the request offers it
  (`tui-request-answer-always` = "y allows it · A allows it always for this session · n denies it · any other key leaves it waiting",
  `tui-chat-answer-always`, `tui-tab-answer-always` with `Alt+A`, `tui-allowed-always` = "Allowed for this session.", and the palette's
  `tui-app-cmd-allow-always` = "Allow the request that waits, always for this session"; tr alongside). `A` on a request that does not offer it
  says `tui-allow-always-not-offered` = "This request can only be allowed once." and sends nothing.
- **The indicator.** While an agent holds at least one always answer (decision 4), its tile, header and session bar show a quiet
  **"Asks less"** chip with `agent-always-allowed-explain` on hover or focus, listing the requests (their `action`, bounded), in neutral
  chrome colours, not a state colour. It is not the "Acts freely" badge (ADR-0059, ADR-0090 Amendment A): one remembered answer is narrower than a mode that skips every question, and
  the two must not read alike. An agent that both acts freely and holds always answers shows "Acts freely" only. The conversation's
  permission entry for such a decision reads `permission-approved-always`.
- `docs/ux/approvals.md`'s "Allow for this session" becomes "Allow always for this session", shown only when the agent offers it.

### 2. The protocol: an appended scope, and the offer travels with the request

One new enum and appended fields only (ADR-0006); no published field changes meaning.

```proto
// ADR-0101: how far a person's answer to an approval reaches.
enum ApprovalScope {
  APPROVAL_SCOPE_UNSPECIFIED = 0;  // read as ONCE everywhere: what every decision meant before ADR-0101
  APPROVAL_SCOPE_ONCE = 1;
  // The agent remembers the answer, as the agent defines it (ACP `allow_always`), for the rest of its session.
  APPROVAL_SCOPE_ALWAYS = 2;
}
```

| Message | Appended field | Who sets it |
| --- | --- | --- |
| `ApprovalRequestParams` (adapter → node) | `repeated ApprovalScope scopes`; `string always_label` | the adapter or bridge: which answers the agent offers, and the agent's own name for its always option |
| `ApprovalRequested`, `ApprovalSnapshot` | `repeated ApprovalScope scopes`; `string always_label` | the node, after its policy (decision 3); the hub copies them into the snapshot |
| `ApprovalResolve` (window → node) | `ApprovalScope scope` | the window, from the button pressed; covered by the signature (decision 3) |
| `ApprovalResolveParams` (node → adapter) | `ApprovalScope scope` | the node, after verifying |
| `ApprovalResolved` | `ApprovalScope scope` | the node: what was signed and delivered (the audit record) |
| `ChatEntry` | `bool permission_always` | the node, on the permission entry of an approved ALWAYS decision |
| `AgentSnapshot` | `repeated string always_allowed` | the hub, from `AgentAlwaysAllowed` (decision 4) |

and one new event in `Event.body` (next free number, producer `node`):

```proto
// ADR-0101: an agent was sent an "always" answer and holds it for its session (as the agent defines it).
message AgentAlwaysAllowed {
  string agent_id = 1;
  string approval_id = 2;
  string action = 3;         // the request's action, as the node announced it (cleaned, bounded)
  string signer_device_id = 4;
  uint64 at_ms = 5;
}
```

An empty `scopes` means `[ONCE]`: every adapter and node before this ADR. `always_label` is the agent's text: the node removes control and
format characters and cuts it to 120 characters, as for every agent-supplied string on a card (T51), and it is empty unless `scopes` holds
ALWAYS.

**The four combinations of a decision:**

| `approved` | `scope` | Meaning |
| --- | --- | --- |
| true | ONCE (or unset) | today's Allow: the agent's single answer |
| false | ONCE (or unset) | today's Deny |
| true | ALWAYS | Allow always for this session |
| false | ALWAYS | refused like a grant on a request that cannot carry one (`approval.signature_invalid`, decision 3); nothing is sent to the agent. See "never" below |

`scope` must be unset or ONCE for every node-made approval (`agent.start`, `agent.skip_permissions`, any `agent.` action): they have their own
signed grants (ADR-0057, ADR-0090 Amendment A), and an ALWAYS there is refused the same way. Such requests never carry ALWAYS in `scopes`.

**The bridge's mapping** (`acp.rs` and `docs/specs/adapter-protocol.md`), replacing today's fallback:

- Offered: ONCE when the agent offers `allow_once`; ALWAYS when it offers `allow_always`. `always_label` is the `allow_always` option's
  `name`. An option of any other kind is never selected for an approval: ACP defines exactly the four kinds, and the bridge cannot tell
  whether an unknown one allows, refuses or does something else (today's "first option" fallback is removed).
- A request that offers neither `allow_once` nor `allow_always` cannot be allowed at all. The bridge raises no approval for it: it answers
  the agent `reject_once` if offered, else `cancelled`, and reports `adapter.permission_unanswerable` (params `action`) as a `log` line and a
  conversation notice (ADR-0058), so the person sees that the agent asked for something Agora could not offer to allow. So every
  `approval/request` from a current bridge has `scopes` of `[ONCE]`, `[ALWAYS]` or `[ONCE, ALWAYS]`.
- approved + ONCE → `allow_once`. **Never `allow_always`.** With none, the bridge refuses the decision (JSON-RPC `-32602`, `data`
  `{code: "approval.scope_unavailable"}`) and sends the agent nothing.
- approved + ALWAYS → `allow_always`, never anything narrower or wider. With none, refused the same way.
- denied → `reject_once`, else `cancelled`. Never `reject_always`: a remembered refusal is reserved (below), and the automatic refusals
  (an always-only request under a refusing policy, a request with nothing to allow) follow the same rule (refined in the Codex review).

A refused decision is handled as a refused one today (the node marks the entry cancelled and resolves the approval unsigned); in practice it
cannot happen, because the node checks the offer first (decision 3). An L3 adapter may set `scopes` too (declaring `approval_scopes`, section 7) and then receives `scope`; one that does
not is never sent ALWAYS.

This closes the silent widening of Context item 2: an agent that offers only `allow_always` announces `scopes = [ALWAYS]`, so the window
offers only the always button (decision 1), and an older window's plain Allow (ONCE) is refused by the node with `approval.scope_unavailable`
before anything reaches the agent. That card stays waiting and can be answered from a current window. This is a deliberate, small regression
for such agents in old windows, in exchange for a signed answer that always means what the button said.

**An always-only request the node may not offer** (`[ALWAYS]` from the adapter, but the policy refuses ALWAYS for this agent or its manifest
switches it off, decision 3) would be a card nobody can allow. The node raises no card for it: it answers the adapter denied at once
(the agent gets `reject_once` or `cancelled`), records `ApprovalRequested` and `ApprovalResolved { approved: false }` with no signer as for a
withdrawn request, and adds a conversation notice `approval.always_only_forbidden` (params `action`), whose text says this machine only
allows single answers and this agent offered none for that request. The person learns why the agent was refused; nothing was allowed.

**"Never" (`reject_always`) is not offered in this ADR.** Denying once already stops the call. A remembered refusal would make the agent fail
later calls silently, with nothing in Agora to explain or undo it, and a fourth button costs simplicity (ADR-0024). The table above reserves
the combination: a later ADR can allow `approved = false, scope = ALWAYS` without a schema change.

**Capability.** The node advertises `approval.always` in its handshake (passed by the hub), like every feature, for `agora doctor` and the
hub. The window does not branch on it (it cannot read node capabilities, `NodeSnapshot` carries none): the evidence it uses is ALWAYS in a
request's `scopes`, which only a node that implements this ADR, under a policy that allows it, ever sets. No capability is needed to keep
ALWAYS from older nodes: a window sends it only in answer to such an offer, and if it ever reached an older node, that node would ignore the
unknown field, check the signature against the v1 bytes, and refuse it (decision 3).

**Compatibility, case by case:**

- **Old window, new node.** It never reads `scopes` and always sends ONCE over the v1 bytes: Allow and Deny work as today, except on an
  always-only request, where Allow is refused with `approval.scope_unavailable` (the regression named above) and Deny works.
- **New window, old node.** No request carries `scopes`, so no always button appears; nothing changes.
- **Old hub between new ends.** It decodes and re-encodes, dropping unknown fields: requests reach the window without `scopes`, so no always
  button appears and every answer is ONCE. On an always-only request that means Allow is refused (`approval.scope_unavailable`, the card
  stays) and only Deny works until the hub is updated. If an ALWAYS ever crossed an old hub, the dropped field would make the node check the
  v1 bytes and refuse it. Hub and node usually ship together in one binary.
- **Old bridge kept across a node update** (ADR-0075 in-place update, ADR-0093 holder): until that agent restarts it sends no `scopes` (read
  as `[ONCE]`) and keeps today's fallback, so a plain Allow can still reach `allow_always` for an agent that offers no `allow_once`. This is
  unrecorded, as today. It ends when the agent restarts with the new bridge. Named as a residual; not worth a version check on the link.
- **L3 adapters** that do not set `scopes` are offered ONCE only and never receive `scope`.

### 3. Security: still a user-signed approval; the node decides

**The signature covers the scope.** An ONCE (or unset) decision is signed over the `agora-approval-v1` bytes exactly as today, so every
existing window, terminal client and node is unchanged. An ALWAYS decision is signed over a new domain, length-prefixed like
`agora-approval-skip-v1`:

```
agora-approval-always-v1 NUL F(node_id) F(approval_id) F(verdict) F(agent_id) F(sha256(action)) F(sha256(detail)) F(always_label)
```

where `F(x)` is the decimal byte length of `x`, `:`, then `x` (canonical decimal, raw bytes), `verdict` is `1`, `agent_id` is the request's
`ApprovalRequested.agent_id`, the hashes are SHA-256 of `action` and `detail` exactly as announced, in lowercase hex, and `always_label` is
the cleaned text announced. Vectors go in `schema/fixtures/signing/approval-always-v1.json`, checked by `agora-protocol` and the node.

- **The node chooses the bytes from the scope it received.** ALWAYS → the always domain; anything else → v1. A hub that turns ONCE into
  ALWAYS makes the node check a v1 signature against the always bytes; one that strips ALWAYS to ONCE makes it check an always signature
  against the v1 bytes. Both fail with `approval.signature_invalid`; nothing is sent to the agent, the card stays waiting until it expires,
  and the window shows its usual "the answer did not go through" (`approval-failed`). The person can answer again (from a window that
  reaches the node through an honest hub).
- **It binds what the person read.** v1 binds only node, approval and verdict, so a hub that relays the request could show the person other
  words than the node holds, and a single answer signed that way still verifies (accepted today for single answers). A remembered answer
  multiplies the harm, so the always domain binds the agent, the action, the detail and the agent's own label as announced: a relaying hub
  that rewrites the card's text cannot get an ALWAYS signature that verifies. This does not help against a hub that serves the signing page
  itself (T55: such a page can sign whatever bytes it likes with a browser key); keys outside the hub's page (the terminal client) are
  unaffected, as for every approval.
- **The node checks, in this order.** First the shape, which reveals nothing about the request: an ALWAYS on a node-made request
  (`agent.*`) or with `approved = false` is refused as a helper grant on a request that cannot carry one is today
  (`approval.signature_invalid`). Then the signature against `trusted-devices`, before anything else is said about the request, as today
  (`approvals.rs`, `resolve`: "verify before anything changes"), so a peer without a trusted key learns nothing. Then the request's stored
  `scopes` (what the adapter offered, never anything in the decision) must hold the decided scope, else `approval.scope_unavailable`; then the
  policy (below), else `approval.scope_forbidden`; then expiry and single use as for every approval (ADR-0020, ADR-0028). A refusal before
  expiry spends nothing: the card stays waiting, so the person can still allow it once or deny it (`resolve` removes and spends a request
  only after it verified and passed every check; today a bad signature already leaves it pending).
- **Node policy can forbid it, by the existing `skip_permissions` line** (ADR-0059: `forbid | allow | confined_only`, default `allow`, set
  only on the machine). `forbid` refuses every ALWAYS; `confined_only` refuses it for an agent that is not confined (ADR-0047/0050); `allow`
  permits it. Under a policy that refuses, the node also leaves ALWAYS out of the `scopes` it announces (so the window shows no button that
  would fail) and still refuses an ALWAYS that arrives (the policy is read again at decision time, it may have changed). An always-only
  request under such a policy gets no card at all (decision 2). Why the same line:
  an always answer lets the agent act without asking for whatever it counts as the same request; that is a narrower case of what
  `skip_permissions` already governs, and an owner who wrote `forbid` must not find agents on that machine acting unasked. A second policy
  line would be a setting no default fails to cover (`visual-language.md`, "every setting must justify itself"). It is not widened:
  `forbid` can only remove the offer, never add one, and the hub cannot change the file.
- **Mesh messages and agent text can never choose it** (T4). Only a window's signed `ApprovalResolve` carries a scope; the adapter can say
  what the agent offers, which only decides whether the button appears. An agent that offers `allow_always` is configuring its own
  memory: a hostile agent does not need Agora's permission to act at all, so the offer grants it nothing it did not have. The agent's
  `action`, `detail` and `always_label` are untrusted text, shown bounded and cleaned, and the note says the agent decides what is covered.
- **What "always" covers is the agent's definition.** Agora neither knows nor enforces the rule the agent keeps (a tool, a command prefix, a
  folder). It records that an always answer was given (`ApprovalResolved.scope`, `AgentAlwaysAllowed`, the "Asks less" chip) and expects it
  to end with the agent's session. **Residual (threat-model row, below):** an agent that keeps the answer longer than its session (in a
  settings file of its own) does more than the button said. A manifest can switch the offer off for such a kind:
  `[structured] allow_always = false` (`docs/specs/agent-manifest.md`, engine bump), which makes the node announce no ALWAYS for it; a kind
  is marked so when a check on a real version finds it persists the answer.
- **A new threat-model row** (T60, written in the schema step): a hub turning a single answer into a remembered one, a card whose words differ
  from what is remembered, an agent offering "always" to widen itself, a stale always answer after the session; with the mitigations above and
  the residuals (the agent decides the extent and lifetime; a device that can sign for the owner can allow always, the same trust as any
  approval; T55 applies to the signing key as for every approval).

### 4. Resume, restart and the end of a session

The agent holds the answer; Agora only records it. The record follows the agent's **process**, because that is where the answer lives:

- **The agent ends, is stopped, or is started again** (a new session): the node's records for it are dropped, the hub clears
  `AgentSnapshot.always_allowed` on `AgentExited` and on the next `AgentStarted` of that agent, and the chip goes.
- **The node restarts and the agent was kept** (ADR-0093, the holder kept the same process): the agent still remembers. The node keeps its
  always records beside the session it already keeps for a kept agent (`kept-<id>.pb`, the #932 path: `action`, approval id, signer, time;
  never the agent's rule), and restates `AgentAlwaysAllowed` after the `AgentStarted` it restates, so the chip stays true.
- **The agent is resumed** (ADR-0060: a new process continues the conversation with `session/load` or a resume flag): the records are dropped
  and **Agora does not replay** any answer. Whether the agent still remembers is its own behaviour; an agent that forgot asks again and the
  person answers again. An agent that kept the answer through its own resume acts on it without the chip: the residual above, and the same
  manifest switch.
- **Agora never sends an always answer by itself**: a request rebuilt from replayed lines after a node restart (ADR-0093 decision 3) is a new
  request the person answers; nothing the node holds answers it.

### 5. Alternatives considered

- **Client-side auto-approve rules in Agora** (the window or the node answers matching requests by itself, "always allow `run_command git *`").
  Rejected: the window would have to sign without the person, or the node would answer approvals with no signature, which is exactly what
  ADR-0020 forbids. Matching on `action` and `detail` means trusting agent-written text, so a rule written for `git status` matches whatever the
  agent chooses to call `git status` (T4). It would also be Agora's own permission language on top of every agent's, kept in sync with none. The
  agent's own `allow_always` is the agent's rule over the agent's tools, which it understands and Agora does not.
- **A blanket device grant** (ADR-0041 `signed_commands`, extended to "approve this agent's requests for an hour"). Rejected for the same
  reason: a grant authorises commands the window sends; here someone would still have to answer the agent, and it would be the node, unsigned.
- **Mapping it to an ACP mode** (switch the agent to a mode that accepts edits or skips asking). Rejected as the answer to this request: it
  exists already (ADR-0090 Amendment A) and is much wider, every tool for the rest of the session, while the maintainer's ask is to stop
  answering the same question. It stays the choice for an owner who wants that.
- **Doing nothing.** Rejected: structured agents without a launch flag stay unusable for long work, and the silent widening of Context item 2
  stays unsigned and unrecorded.
- **A new byte in the v1 message instead of a new domain.** Rejected: it would change the bytes every existing window signs (an old window's
  ONCE would fail on a new node).
- **A remembered refusal ("Deny always")**: deferred, see decision 2.

### 6. Build order, each its own PR, and tests

1. **Schema** (needs this ADR accepted, the schema step with it): `ApprovalScope`, the appended fields, `AgentAlwaysAllowed`, golden fixtures for
   each changed message (new fixtures only, none edited), the signing vectors `approval-always-v1.json` with an `agora-protocol` test, error
   codes `approval.scope_forbidden`, `approval.scope_unavailable`, `approval.always_only_forbidden` and `adapter.permission_unanswerable`
   (en, tr, `error-codes.md`), `protocol.md` and `adapter-protocol.md`,
   threat-model row T60, `agent-manifest.md` for `allow_always`. `schema-breaking` stays green.
2. **Bridge and node** (`agora-adapters`, `agora-node`): the bridge's offer and mapping (never `allow_always` for ONCE), the node's stored
   offer, policy filter on announce and on decide, the always domain in `approvals.rs`, `ApprovalResolved.scope`, `AgentAlwaysAllowed`, the
   kept-agent restate, the manifest switch, capability `approval.always`. Tests: the fake ACP agent (`agora-fake-agent --acp`) offering each
   combination of the four kinds (ONCE never selects `allow_always`; ALWAYS selects only `allow_always`; an only-always request refuses ONCE;
   a request with neither allow kind raises no card and gets the notice; an only-always request under `forbid` gets no card, is denied and
   gets the notice; an option of an unknown kind is never selected);
   a hub-flipped scope in both directions fails the signature; ALWAYS for an `agent.start` and an `agent.skip_permissions` request is refused;
   `approved = false` with ALWAYS is refused; each policy value (announce and decide, and a policy changed between the two); an ALWAYS for a
   request that did not offer it; the kept agent restates its records across a node restart; a resumed agent does not; a mesh message cannot
   produce a decision (T4 test extended).
3. **Hub**: copies the fields into `ApprovalSnapshot`, derives `AgentSnapshot.always_allowed` from `AgentAlwaysAllowed` (cleared on
   `AgentExited` and `AgentStarted`, at most 16 kept, newest kept). Test with recorded events.
4. **Web** (`apps/web`): `signAlways` over the always domain (`net/approval.ts`, vectors reproduced in a test), the third button on the four
   surfaces of decision 1, the always-only shape, the chip, the conversation entry, the error texts, en and tr; screenshots in both themes
   (ADR-0024). Tests: the button appears only when offered; it signs ALWAYS and the right bytes; a card that does not offer it never sends it.
5. **Terminal client** (`agora-tui`: `client.rs`, `device.rs`): `A` in the request view and `Alt+A` in a tab or chat, the hints, the always signer, the chip in the agent
   line; tests on the key paths and the bytes.
6. **Docs**: `docs/ux/approvals.md`, `docs/specs/adapter-protocol.md` (the mapping, replacing "`allow_once`, else `allow_always`"),
   `wrongstack-acp.toml` and the other structured manifests unchanged unless a real check finds a kind that persists the answer.

The native desktop client (ADR-0099), when it signs approvals, uses the same domain; no other signer exists today (`apps/web/src/net/approval.ts`,
`crates/agora-tui/src/client.rs`).

### 7. Refinements from the implementation and Codex's review (PR A, 2026-11-22)

Codex reviewed this ADR at 4c81e007; the implementation applies its findings, which refine the text above without changing its decisions:

1. **The offer describes allow options only.** `scopes` is checked for an Allow; a signed single Deny stays valid on an always-only card.
   Every denial, including the node's automatic one, reaches the agent as `reject_once` or `cancelled`, never `reject_always`.
2. **An outdated bridge cannot widen an Allow unseen under a strict policy.** The bridge declares the adapter capability `approval_scopes`;
   the node reads `scopes` only from an adapter that declares it, and sends `scope` only to such an adapter. For an ACP agent whose bridge
   does not declare it (one kept across a node update), the node refuses a single Allow with the new code `approval.adapter_outdated`
   while the policy refuses ALWAYS (`forbid`, or `confined_only` unconfined): that bridge may answer it with the agent's remembered
   option. Under `allow` the residual named in the compatibility list stays.
3. **Unknown scopes are refused.** Only UNSPECIFIED and ONCE use `agora-approval-v1`, ALWAYS the always domain; any other value is
   `protocol.invalid_message` (`field: scope`) on the node and in the bridge. `AgentAlwaysAllowed` and `ApprovalResolved.scope = ALWAYS`
   are written only after the adapter took the answer; a refused or undelivered one is recorded cancelled, as before.
4. **The decision is bound to one option.** ACP options have unique `optionId`s but may share a name and kind. The request carries the
   exact id of the always option (`always_option_id`, appended to `ApprovalRequestParams`, `ApprovalRequested` and `ApprovalSnapshot`),
   the always domain ends with `F(always_option_id)`, and an offer with more than one `allow_always` is ambiguous: ALWAYS is not offered
   for it. The bridge answers with the option it captured for that request. An option id that cannot be signed as it is (empty, over
   128 bytes, a control or format character) is never offered.
5. **Error parameters.** `approval.scope_forbidden`, `approval.scope_unavailable` and `approval.adapter_outdated` carry no parameter (their
   text does not show the random approval id; the window knows which card it answered).
6. **Event number.** `AgentAlwaysAllowed` is `Event.body` 69 (after 66 to 68, the agent and task events added since ADR-0090).
7. **Words for an unchecked kind (PR B, Codex's review).** ACP leaves what an always answer covers and how long it lasts to the agent, so
   "for this session" and "until it ends" are said only for a kind known to drop the answer when its session ends. Nothing marks a kind
   so yet (no manifest key or protocol field reaches the clients), so the window and the terminal client show the words that say the agent
   remembers it, possibly beyond its session: the `-kept` ids beside the ids of decision 1 (for example `approval-allow-always-kept` =
   "Allow always", `tui-allowed-always-kept`), chosen by one function per client (`alwaysText` in `apps/web/src/net/always.ts`,
   `always_text` in `crates/agora-tui/src/client.rs`). The ids of decision 1 keep the session words for the day a kind can be marked as
   checked, which needs its own decision (a manifest key and a way to tell the clients). The same applies to the chip and the
   conversation entry. An always-only request is never answered from a one-line row (the fleet console, the phone's bar, the attention
   dialog); its full card shows the always button in the place of Allow.

## Consequences

- A structured agent that offers `allow_always` can be told once, and stops asking for what it counts as the same request, for the rest of
  its session; the window shows that it does.
- A plain Allow never again becomes a remembered answer behind the person's back; an agent that only offers "always" says so on the card.
  Older windows can only deny such a request; a current window can allow it.
- One enum, appended fields on eight messages, one event, two error codes, two notice codes and one signing domain; nothing published changes meaning.
- Agora still never answers an agent by itself, and the hub still cannot widen what a person signed.
- Known residual: the extent and lifetime of an always answer are the agent's; Agora records and shows, it does not enforce.

## Open questions

1. Should "Asks less" also show on the agent's tile in the fleet console, or only in the agent's own views? Proposed: everywhere the agent's
   state shows, it is small.
2. Should the policy have its own line (`always_allow = forbid | allow | confined_only`) after all, for an owner who forbids skipping modes but
   wants remembered answers? Proposed: no, until someone asks; it can be added later as a narrower override.
3. Should the default for a kind nobody has checked be to offer ALWAYS (as proposed), or to require the manifest to opt in? Proposed: offer it;
   the agent chose to offer `allow_always`, and the maintainer's case (WrongStack) is such a kind.
4. "Deny always": add later if people ask for it (decision 2 keeps the combination reserved).
