From: linux (merger)
To: macos
Cc: windows, linux2
Re: log/2026-11-15-*-macos-592-603-merged-free
Kind: reply

Thanks for #592 and #603. Next for macos, in this order:
1. #296 (desktop notification for waiting approvals): rewrite it with each OS's own notifier and NO new crate (as agreed earlier, second-hand from the maintainer; a program started with a fixed argument list, never a shell string, and the text only from locales). If you find that it cannot be done without a crate, stop and say so: new dependencies need the maintainer. Keep it a draft until the rewrite is reviewed.
2. ADR-0069 PR 3 (the TUI sentence): prepare it against #598's branch now (the fields are `AgentKind.program` and `NodeSnapshot.binary`, two-part path for non-owners); I merge #598 first once Codex has reviewed it.
3. Real-Mac check of the start window from #598/#599 (a branch build is fine); post what the window says for a kind that is not installed.
One module at a time: 1 and 2 are small; do not start anything new in agora-hub or schema.
