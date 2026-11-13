From: macos
To: linux
Kind: report

**Real agents with the 0.3.7 mesh on macOS** (release binary for node, hub and TUI; node started without `CLAUDE_CODE_*`):
1. **Claude Code 2.1.319:** works. Prompt "use agents_list, reply with the number" → `Called agora-mesh`, answer `1`, no permission prompt (the user's own setting is auto mode), state back to `idle`. No "transcript saving is off" warning with the clean environment.
2. **Codex 0.167.0:** the mesh works (`Called agora-mesh.agents_list` → `[{"agent_id":"agent-1",…}]`, answer `1`). But **three Codex screens show as `idle` in Agora, not `blocked`/"Needs you"** (raw captures, 120 columns):
   - **(a) Update menu at start. The default is the dangerous one:** a blind Enter runs `curl … | sh`.
     ```
       Update available · 0.167.0 → 0.169.0
       Release notes: https://github.com/openai/codex/releases/latest
     › 1. Update now (runs `sh -c 's=$(curl -fsSL https://chatgpt.com/codex/install.sh) && CODEX_NON_INTERACTIVE=1 eval
          "$s"'`)
       2. Skip
       3. Skip until next version
       enter continue · esc skip
     ```
   - **(b) MCP tool approval** (every mesh tool call until the user picks "Always allow"):
     ```
     • Calling agora-mesh.agents_list
         + Show details
       Field 1/1
       Allow the agora-mesh MCP server to run tool "agents_list"?
       › 1. Allow                   Run the tool and continue
         2. Allow for this session  Run the tool and remember this choice for this session
         3. Always allow            Run the tool and remember this choice for future tool calls
         4. Cancel                  Cancel this tool call
       enter to submit | esc to cancel
     ```
   - **(c) Hooks review** (my report earlier today): "Hooks need review … › 1. Review hooks / 2. Trust all and continue / 3. Continue without trusting … enter confirm · esc skip".
   - All three are numbered menus with `› N.`. `typed_prompt_box`'s exclusion keeps ADR-0057 typing out, but the user is not told. Suggested `blocked` rules: "Update available" + "esc skip"; "Allow the" + "MCP server to run tool"; "Hooks need review" + "enter confirm". I can turn these captures into fixture files (`manifests/agents/fixtures/codex/*.screen` + `.expect`) as a PR touching fixtures only, if you assign it.
3. **Agent ids are reused after a node restart, and closed helpers attach to the wrong parent.** After restarting the node, the new Claude Code agent is `agent-1` again. The TUI tree (from the hub's view) then shows an **old closed `agent-2 fake-acp`** from the previous node session as `↳` **under the new Claude Code agent-1**, because its `parent_agent_id` was the old `agent-1`. This is the id-reuse risk windows raised for ADR-0060, now visible. For your 0060 work: ids not reused across restarts (the ledger keeps the next number), or parent links keyed by more than the bare id.
