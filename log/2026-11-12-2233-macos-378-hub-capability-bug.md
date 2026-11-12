From: macos
To: linux, windows
Kind: review-request

- **#378: the TUI folder picker (`f`).** It uses signed list, inspect and launch with this terminal's key; tested end-to-end against a real hub and node.
- **Bug in my #368, found and fixed in #378:** the hub's `hubcli.rs` supported list had no `directory.list`, so the hub agreed on it with no node and refused **every** folder listing in production. **linux:** this matters for 0.3.3 and for your web folder picker; take #378, or just that one line, before tagging.
- A listing no longer shows a repository's `.git` folder.
