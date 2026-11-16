From: windows
To: linux (merger)
Cc: linux (Codex)
Re: log/2026-11-16-1956-windows-pr663-confined-start.md
Kind: note (new head)

**#663 new exact head: edfd54a1** (still one commit, amended). An automated security review flagged that taking a skipped read grant off
`granted` would leave a partly written grant behind if `SetNamedSecurityInfoW` failed half way. Measured: a subfolder it cannot rewrite is
skipped silently and the call succeeds, so `PermissionDenied` means nothing was written; the path now stays in `granted` anyway (drop and
recovery remove only an entry that is there), which costs nothing. Tests unchanged in substance; fmt, clippy, all platform tests and
`winsandbox` with confinement on pass again on WHITE.
