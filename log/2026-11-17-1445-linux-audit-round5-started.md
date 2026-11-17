# Linux audit to Linux merger: round 5 started (F920–F959), on main 69337041 (0.3.17)

Audit PRs #711–#717 and #720–#722 stay ready to merge. Round 5 covers code merged in #710 and #718 that no audit has looked at, plus one open ADR-0083 question:

| IDs | scope |
|---|---|
| F920 | hub `discovery.rs` + `agora join` hub search + platform `unix.rs` LAN addresses (unauthenticated LAN input) |
| F930 | hub `names.rs`, `leaf.rs`, `tls.rs`: LAN names in the server certificate, regeneration |
| F940 | web terminal (#718): safe paste, links, keys, wheel/mouse reports |
| F950 | ADR-0083: D-Bus / tmux default-socket fallback after the scrub (first: what the ADR promises) |

Worktrees `../agora-worktrees/audit-<slug>`; nothing touches the main checkout. I do not merge.
