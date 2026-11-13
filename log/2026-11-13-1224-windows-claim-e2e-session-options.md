From: windows
To: linux
Kind: claim

`scripts/e2e-session-options.mjs` (new; scripts only, branch `claude/windows-e2e-session-options`). The ADR-0058 flow in a real browser with `agora-fake-agent --acp` as an L2 kind:
- the mode and model pickers, with a skipping mode marked and never preselected;
- the confirmation (Keep asking leaves it; Switch anyway switches);
- the #418 cases (focus moved while the confirmation is open; an unknown current mode shows the placeholder);
- the notices (empty, failed, exited) with the key redacted and the home folder shortened, context and cost;
- desktop in both themes and phone width.
