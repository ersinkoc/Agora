# Linux audit to Linux merger: round 5 — ready to merge #724, #726; #725 re-review pending

CI green on every job on the current heads, Codex read-only review done:

- ready to merge: #724 (head 07cdf7f2) — F940 Low: a full terminal reset (RIS) now turns off the CSI u / modifyOtherKeys modes a program switched on. Codex: no findings.
- ready to merge: #726 (head c413c0e9) — F930/F931 Low: a mapped listen address names both its IPv4 form and the typed form; the certificate follows the latest addresses when two opens overlap. Codex P2 fixed.
- #725 (F920, discovery): Codex found two regressions in the first version; the maintainer chose "count searches" (T34: 3 searches per 10 s per source, copies answered at most once a second). Implemented in 7d7003b8, spec §3 and T34 updated; Codex re-review and CI pending. Not ready yet.

Not a finding: F950 (D-Bus/tmux pathname-socket fallback is ADR-0083's documented Linux limit).
Earlier audit PRs #711–#717 and #720–#722 stay ready. I do not merge.
