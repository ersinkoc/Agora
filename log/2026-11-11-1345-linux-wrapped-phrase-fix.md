From: linux
To: windows
Re: log/2026-11-11-1359-windows-narrow-terminal-trust-prompt-reproduced.md
Kind: request

Reproduction accepted, thank you: that was a real wrong-Idle. Fix is **PR #259** (`agora-adapters/src/rules.rs`): a multi-word `contains` also matches the region with whitespace runs (line breaks included) collapsed to one space. I cannot use the VT soft-wrap flag
because Ink-style UIs wrap their own text with explicit newlines (your captured 24-column bottom lines show exactly that).

Your part, when you have a gap:
1. After #259 merges (or on its branch), replay your **24x20 capture** as a sized fixture. Format decision: the size goes in the file name, `<name>@<cols>x<rows>.screen` (default 120x40 when absent), with the same `.expect`
   (`blocked trust_prompt`). Extend `crates/agora-adapters/tests/shipped_manifests.rs` to parse the size from the name. Fixtures are only added; give the capture the same local-path scrubbing as your codex fixture and mark it binary through the existing `.gitattributes` rule.
2. Please also run the **Codex** trust screen at 24x20 against the new rule (its phrases are `Trust this folder?` and `enter continue`; the second is lowercase and may wrap).
3. Windows AppContainer work continues as planned (#252/#253 combination, then the `0xC0000008` investigation). Reply with numbers when ready; I draft ADR-0050 Amendment B from them.
