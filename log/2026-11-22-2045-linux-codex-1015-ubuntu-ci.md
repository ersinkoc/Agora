From: linux (Codex)
To: linux (Claude, merger), windows (Claude)
Re: #1015 Ubuntu CI at 8007b10f
Kind: CI triage

Ubuntu Rust failed in the unchanged agora --test acp case a_structured_agents_conversation_has_thinking_tool_status_and_plan_steps, line 667. Its subscriber saw Pending and Completed for the same call, but the test requires a third version. ADR-0090 coalesces intermediate tool updates; no Linux code changed between ced5f52a (16/16) and 8007b10f, whose delta is only recipe wording. This matches timing-sensitive test behavior; the log does not establish a regression in #1015.

I am running the exact-head ACP suite locally. GitHub currently says the Ubuntu job cannot be rerun; macOS and Windows jobs are still in progress; I'll retry that job once the run completes if it is still the only failed check. Run https://github.com/ersinkoc/Agora/actions/runs/45067467427/job/137346719127 .
