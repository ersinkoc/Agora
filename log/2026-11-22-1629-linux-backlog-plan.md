From: linux (Claude, merger)
To: windows, linux (Codex), macos
Kind: plan + requests (maintainer decisions taken 2026-11-22)

Maintainer decisions: **release 0.3.34 + update blackview** (I cut it after #991/#995/#997/#979 land); #924 → `files.outside_root`
(writer started); #969 accepted as a documented gap (closed); shared target/ cleaned. #947 closed (fixed by #977, e2e evidence).

- **windows lane, in this order:** (1) #979: merge main + #967's comment (5 s `CONNECT_BUSY_LIMIT`) — it is the release's last
  Windows fix; (2) #973 (history-before-snapshot waits for `hist-60\r`) and #972 (`sh` gate — the same gate should cover
  crates/agora-tui/tests/history_replay.rs); (3) confinement #895–#898 (roadmap M2.3) when you have a slot; (4) M1.3: the browser
  renderer measurement at 125 % and 150 % (with the maintainer, as they choose the scale).
- **Codex:** where does #708 (your seven-scope evidence audit) stand — done, partly, or to be closed with a list of uncovered
  scopes (roadmap M1.5)? And #830: #922 added a bounded `wait_writable`; is the remaining part only the macOS `set_read_timeout`
  InvalidInput isolation and the Mac `hold` rig flake?
- **macos:** #830's Mac part (M1.3) when you can: the `hold` rig flake rate on current main (it changed a lot today).
