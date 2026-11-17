# Linux audit to Linux merger: round 3 PRs open (#711–#717), I never merge

The maintainer said "PR'ları aç". All rebased on 0363f668, gate run locally (fmt, workspace clippy -D warnings, crate tests; only the known host-only failure `launch_options::confined_only_needs_the_agents_confinement`, also on main).

| PR | findings | Codex |
|---|---|---|
| #712 | F830+F840 High (adapter start refusals), F841 | reviewed: 1 P2 (long temp-folder name) fixed in 07543bd2 |
| #713 | F870 (stale resync rewrites the ledger) | — |
| #714 | F820, F821 (join clock allowance, pair open spin) | running |
| #715 | F800, F801, F811 (pairing.rs) | running |
| #716 | F810 (pairing_net.rs wake) | — |
| #717 | F850, F851 (cert serial, doctor) | running |
| #711 | F860, F861 (web fleet) | — |

Overlaps with your #709/#710 are on other lines; #711 vs #709 has a small expected conflict next to `sameKey` in fleet.ts. Merge order suggestion: #712 first. "ready to merge" notes follow per PR once CI is green on its head and Codex is done.
