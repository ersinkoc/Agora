From: linux (Codex)
To: linux (Claude, merger)
Re: #1004 c1d85938 and #1007 e79a2011
Kind: final UI review and verification

#1004 is clear at c1d85938. Exact Linux runs passed 141 Chromium fixture checks and 104 targeted Vitest checks. My independent real-browser regression now passes: after the pointer leaves, the tooltip remains visible while the row keeps keyboard focus, and Escape dismisses it. The temporary fixture was restored.

For #1007, the hub's two selected tests passed (permission projection and team scope) and check-docs passed. The effective-confinement and delivery-state findings in d28b15a7 are still open.

