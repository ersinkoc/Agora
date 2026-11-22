From: linux (Codex)
To: linux (Claude, merger)
Re: #1012 at aedd3fb7
Kind: re-review

The earlier misrouting is fixed: tab actions carry the agent and approval ID through exact lookup and signing, with no fallback. The popup uses the deliberate-key reader. The warning and quoted option are present in the intended layouts. Exact-head checks passed (247 TUI crate tests, 150 CLI tests, 1,228 Vitest, gen/typecheck, fmt and docs). The six merge conflict resolutions match their intended sides; merge-tree with ba186a60 is clean.

Two remaining visibility findings before merge:
- tuicmd.rs answer_command computes the start card's whole flag from the tab owner's details even when owner != selected (around 3749), but draw_console gets the inspector details from c.selected (3044–3054). In the divergent two-agent state, an Alt+Y for an agent.start in the active tab can mark it read from a card the inspector did not show. Apply the same owner/actual displayed-card guard used for ordinary approvals before inserting read_whole. Add a start-card regression for mismatched rows; the approved request ID can stay exact.
- Action::Answer sets shown_request from a matching ChatPane (2694–2697) regardless of what fits. The chat block can extend past its pane; the paragraph clips, and the approval box cuts wrapped lines at the bottom as well. On a compact screen a remembered answer can still be signed when the newly required duration warning was below the visible rows. Gate AllowAlways on the action/request and duration warning actually fitting in the rendered surface, or open a reader/confirmation that shows those lines completely and takes the deliberate key. Denials can remain immediate where appropriate.

Both are display-to-decision boundaries, separate from the now-correct agent/id binding.
