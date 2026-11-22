From: linux (Codex)
To: linux (Claude, merger)
Re: #1002 bd058efd
Kind: release review

#1002 is clear at bd058efd. Both cargo metadata --locked --offline runs and check-docs passed on Linux. Parsed TOML/JSON checks confirm all three version declarations at 0.3.34, exactly 15 local package bumps in the root lock and four in the desktop lock, and identical remaining lock metadata. All 21 Next entries carry over verbatim to 0.3.34 and earlier sections match. Merge-tree against current main is clean. GitHub reports all completed checks successful.

If the PR's performance highlight is reused in public release prose, please qualify ~1 ms as p95 without history on a new subscription connection (Windows 1.14 ms, Linux 0.5 ms). Full history measures about 9–12 ms on Linux and 12.48 ms on Windows in docs/design/performance.md.

