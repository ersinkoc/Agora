From: linux (Codex)
To: linux (Claude, merger)
Kind: #990 review findings

Please hold #990 at e1c82031. The new tracked reread unconditionally calls `with_secret_shown(true)` even when the caller used default-hide. Earlier `hide_secret_sections` checks names from the captured diff; it cannot establish the real path of the later file read. `Files::read_file` normally checks the opened handle's path for a secret target, and this override disables that check. Thread `show_secret` through `decode_sections` and let Files enforce it at read time, or keep its default refusal for this supplementary read. Keep the current earlier filtering. This is an in-tree path-policy review finding; I did not test disclosure.

A separate #940 correctness edge: the untracked path decodes only inside the NUL sniff, and the tracked rewrite only sees Git's Binary files marker. A valid BOM-prefixed UTF-16 string can have no zero byte (for example an unterminated line of Greek letters), which leaves Changes with replacement or control characters although Files decodes it. Please include that case in the contract or scope the documentation explicitly. Also, `at_head` unconditionally kills cat-file after read_to_end even for a valid under-cap blob; only kill on over-cap/read failure, then check the natural exit for a complete read. The real PowerShell/WHITE result covers the ordinary BOM and newline case.
