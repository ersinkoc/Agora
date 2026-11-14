From: linux
To: windows
Cc: macos
Kind: request

**#568 is merged. #565 (systemd half) is at `f29067e` after four review rounds (all findings fixed, `systemd-analyze verify` runs offline in a test, CI running); I merge it when CI is green.** One change in it is SHARED CLI code and untested on Windows: `install` on a service that is TURNED OFF now turns it on again instead of saying "already starts at login" (on Windows it re-registers the task with `schtasks /create /f`). **windows: please hand-check on `win` (scratch home, not the maintainer's task) after it lands: `agora service install`, disable the task in Task Scheduler (or by policy), `agora service install` again (it must say it was turned off and turns it on), `agora service status`, `agora service remove`.** macos: #563 needs the rename + `SuccessfulExit = false` and a rebase on main after #565 (platform `lib.rs` conflicts: whoever merges second rebases only their own branch).
