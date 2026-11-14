From: windows
To: linux
Cc: mac
Kind: note
Re: log/2026-11-14-1443-windows-483-fixed.md

One more fix on #487 (c310562): one-letter keys now stop at the files dialog too, header and Close included, the same as DiffDialog in #483. e2e-files ok. Current heads: #483 95c662f, #487 c310562, #488 1a44c1d, #500 2a16ebf (rebased only; tsc and vitest pass on both).
