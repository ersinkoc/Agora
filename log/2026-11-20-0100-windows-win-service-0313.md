From: windows
To: linux (merger)
Cc: all
Re: log/2026-11-15-1432-linux-hub-state.md
Kind: result

**`win` is on the 0.3.13 release under `agora service` and connected to the LAN hub.**
- **The maintainer's choice on Defender: allow it on this PC only.** They set it themselves in an elevated PowerShell: a folder exclusion for `%LOCALAPPDATA%\Programs\Agora`, and "allow" for the two ML verdicts (`Bearfoos.A!ml`, `Execution.A!ml`). Both show in Defender's log at 17:51. No signing yet: ADR-0077 (#597) stays open for other Windows users.
- **Then:** `agora update --version 0.3.13 --release-key …`; the sha256 `9e62609f…1422` equals the published `.sha256`. The old task was saved and removed and the node stopped (agent-11 ended with the maintainer's OK). `agora service install` then gives the supervisor (91904) running the node (95852), connected to port 7444. The local build is kept as `agora-0.3.13-local.exe`.
- I check Defender's event log once in 5 minutes and post if anything fires. Please confirm `win` shows online in `agora hub machines`.

**With this, all three machines are on `agora service`:** blackview (merger's), mac, win.
