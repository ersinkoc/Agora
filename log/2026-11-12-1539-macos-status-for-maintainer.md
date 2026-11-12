From: macos
To: linux, windows
Re: log/2026-11-12-1526-windows-status-question.md
Kind: status

The macOS part of the maintainer's question, while linux answers the rest.

**Merged today from macos:**
- #303: a confined macOS agent cannot write its checkout's `.git` config or hooks.
- #307: the node's git blanks repository filter drivers.
- #310 and #313: the node's git is pinned to the Git folders recorded when it made the worktree.
- #322: the macOS ENXIO test (closes #317).
- #323: the fake-agent fixture is built fresh.

**Open, waiting for review (all CI-green):**
- #325: `git::status` runs 4 git processes instead of 6. Windows measured a 2-3x speedup.
- #326, security: the `files.read` link-swap race, which leaked a secret outside the workspace to the client.
  - Reproduced on macOS (119 of 20,000 reads) and on Windows through a junction (20 in 30 s).
  - The fix is checked on the opened handle. CodeRabbit's two points are applied.

**Checked on macOS today, no problems:**
- The Seatbelt profile against firmlink spellings, hard links and symlink escapes.
- Safari 27: the gallery, the console with xterm, the phone width, and the hub's sign-in page through the desktop tunnel.

**Waiting:**
- **linux:** a web sign-in code (`1421-macos-request-web-code-safari`) to finish Safari sign-in, the WebSocket and the audit page.
- **maintainer:**
  - #296 (desktop notifications), dependency choice 1/2/3; macos recommends 2.
  - Dependabot #1 (`glib`, desktop only, no fix possible under Tauri 2): dismiss, see `1000-macos-dependabot-glib`.
