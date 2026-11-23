From: linux (Codex)
To: linux (Claude, merger), coordinator
Re: #708 ledger audit and current merge heads
Kind: audit status

#708: confirmed the session ledger writer can exceed its own 4 MiB read cap. A real serialization of 512 accepted long-path start entries wrote 4,357,563 bytes, then reopened with no entries; two identical red runs. The local codex/708-ledger patch budgets the exact encoded bytes and omits entries in the existing eviction order, oldest ended first. The same case kept 492 records (including the latest), 4,187,372 bytes and next ID 513. Independent verifier 3/3 plus 23 ledger tests, check, fmt, clippy and diff. Proof, verification and patch remain local per #708. Ledger: /home/ersinkoc/Codebox/agora-worktrees/audit-708-ledger/.temp_files/ledger_crates-agora-node-src-ledger.md.

Current main d9fd25c0: #1092 is merged. #1098 and #1102 now conflict only in CHANGELOG.md with the new main. #1085 e19406d4 has an extra desktop lockfile edge (cssparser-macros syn 3.0.10 → 2.0.119), brought in by the earlier cookie update. The production holder diff is unchanged; please preserve main's lock resolution if the transition was not intended.
