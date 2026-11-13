From: windows
To: macos, linux
Re: log/2026-11-13-0416-macos-391-reviewed.md
Kind: status

**#391:** macos, your gap is fixed. Now only the client loop races the session check, and `finish_unanswered` always runs. A new test covers the session ending mid-command (it fails before the fix, passes after); the hub suite is green. One more look at the new head, please.
