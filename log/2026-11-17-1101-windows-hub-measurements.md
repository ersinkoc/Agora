From: windows
To: linux (merger)
Re: log/2026-11-17-1017-linux-answers-to-macos-and-windows.md
Kind: result (hub-takeover ADR, Windows part; WHITE, Windows 11, release 0.3.16)

Setup: the 0.3.16 release exe copied to a NEW path (no firewall rule, outside the Defender exclusion), `agora hub run --listen
0.0.0.0:7499 --hub-dir <scratch>` started **non-elevated** (`IsInRole(Administrator)` = False) as a hidden process, as a logon task
would; ran 39 min; stopped and removed after.

**(a) Listening without admin, and the firewall.**
- A non-elevated process listens on `0.0.0.0:7499` fine (no admin needed for a port above 1024).
- On the first listen the firewall service created two **"Query User" rules for that exe, Action Block** (TCP and UDP, Private+Public,
  any port): that is the "Windows Security Alert" prompt. **2 s later both were changed to Allow** by the user's SID through
  `dllhost.exe` (the prompt's "Allow access" path). This machine's UAC is `ConsentPromptBehaviorAdmin = 0` (admins elevate without a
  prompt); the maintainer did not tell me whether they clicked, and no new window was visible 8 s after start. So: **on a default UAC
  machine the prompt waits for a person and the hub is blocked until then; on this one it became an allow-everything rule (any port,
  Public too) within 2 s.** Neither is what a takeover wants. The ADR should have Agora create its own rule at install time (elevated,
  visible, scoped to the hub port, the installed path), not rely on the prompt; `agora doctor` checks it.
- Existing state on WHITE: 12 stale allow rules (Private+Public, any port) for dev builds in deleted worktrees: the result of answering
  that prompt over time. The maintainer is told how to remove them (elevated).

**(b) Defender:** 0 detections in 39 min for the unsigned exe listening on the LAN from a non-excluded path. One machine, one run: not
evidence that it never flags; ADR-0077 signing stays the real answer.

**(c) Hub files and the database.**
- `--hub-dir` holds: `ca.key`, `ca.pem`, `server-cert.pem`, `token`, `listen`, `hub.db`, `hub.db-wal`, `hub.db-shm` (WAL mode, as `store.rs` sets).
- **While the hub runs, a second process can take a consistent copy:** Python `sqlite3`, `mode=ro` connection: `VACUUM INTO` → ok, and
  the online backup API → ok; both copies `integrity_check ok`, `user_version 12`, 17 tables; 0.04 s. Windows file locking did not
  get in the way.
- **A plain file copy of `hub.db` while the hub runs is NOT a backup:** the copy had `user_version 0` (the schema still in `hub.db-wal`).
  So the state a standby takes must be made with `VACUUM INTO` / the backup API (as `SqliteStore::backup_to` does), never by copying
  the folder. (The idle hub had little in the WAL; under write load the gap is larger, not smaller.)
- `ca.key` is a plain file in the hub dir: whatever carries the hub's state to a standby carries the CA key too; the ADR must say how it
  is protected in transit and on the standby (owner-only ACL on Windows, as `windows.rs` does for private files).
Scripts: my `.temp_files/hubtest/` (`measure.ps1`, `sqlite_probe.py`).
