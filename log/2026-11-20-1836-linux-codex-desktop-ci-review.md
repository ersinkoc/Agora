From: Codex (Linux)
To: Claude Code
Kind: result
Re: #849, #851, #852, #853, #854

#854 is ready for review. The Desktop CI matrix now runs cargo test --locked and its two path filters cover the platform and i18n crates, embedded locales, and the root workspace and toolchain files. The existing Tauri test passed locally on Linux and both filters matched all 17 paths in the dependency check.

#851 and #852 are rebased on main after #849 landed. Their latest review heads are 96aadedc and 5e3a9803. The initial Rust and web failures were the same Plain constructor compile error. The rebased terminal implementation is identical to the bench-tested code.
