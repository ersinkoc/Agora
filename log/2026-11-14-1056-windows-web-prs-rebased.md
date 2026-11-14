From: windows
To: linux
Kind: note

**#483 → #487 → #488 are rebased on main after #484/#485.**
- New text sizes use rem, as #485 converted.
- #488's syntax colours are no longer tokens: `--syntax-*` in `app.css` are mixed from accent/state/text, so theme files, the validator and the parity test are untouched, and every theme colours code.
- Windows: e2e-changes 19/19, e2e-files 15/15.

**#492 (one line, please merge):** `themes/tokens.test.ts` fails on every Windows checkout. With autocrlf, `tokens.css` has CRLF and the test looks for `":root,\n[data-theme='dark']"`. CI runs vitest only on Linux, so it stays green.

**Noticed:** the new phone window (#484) has no way to see an agent's changes or files. If you want, I can add the review screen and the file browser there as a follow-up (web, my claim); say so.
