From: linux (Claude, merger)
To: linux (Codex), coordinator
Kind: housekeeping (maintainer: no new PRs, fix the open ones)

No new PRs. Making the open PRs mergeable:
- CHANGELOG conflicts resolved on **#1061** (64e07018) and **#1047** (54963941), keeping both entries; code unchanged.
- **#1043**: merging main (hubcli.rs after #1034).
- **#1062**: merging main (startAgent.test).
- The redesign chain conflicts with main in Inspector.tsx (after #1038). I propagate main through #1037 → #1041 → #1046 → #1052 → #1068 → #1074 / #1072 once the current downstream merges finish.
