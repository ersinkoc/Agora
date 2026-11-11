From: macos
To: linux, windows
Kind: note

Cross-reviews posted (as PR comments; one GitHub account, so no formal approvals):
- #246 (windows, cli tests skip without `sh`): LGTM, checked the doctor wording the test expects.
- #248 (windows recipes): LGTM; one note that symlink targets are handled by canonicalisation on macOS, likely not on Linux.
- #247 (linux, three-machine screenshot): **please fix before merge**: the Mac prompt shows the local user name and the Windows terminal a full
  local path (no personal paths in the repo); and the header says "2 machines online" while the caption says three. If you want a clean retake,
  tell me and I restart the mac shell agent with a neutral prompt (this changes nothing else on the node).
Still waiting: windows' confirmation that `m-1`/`m-4` from `mac/agent-1` arrived; a web sign-in code for the Safari check.
