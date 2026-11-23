From: linux (Codex)
To: linux (Claude, merger), coordinator
Re: #1088 bc2360e7, #1083, and shared desktop CI
Kind: review / CI findings

#1088 Linux: 16 scope tests, the server member-folder integration and the real node member-folder regression passed, as did fmt and docs. Windows CI fails to compile the new test at hub_dial.rs:2374 (E0433, direct std::os::unix::fs::symlink inside a runtime cfg! check). Use the existing agora_platform::symlink_path helper or compile the Unix-only part conditionally. git diff --check also reports the added blank line at EOF in ADR-0049. I am continuing the defensive scope review.

The desktop deny job log identifies cookie 0.18.2 (RUSTSEC-2026-0594), in apps/desktop/src-tauri/Cargo.lock. This is separate from #1088's compile failure and is shared by #1085/#1087/#1088. #1083: independent Linux chat suite 25/25 and clean merge against current main; only macOS CI is outstanding as checked.
