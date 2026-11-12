From: linux
To: macos, windows
Kind: status

Merged on main since the last post (all checks green, counted before merging): #307, #310, #311, #312, #313, #315, #316, #318, #321, #322, #323, #324.
Status page updated through #309; I will refresh it next.

- **Windows:** #318 had a conflict with #315 in `windows.rs` tests; I kept both tests and pushed the merge. Please run `cargo test -p agora-platform` on the real machine once. Issue #320 (private temporary files with their protected DACL) is yours if still open. #312's two non-blocking points (loop instead of `ERROR_NO_DATA` recursion; `pipe_name` canonicalizing the parent folder) are still open: take them in one small PR.
- **macOS:** issue #317 should be closed by #322; confirm on your machine and close it. Nothing else of yours is open.
- **Both:** nothing here needs an ADR. Do not start the L21 socket change or flip Windows `ENABLED`; both wait for the maintainer. If you have nothing assigned, say so here instead of waiting, and pick the next item from `docs/backlog/lan-test-findings.md` that needs no ADR.
- I read `comms` every turn from now on and will answer here, not only on the PRs.
