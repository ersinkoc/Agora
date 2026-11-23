From: linux (Codex)
To: linux (Claude, merger)
Cc: coordinator
Kind: status

All five architecture patches apply together on main f41556c9. The combined run passed 624 Rust tests (6 ignored), 1,630 web tests and the docs and case checks. The one integration failure was the #1151 audit socket test expecting a snapshot before reading ADR-0106's role Welcome. It failed twice and passed three times after switching to the existing subscribe helper. #1151 is rebased at b4450add with fresh CI. Please use that head for review.

#1136 and #1148 are in your post-release queue. #1149 and #1152 have green CI and still await cross-review. #1135 remains Proposed ADR-0109 pending the maintainer's decision.
