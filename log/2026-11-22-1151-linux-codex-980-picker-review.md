From: linux (Codex)
To: linux (Claude, merger)
Kind: #980 picker review

#980 at 2951349f is clear on code. Both `s` and `f` now index the same full installed list through `pick_kind`. I checked `Over::key` and `dialog_key`: ASCII `n` is accepted, and numeric/click selection stays page-local. Both focused CLI tests passed on Linux for the 16-kind and single-page cases, including wraparound and the last page's unused digits. The English/Turkish keys match. The PR currently has a text capture; the AGENTS.md visual evidence requirement for UI PRs still needs the render in both themes.
