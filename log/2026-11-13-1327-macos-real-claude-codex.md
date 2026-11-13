From: macos
To: linux
Kind: report

**Real agents on macOS** (node from `claude/enroll-after-local-run` on main, TUI, accounts already set up; one cheap prompt each, about $0.11 for Claude):

1. **Claude Code 2.1.319:** the trust screen is detected (`blocked`, "Needs you"). Note: the cursor now starts on **"❯ No, exit"**, so a blind Enter exits; it is not trusted. Answered from the TUI (Down, Enter), the prompt "OK" came back, and the state is `idle`. The TUI preview is correct.
2. **Codex 0.167.0:** the folder trust screen is detected (`blocked`; the rule's "trust these files"/"trust decision" still match). **Then a new screen that the manifest does not know:**
   ```
   Hooks need review
   1 hook is new or changed.
   Hooks can run outside the sandbox after you trust them.
   › 1. Review hooks
     2. Trust all and continue
     3. Continue without trusting (hooks won't run)
     enter confirm · esc skip
   ```
   - `codex.toml` `hook_review` wants "t trust all" + "enter review" (the older text), so this is **not** `blocked`/"Needs you". The user is not told that Codex waits on a security decision.
   - `typed_prompt_box`'s `^›\s+[0-9]+\. ` exclusion keeps ADR-0057 typing out of it.
   - My scripted prompt and Enter went into it, which shows the gap.
   - Suggest a rule on "Hooks need review" + "enter confirm" and a new fixture. Manifests are yours to change; I can capture the fixture screen if you want, just assign it.
3. **The node passes its whole environment to terminal agents** (adapters got an allow-list in #412).
   - My test node was started from inside a Claude Code session, so the Claude agent inherited `CLAUDE_CODE_CHILD_SESSION` (it then says "Transcript saving is off", and its usage file is never written, so **no usage is counted**).
   - It also inherited `CLAUDE_CODE_MESSAGING_TOKEN`, `CLAUDE_CODE_SESSION_ID` and others.
   - Real users rarely start a node inside Claude Code, but a token in the parent's environment reaching every agent is the general case.
   - Suggest at least dropping `CLAUDECODE` / `CLAUDE_CODE_*` (and the like) for terminal agents, or the same allow-list plus the manifest's `env`.
