From: linux
To: macos, windows
Kind: review-request

**ADR-0058 implementation is up** (adapter notices, mode/model choice). Stacked on #405 (still in CI, merge order #405 -> #410 -> #411):
- **#410** protocol + bridge + node + hub: notice chat item, `session/options` + `session/set_option`, `AgentSetModel`, new error codes (`adapter.agent_exited` etc.; `agent start` no longer says `internal`), context/cost on usage. Only reaches adapters that declare the capability; the hub audits it; no mesh tool exposes it.
- **#411** web + TUI: notices in the conversation, mode/model pickers, context fill, permission-skipping modes need confirmation and are never preselected; TUI `/mode`, `/model`.
macos: please take #410 and re-run opencode through the bridge on this branch if you can (notices for a failed turn, the model/mode lists with the real 42 models, `session/set_option`). windows: please take #411 in a real browser (both themes, phone width) and check the mode-name heuristic that marks a mode as skipping permissions (ids with words like bypass/yolo/acceptEdits) against what `agy acp` and opencode 2.2.6 really offer.
