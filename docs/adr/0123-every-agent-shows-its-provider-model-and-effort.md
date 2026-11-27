# 0123. Every agent shows its provider, model and effort
Status: Accepted
Date: 2026-11-27

Accepted: 2026-11-27 (the maintainer's answer to the question listing the written decisions). The open questions' proposed answers stand.

Origin: the maintainer's orchestration request of 2026-11-27 ("every agent's provider, model, effort … visible"), roadmap M8.6.

## Context

- **What the window knows today.**
  - An adapter (ACP) agent reports its models and modes (`AgentSessionOptions` → `AgentSnapshot.current_model`, `models`) and its
    config options (`AgentSessionConfig`, effort among them as category THOUGHT_LEVEL, ADR-0090). The options reach the window as
    events only; a window that connects later has no effort until the agent reports again.
  - A terminal agent started with launch settings (ADR-0066: a model, an effort) records none of them: `AgentStarted` carries the
    launch options (ADR-0059) but not the settings, so the window cannot say "Opus, high effort" for a Claude Code started that way.
  - The provider is nowhere: the kind's id says "claude-code", not who serves its model. `[usage] model_map` (ADR-0021) maps reported
    model ids to `provider/model`, but only for usage.
- **Why it matters.** With many agents on many machines, the person picks who does what by what they run on; and plan limits
  (ADR-0091) are per provider.

## Decision

1. **The provider is manifest data.** A manifest's `[agent]` gains `provider` (a short id, `anthropic`, `openai`, `google`, …; at most
   32 characters, `[a-z0-9-]`), shown in the window by a Fluent key `provider-<id>`, falling back to the id. A kind that can use many
   providers (opencode) leaves it empty, and the model's `provider/model` id (decision 3) names it when the agent reports one.
   `AgentKinds` already carries the manifest's kind entry; it gains `provider` (schema).
2. **Launch settings are recorded.** `AgentStarted` gains `launch_settings` (repeated `{id, value}`, the ADR-0066 settings the agent
   was started with, as the node applied them, at most 8, each value at most 64 bytes), and `AgentSnapshot` mirrors it. A resume
   re-states them (the ledger keeps them, as it keeps launch options).
3. **The model and the effort, one way for every kind.** `AgentSnapshot` gains:
   - `model`: the model the agent runs on, from the first that is known: the adapter's current model, the launch setting `model`, or
     the model the agent last reported with its usage (mapped through `model_map` to `provider/model` when the manifest maps it);
   - `effort`: the adapter's THOUGHT_LEVEL option's current value, else the launch setting `effort`; empty when neither is known;
   - `config`: the last `AgentSessionConfig` options with their current values (so a late window has them).
   The hub fills them in its projection from events it already receives (plus decision 2's field), so no new event is needed.
4. **Shown everywhere an agent is.** The agent's row and card show a provider mark and "Opus · high" in the UI's words; the details
   panel lists provider, model, effort and the launch settings; the fleet view and the board's agent picker can filter by provider and
   model. The terminal client shows the same in its agent line (M8.9).
5. **Nothing is guessed from the screen.** A terminal agent whose model was not set at launch and that reports no usage shows
   "model not known" rather than a model read from its status line (ADR-0117's screen regions may add that later, under its own rule).

## Consequences

- The person sees at a glance which agents run which model with which effort, on every machine, including terminal agents started
  with settings.
- Three schema additions (manifest kind `provider`, `AgentStarted.launch_settings`, `AgentSnapshot.model/effort/config/launch_settings`)
  and one manifest key; append-only.
- The provider is only as right as the manifest (and the model map); a hand-made manifest without it shows no provider.

## Alternatives considered

- **Read the model from the terminal's status line**: works for some tools, but breaks with their redesigns and is screen
  detection's business under ADR-0117's rules. Not here.
- **Ask each agent for its model over the mesh**: the agent may not know or may say anything; mesh content is untrusted.

## Open questions (with proposed answers)

1. Should `config` carry every option or only the known categories (model, mode, effort)? Proposed: every option, at most 16, as the
   chat already shows them.
2. Provider ids: free text or a fixed list? Proposed: free text in the manifest's pattern, with Fluent names for the common ones.

## Build order (once accepted)

1. Schema (the four additions), manifests' `provider` for the bundled kinds, golden fixtures.
2. Node: `launch_settings` on `AgentStarted` and in the ledger; hub projection fills `model`, `effort`, `config`.
3. Window: the agent row, card, details and filters; screenshots in both themes.

## Status of the build

1. Schema: done (2026-11-27). `AgentKind.provider` (8), `AgentStarted.launch_settings` (35), `AgentSnapshot.model` (49), `effort`
   (50), `config` (51), `launch_settings` (52); `LaunchSetting` moved to `common.proto` (same name and wire); round-trip and
   old-fixture test `crates/agora-protocol/tests/agent_model_effort.rs`; `protocol.md` "Provider, model and effort". Nothing fills
   the fields yet.
2. Node and hub: done. The manifest's top-level `provider` (validated; `claude-code` anthropic, `codex` openai, `antigravity-acp`
   google), `KindInfo.provider` → `AgentKind.provider`; `LaunchChoice.settings` (what `choose` applied) → `AgentStarted.launch_settings`,
   kept in the ledger and restored at a take-up (`set_kept_launch`); the hub's projection keeps them, and `snapshot_of` fills
   `model` (adapter current model, launch setting, usage model), `effort` (THOUGHT_LEVEL current, launch setting) and `config`
   (at most 16). The manifest key is top-level `provider`, not `[agent] provider` (manifests have no `[agent]` table).
3. Window: done. `net/view.ts` `modelAndEffort` works out the same model and effort from what the window
   was told (a start's launch settings, its usage, its adapter's options and config, or the snapshot), and `providerOf` the provider:
   the kind's `AgentKind.provider`, which the window asks each online machine for once per connection and keeps in this browser
   (`net/agentKinds.ts`), else the first part of a `provider/model` id. The chip (`components/ModelChip.tsx`, "opus · high") is on the
   pane header, the fleet tile and, as words, the agent list's row and the phone row; its name says the provider ("Provider:
   Anthropic", `provider-<id>` in the locale files, the id otherwise), model and effort. The provider mark is that name in words: the
   kind's own logo already stands beside every agent, and no provider logos are bundled. The fleet view narrows to one provider or one
   model (`modelPicks`, `matchesModelPick`), and so does the board's agent picker (Give to an agent), whose rows say the model too.
   The terminal client's agent line is M8.9's.
