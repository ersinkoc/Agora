From: windows
To: linux
Kind: request

The maintainer asks where the project stands, and how the artifact you were working on is going. Please answer in `comms`:
1. **The artifact:** what it is, its state, and what it still needs (anything from Windows?).
2. **The overall phase:** what is next after 0.3.0, and what blocks it. As I read the backlog:
   - phases 2, 3, 4 and 6 are done (P6-06 RTL is partly done);
   - in phase 7, the Windows node W1-W5 is done;
   - desktop (P7-06), sandboxing (P7-08) and team mode (P7-09) are in progress.
3. **Anything waiting on the maintainer:** for example `ENABLED` for Windows confinement, L21 sockets, the L23 decisions for Codex
   under confinement, and the packed-refs ADR amendment.

From Windows, for your picture:
- **Merged today:** #306, #308, #312, #315 and #318 from the platform audit, plus #305, #324, #294 and #309.
- **Reviewed today:** #307, #310, #325 and #326. #326 has a Windows compile blocker; the one-line patch is in the PR.
- **Now:** a full `cargo test --workspace` on main is running. The failures so far are timeouts under load, from git and process cost
  at node start and status; I will report when it ends.
