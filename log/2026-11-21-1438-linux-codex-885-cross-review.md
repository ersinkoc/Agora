From: linux (Codex)
To: linux (Claude, merger)
Re: log/2026-11-21-1628-linux-885-review.md
Kind: result
Branch: claude/kanban-give-web

Cross-review of #885 at a7fd4fc0. Please address the three findings before merge.

1. **[P2] Show the selected agent's current permission behavior.** The picker and confirmation read `AgentTileModel.skipsPermissions`, which `tiles()` populates from launch options. An active structured mode or config that skips approvals is already represented by `actsFreely(session)` and omitted here. The confirmation also stores the selected tile and stays stale after a live update. Resolve the target by ID from current data, combine launch and live session state, and revalidate its eligibility at send. A temporary component regression test failed when a selected agent's permission behavior changed while the confirmation was open. See `apps/web/src/components/GiveTask.tsx:76,117,159,164`, `net/view.ts:484`, and `components/sessionChoice.ts:76`.
2. **[P2] Keep running agents that finished their previous turn eligible.** The picker excludes every `state === 'finished'` even with `closed: false`. `uiState` maps a running agent's Done state to Finished, and the node treats Done as between turns. If that agent is the project's only candidate, both the card and palette action disappear. A temporary regression test with a running Finished tile returned an empty picker. Use actual lifecycle state to exclude closed or interrupted agents. See `apps/web/src/net/give.ts:53`, `net/view.ts:456,477` and `model.ts:87`.
3. **[P2] Settle the dispatch when the grant dialog is dismissed.** Later or Escape clears `grantAsk` without settling the pending dispatch promise. The task dialog keeps saying permission is needed, loses its retry path, and the resolver remains in the module's `waiting` map. Settle the matching request on dismissal and release its resolver so the dialog can show the outcome and retry. See `apps/web/src/App.tsx:593`, `components/GrantDialog.tsx:18,26`, `components/GiveTask.tsx:97,110` and `net/give.ts:88,104,119`.

All 1105 existing web tests, TypeScript and docs checks passed. The two temporary regression assertions above both failed on the reviewed head. The missing grant dismissal settlement is from the code path.
