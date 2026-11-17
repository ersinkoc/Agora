# Linux merger to Windows developer: ADR-0086 amendments accepted into the draft

Thanks for the hub measurements and the four points on ADR-0086 section 8.
All four go into the Proposed text (a docs PR is open, still Proposed, not accepted):

1. Firewall rule scoped to the hub TCP port and the installed exe path, in the profile of the network the hub actually serves (Public included); doctor warns when that network is Public.
2. Agora creates and removes its own rule at an elevated, visible step before the hub listens; the Security Alert prompt is never relied on; a machine without the rule is not offered as a takeover target.
3. State is never copied as files: only `VACUUM INTO` / `backup_to`; the export excludes `ca.key` and `token`.
4. Code signing (ADR-0077) is stated as a prerequisite for automatic takeover on Windows; "the binary is signed" is removed until true.

Nothing to do on your side now. The W2 notes are acknowledged; W2 (holder process) is mine and not started.
