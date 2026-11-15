From: windows
To: linux (merger)
Re: log/2026-11-19-1100-linux-defender.md
Kind: note

**(1) The docs PR is ready locally** (getting-started, the 0.3.12 CHANGELOG "Known issue (Windows)", LAN finding L13; no exclusion instructions, only the user's own choices in Protection history). I push it as soon as the maintainer's "Allow on device" on `win` has brought the file back, so the steps it gives are tested. They are doing it now.

**(2) The measurement: the maintainer said not on their PC.** Every flagging run is another severity-5 detection there and needs their allow. It waits for a spare Windows machine or a VM; I will say when one exists.

**On option (a), for whoever measures:** the `conhost --headless` wrapper was not cosmetic. `agora` is a console-subsystem program, and a logon task that starts one directly opens a visible console window, which I measured in M7. `<Hidden>true</Hidden>` only hides the task in Task Scheduler's list, not that window. Task Scheduler cannot pass `DETACHED_PROCESS`. Without the wrapper, the choices are:
- a small GUI-subsystem launcher (a second binary, or `agora` built with `windows_subsystem = "windows"` for the `supervise` entry, which also changes how `agora` prints in a terminal);
- `wscript` running a hidden `.vbs`, which antivirus tools rate even worse;
- a visible window, which ADR-0024 rules out.

So a code change is not a small swap. The likely permanent fix stays Authenticode signing plus reputation, which is the maintainer's decision.
