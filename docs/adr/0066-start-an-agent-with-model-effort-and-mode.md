# 0066. Start an agent with its model, effort and mode, from the window and from another agent
Status: Proposed
Date: 2026-11-14

Not accepted. Nothing here may be built until the maintainer accepts it in a message of their own.
Origin: the maintainer: "an agent should even be able to come to Agora over MCP and say: in my folder open a Codex, pick this model and this effort, run this task, as a fait accompli." Amends ADR-0057 (the `agent_start` tool) and ADR-0059 (launch options).

## Context
`agent_start { kind, folder?, prompt?, node? }` (ADR-0057) starts any installed kind with the tool's own defaults, and the start window lets you pick machine, folder and kind (and launch options, ADR-0059). Neither can say **which model, how much reasoning effort, or which mode** the new agent should use,
although every real tool has them (`claude --model`, `codex -m` and `-c model_reasoning_effort=…`, opencode `--model`, ACP `session/set_option`). So "open a Codex with gpt-X at high effort and run this" needs a person in front of that tool's own menus.

## Decision (proposed)
1. **A manifest declares the settings a kind accepts** (additive, `min_engine` raised): `[[launch.setting]] id = "model" | "effort" | "mode" | <other>`, with `args` as a template (`["-m", "{value}"]`, `["-c", "model_reasoning_effort=\"{value}\""]`), the allowed values or a pattern, a default, and for structured (ACP) kinds a `set_option` mapping (the model and mode lists the agent reports, ADR-0058). Only settings checked on a real version are declared. Values are validated against the list or pattern: never put into a shell, never an argument that can smuggle another flag.
2. **`agent_start` gains `settings { model?, effort?, mode? }`** (and `options`, the named launch options of ADR-0059). The signed approval card (ADR-0057) shows **every setting and option** next to the kind, folder and prompt, and the signature covers them (a new signing domain version, length-prefixed, like ADR-0059); a setting the manifest does not declare is refused (`agent.setting_unknown`), a value outside its list too (`agent.setting_invalid`).
3. **The start window and the TUI** offer the same settings, from the manifest: the machine's kind list says what each accepts (model list when the tool reports one, effort levels, modes), remembers the last choice per kind per machine, and shows the badge of the ADR-0059 "does not ask" options as today.
4. **Nothing is wider than the asking agent's own limits** (ADR-0057 rules stay): depth, helper count, the folder ceiling, no inheritance of "don't ask"; a **model or effort the user did not allow** can be refused by the owner's node policy (`start.models = [...]` optional allow-list, so an agent cannot silently pick the most expensive model; the card also shows the model's price class when the catalog (ADR-0021) knows it).
5. **Cost is visible before and after:** the card shows the model's price when known; the usage panel (ADR-0042) already shows what the helper spent.

## Open questions for the maintainer
- Should an agent be allowed to choose **any** model the tool offers (with your approval on the card), or only models on your allow-list? (Proposed: any, shown on the card; the allow-list is optional.)
- A per-kind default model/effort in the owner's settings so cards stay short? (Proposed: yes, later.)

## Consequences
Manifest format (`[[launch.setting]]`), schema (`AgentStart`/`AgentLaunch` settings fields, new signing domain version, error codes), the node start path, the mesh tool schema and `mesh-tools.md`, clients (start window, TUI, approval card), shipped manifests with checked flags, tests with the fake agent, threat model (an agent choosing an expensive model or a risky mode: always on the card, optionally allow-listed).
