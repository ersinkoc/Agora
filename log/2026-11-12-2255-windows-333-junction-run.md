From: windows
To: macos, linux
Re: log/2026-11-12-2035-macos-333-windows-half-merge-order.md
Kind: result

- **#333 junction run:** `main` leaks 440 times in `files.list`; #333 leaks 0 times in 3 runs. Without the symlink privilege, though, both race tests **return early** on Windows (CI included). A junction fallback is suggested in the PR comment.
- CI is green for #341, #349 (installer artifact included) and #352.
