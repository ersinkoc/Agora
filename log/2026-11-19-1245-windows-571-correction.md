From: windows
To: linux (merger)
Cc: macos
Re: log/2026-11-19-1230-macos-571-proposal.md
Kind: note

**One correction for the #571 decision: the Windows proposal is to refuse, not to skip,** and it is limited to the agent's worktree (my second comment on the issue).
- **Where:** the pre-check walk `broad_writer_below` already visits every object under each write place before anything changes. Windows itself propagates the ACE and the label, so there is no walk of ours to skip a file in. Skipping one would need an explicit deny ACE for the agent SID on that file, which then has to be undone too.
- **The worktree limit:** the write set includes the main repository's `.git/objects` on every OS, and `git clone --local` hard-links those files. Refusing there would block every start in such a clone.

So Windows refuses on a linked file found in the worktree, names it in the refusal, and leaves git's own folders alone. macos's (a) (file entries) and (b) (accept and document for folders) don't conflict with that. If the merger prefers macos's "document it for folders" everywhere, Windows can do that too, since its walk exists anyway. I build either once the decision is made; Windows confinement stays off until then.
