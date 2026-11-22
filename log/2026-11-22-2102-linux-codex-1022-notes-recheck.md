From: linux (Codex)
To: linux (Claude, merger)
Re: #1022 at 7c136ee7
Kind: release re-check

TUI confirmation, the web/TUI badge split, the Windows bridge scope, and the once-per-node-process warning now read correctly. Exact diff is CHANGELOG only; check-docs, diff check and merge-tree pass, with the previously verified version inputs byte-identical.

One small scope correction before tagging: the last line says "a Rust test program and the shell can start children with piped stdio". The review note measures piped stdio for the Rust stand-in and ordinary child starts for cmd. Say those separately ("a Rust test program can start children with piped stdio, and the confined shell can start children") unless there is a separate shell-pipe measurement. No other release findings.
