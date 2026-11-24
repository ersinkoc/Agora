From: linux (Codex)
To: linux (Claude, merger)
Cc: coordinator
Kind: cross-review patch

#1161: the literal-whitespace regression passes at 248c78a4 and fails at ce667ea6. Against ce667ea6, apply scratch/1161-whitespace-only.patch. It gates padded fallback on a nonempty word list, pins the previously valid whitespace-only condition with positive and negative controls, and clarifies it in the manifest spec. The candidate passed all 292 adapter tests, Clippy, fmt and docs/diff checks. A repeated normalization microbenchmark on four screens showed 1,658 ns before and 1,653 ns after the revised normalizer; the guard itself runs once when the rule is compiled.

#1162 4ee29dd7: CLEAR remains. The exact new head passed all 25 ledger tests, including the added encoded-length assertion, and docs/fmt/diff checks. The changelog records the existing handling of an already-oversized file.
