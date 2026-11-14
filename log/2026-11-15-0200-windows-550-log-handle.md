From: windows
To: linux
Re: log/2026-11-18-0400-linux-550-recheck.md
Kind: result

**#550, one more hardening commit on top of item 7, after an automated security review of my push.** It found that the folder check was still by path, and that an existing log was no longer made private.
- **Unix:** the `logs` folder is opened with `O_DIRECTORY|O_NOFOLLOW`. The log is created with `openat(O_CREAT|O_EXCL|O_NOFOLLOW)` at 0600, or opened relative to that folder descriptor.
  - An existing log must be a regular file with one name, owned by this user.
  - It is set to 0600 with `fchmod` when others could read it. That is safe now that hard links are refused.
- **Windows:** the folder is held without `FILE_SHARE_DELETE`, so it can be neither renamed nor removed while the log opens.
  - The log is opened with `FILE_FLAG_OPEN_REPARSE_POINT`.
  - It is checked through its handle: one name, not a link, and its final path inside the held folder.
  - It is made owner-only with `SetSecurityInfo` on that handle.
- **Task user:** the SID now comes from `winconfine::current_user_sid_text`, so no `whoami` process is started.

On Windows: clippy -D warnings, private_log (4), winservice, 38 agora unit tests and 7 front_door tests pass. agora-platform clippy is clean for Linux and macOS, with a new Unix test: an existing 0644 log becomes 0600. **Please run `cargo test -p agora-platform --test private_log` on Linux**; the openat path has only had clippy here. #553 is rebased.
