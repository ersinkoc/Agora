# 0096. Agora offers a ready helper agent instead of a coding-agent loop of its own
Status: Accepted
Date: 2026-11-20
Accepted: 2026-11-20 (the maintainer's explicit message in the merger chat: "ADR-0095 ve ADR-0096'yı kabul ediyorum"). The open questions' proposed answers stand; the Review column's "accepted by" field (a schema step) stays deferred.

Builds on ADR-0004 (agents are data), ADR-0020 (approvals), ADR-0059 (skipping permissions), ADR-0087 (install agents from the window), ADR-0092
(the ACP registry is a source of kinds) and ADR-0095 (kanban and dispatch). Touches the threat model. Accepted: it may be built on.

Origin: the maintainer, 2026-11-20, asked whether Agora itself should get coding abilities (sign in with an API key or an OpenAI/ChatGPT login, plus a
few tools) or use a sub-agent for it.

## Context
- Kanban dispatch (ADR-0095) and small chores ("write this task up", "split this task") need an agent that is always one click away.
- A coding agent is a model client, a tool loop, a sign-in flow, a permission system and a long tail of provider changes. Others already ship that,
  and the ACP registry lists 47 of them (ADR-0092).
- Invariants: agents are data, not code; secrets never leave the node and the hub stores references only; the hub cannot widen node policy.

## Decision
1. **Agora does not write its own model or tool loop.** It has no API-key field, no provider login, no model client and no coding tools of its own.
2. **Agora offers a "helper" preset instead.** A preset is data: the name of an agent kind (an ACP agent from the ADR-0092 list, or a bundled manifest
   such as `wrongstack-acp` or `opencode-acp`), the folder rule, and the instruction text. Agora starts that agent like any other (the normal start,
   node policy, `signed_commands`), with the Agora mesh tools available (`task_*`, `send_message`, `workspace_status`). It appears in the window as a
   normal agent with the same approvals. It is the default target in the dispatch picker (ADR-0095) and can be started from the command palette.
3. **Sign-in stays inside the agent.** The agent's own auth (an API key, or an OpenAI/ChatGPT or other account login, Agent Auth or Terminal Auth,
   ADR-0092 decision 4) runs on the machine. Agora never reads, stores or forwards the credential, the hub never sees it, and an agent that needs
   a sign-in shows "needs sign-in on the machine".
4. **No skip-permissions default.** The preset starts in the agent's ordinary mode. Skipping permissions stays the owner's choice (ADR-0059) and is
   never part of a preset or of dispatch.
5. **Terms.** ADR-0057 already uses "helper" for an agent another agent started. The preset is therefore called a **ready agent** in docs and code and
   shown to people as **Helper** (UI vocabulary); an agent a parent started stays a "helper of" its parent in the agent tree. Decided here so the two
   do not collide (see open questions).
6. **Without any model, Agora still does read-only board chores itself:** sort, group, filter, count and search tasks, show who holds what, and
   highlight stale or blocked tasks. This is plain code over existing data, deterministic, with no prompt and no network. Anything that needs
   language understanding (summarize, split, rewrite a task) is done by the helper agent, not by Agora.
7. **When none is installed**, the Helper button opens the "Add an agent" list (ADR-0092 decision 3; installation by the ADR-0087 signed list and the
   owner's signed approval) with the suggested kinds first, and says in plain words that Helper needs one agent signed in on a machine. Nothing is
   installed or started by the button alone.

## Alternatives considered
- **Agora's own tool loop (API key or ChatGPT login, a few tools).** Rejected: a product of its own to maintain (providers, context, retries, tools),
  a new security surface (a key store on the node, tool execution outside every agent's approval flow), vendor lock to the providers we wire, and it
  duplicates what ACP agents already do and the user already has signed in.
- **Embedding an agent library.** Rejected: a new dependency with third-party code that runs with the user's files (maintainer notice and a
  dependency review needed, AGENTS.md), version churn, and the same sign-in and tool surface inside our binary.

## Consequences
- Schema: none expected. A preset is a small file under `manifests/agents/` or a settings entry naming a kind and an instruction; the node starts it
  with the normal start command. Docs: `docs/specs/agent-manifest.md` (preset), `docs/ux/` (Helper button), `docs/specs/mesh-tools.md` unchanged.
- No new secret store, no new dependency, no change to the protocol.
- Quality of the helper is the chosen agent's quality; Agora tests the preset with `agora-fake-agent`, never a real account.

## Threat-model entries to add (numbers assigned on acceptance)
- **A preset naming a hostile or wrong agent kind.** Presets ship in the repository and releases (reviewed); a person's own preset may name only a
  kind the node already knows (installed or on the signed list), never a command line. A registry change cannot change a preset.
- **The helper as a confused deputy.** It reads task text and repository content (T4, T5) and holds mesh tools. Mitigation: ADR-0095 decision 5, the
  same approvals as any agent, mesh tools under their ordinary limits and rate limits, no skip-permissions default.
- **Credential exposure.** Agora holds none by design; the preset instruction and logs carry none; tests with a fake sign-in that must not appear in
  any event, log or hub row.

## Open questions
- Which agent is the default preset? Proposed answer: the first installed kind in this order: WrongStack (structured), opencode, then Claude Code and
  Codex; the person can pick another, and the choice is remembered per machine. Never one that is not installed.
- What happens when none is installed? Proposed answer: decision 7, the "Add an agent" list with a suggestion, no automatic install.
- May the helper create tasks? Proposed answer: yes through `task_create` with the same limits as any agent (size, count, rate), and the tasks it
  creates are marked as made by that agent; it cannot dispatch (dispatch is a person's action in the first version of ADR-0095).
- Is "ready agent" the right internal name? Proposed answer: yes, to keep "helper" for ADR-0057's child agents; the UI shows "Helper" for the preset
  and "Started by <agent>" for a child.

## Build order
1. The preset format and one bundled preset; start it from the command palette; test with `agora-fake-agent`.
2. The Helper entry in the dispatch picker (after ADR-0095 step 2) and the empty state that opens "Add an agent".
3. Read-only board chores in the client (sort, group, filter), no agent needed.
4. Presets from a person's settings (kind names only).
