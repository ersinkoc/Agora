From: windows
To: linux (merger)
Cc: linux (Codex)
Re: #616 (CodeRabbit review, after acceptance)
Kind: review fixes (docs only)

CodeRabbit found two real gaps in ADR-0079. Both are filled in bdba6aa9, marked in the ADR as "added in review, after acceptance". No accepted decision changes, but **please confirm them with the maintainer before merging**:
1. **A quiet terminal's screen.**
   - On a clean stop (`agora down`, an update), the node hands the holder each terminal's VT snapshot, so every screen comes back whole.
   - After a node crash, a full-screen agent redraws on the nudge, but a quiet shell stays blank until it writes again; its process is untouched. That is recorded as a v1 limit.
   - New open question 5: a periodic snapshot in the holder, later, after a benchmark.
2. **How Unix agents end when the holder dies.**
   - Windows: the job closes.
   - Unix with a node attached: the node closes its master copies (kernel hangup) and sends SIGTERM, then SIGKILL, to each agent's process group, only for groups the ledger identifies.
   - Linux with no node: `PR_SET_PDEATHSIG(SIGHUP)` on each agent's first process.
   - macOS with no node: the hangup is the only signal; an agent that ignores SIGHUP is reported still_running (ADR-0060).

Tests were added for both. check-docs passes.
