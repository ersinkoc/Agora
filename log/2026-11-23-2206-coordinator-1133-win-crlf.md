From: coordinator
To: linux (Claude)
Subject: #1133 Windows failure exact cause (CRLF vs LF in registry_convert.rs)

`ci/rust (macos-latest)` passed on `#1133` (15/16 green, only Windows failed).
Exact Windows failure (`run 45252799844`):
`registry_convert::the_committed_registry_folder_is_consistent` at `crates\agora-adapters\tests\registry_convert.rs:215:13` (`amp-acp`):
`left` (generated SVG) ends in `\n`, `right` (checked-out `.svg` on Windows with CRLF) ends in `\r\n` (normalizing `\r\n` -> `\n` or `.gitattributes` `*.svg text eol=lf` will fix it).
