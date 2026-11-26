# 0066. Start an agent with its model, effort and mode, from the window and from another agent
Status: Accepted
Date: 2026-11-14
Accepted: 2026-11-14 (maintainer's explicit answer to a question listing each Proposed ADR with its summary and "approve as proposed": they ticked 0064, 0065, 0066, 0069, 0071, 0072 and 0074). The open questions' proposed answers stand.

Origin: the maintainer: "an agent should even be able to come to Agora over MCP and say: in my folder open a Codex, pick this model and this effort, run this task, as a fait accompli." Amends ADR-0057 (the `agent_start` tool) and ADR-0059 (launch options).

## Context
`agent_start { kind, folder?, prompt?, node? }` (ADR-0057) starts any installed kind with the tool's own defaults, and the start window lets you pick machine, folder and kind (and launch options, ADR-0059). Neither can say **which model, how much reasoning effort, or which mode** the new agent should use,
although every real tool has them (`claude --model`, `codex -m` and `-c model_reasoning_effort=…`, opencode `--model`, ACP `session/set_option`). So "open a Codex with gpt-X at high effort and run this" needs a person in front of that tool's own menus.

## Decision
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

## Status of the build
- **PR 1 (#605, built):** `[[launch.setting]]` in manifests (engine 3), `AgentStart`/`AgentLaunch.settings`, `AgentKind.settings`, the node's checks
  (`agent.setting_unknown`, `agent.setting_invalid`), `agora agent start --setting`, Claude Code and Codex `model` and `effort`.
- **PR 2 (built):** the signing. `AgentLaunch` with settings is authorized over `directory-action-v4` and an `agent.start` approval with
  settings over `agora-approval-start-v2`, both length-prefixed and in the order sent or shown (decision 2); the node verifies them and applies exactly
  the signed settings; the capability `agent.launch_settings` makes an older hub or node pair refuse rather than drop them; the mesh tool
  `agent_start` takes `settings`; the web approval card lists them in the window's words (decision 2); a request with settings offers no
  grant. Specs: `docs/specs/protocol.md`; threat model T28.
- **PR 3 (built): the clients.** The web start window offers each setting the machine lists for the chosen kind (`AgentKind.settings`): a
  list is a choice among exactly its values, a setting without a list (a model name) is a text box limited to the plain token the node
  accepts; both start on "the agent's own default" (nothing is sent, so that request is byte for byte the one without settings), a value
  the machine did not list is not sendable, and the chosen values are shown before Start and signed over `directory-action-v4`. The TUI
  approval card shows the settings in words (before the prompt, in the lines that must be read before y counts) and signs
  `agora-approval-start-v2` for a request that carries them (`decision_message`: an `agent.start` is only ever answered over v1 without
  settings or v2 with them); the `approval.needs_newer_window` refusal is gone.
- **Remembered choices (web window):** the start window remembers the last model and effort per kind per machine on this device (localStorage; checked against what the machine lists now, so a value it no longer takes is dropped and never sent) and shows what an agent was started with, read-only, in its header. The agent never reports an effort (the adapter protocol carries modes and models only), so there is no effort menu.
- **`options` on `agent_start` (built, ADR-0059 decision 5):** an agent may name launch options (`skip-permissions`); the card lists them
  with the confirmation, and the decision is signed over `agora-approval-start-v3` (settings, then option ids, before the hash). No grant.
- **The TUI's start picker, the client part:** `crates/agora-tui/src/launch.rs` asks the kind's settings one at a time in the manifest's
  order (a numbered list whose first line keeps the tool's default, a typed value for a setting without values, Enter keeps the default,
  Backspace goes back, Esc cancels) and gives the chosen pairs in that order for `Device::authorize_launch_with_settings`;
  `Session::kind_settings` reads them from the machine's kind list. The console's start (`start_in_folder` in `crates/agora/src/tuicmd.rs`)
  does not ask it yet and still sends no settings, and the TUI does not remember the last choice per kind per machine.
- **Not built yet:** asking the settings in the TUI's own start flow (above), the owner's `start.models` allow-list and the model's price class on the card (decisions 4 and 5: the ADR
  does not specify the policy key's place or the catalog lookup closely enough for a small change), `mode` as a shipped setting, and
  keeping the settings across a resume after a node restart.
