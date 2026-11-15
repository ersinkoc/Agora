# 2026-11-14: the supervisor and the login service on real machines

Real-machine evidence for ADR-0062 item 8 (restart after a crash, #543, #548, #551) and item 6 (start at login, #550, #553).
Each run used a scratch Agora folder (`AGORA_HOME`), never a live node; every task, process and file it made was removed afterwards.

## Windows 11 (WHITE, unelevated)

- **Restart loop on the live LAN node `win`** (M7 b, the measured design the supervisor was made from). The node, on 0.3.11 under its logon task, was killed at 19:46:59. The loop logged `node ended (exit -1 after 31 s): restarting in 30 s (restart 1 of 5 in 10 min)`, and the node was back at 19:47:34, connected to the hub.
- **Task Scheduler** (measured three ways): its restart on failure never fires for a program that ends after starting. `Stop-ScheduledTask` / `schtasks /end` end only the action's process (`conhost --headless`), not what it started.
- **Supervisor (#543)**, scratch folder:
  - a node killed after 11 s is started again 30 s later, and `agora status` says so;
  - a second `agora supervise node` refuses;
  - a supervisor killed outright (TerminateProcess) leaves no node;
  - `agora down` stops the supervisor and the node;
  - the supervised node has no console window of its own (first measured with one, then fixed);
  - an agent of a node that crashed ends with that node (its job), and the restarted node marks it interrupted in the ledger.
- **`agora service` (#550)**, scratch folders:
  - `install` prints the command, registers `agora-node`, and starts conhost → supervisor → node, with the node writing `logs/node.log`;
  - `status` → "starts when you log in … running now";
  - after `schtasks /change /disable`, `status` → "turned off in this system's settings";
  - `remove` → "Stopped the node and its supervisor … removed", leaving no processes, task or task file;
  - a second `install` changes nothing; a second `remove` → "was not set to start";
  - a relative `AGORA_HOME` becomes absolute in the task;
  - a folder with `%USERNAME%` is refused with the path named;
  - a folder with spaces, a trailing backslash and non-ASCII letters (`my hömé ş\`) runs;
  - `remove` with no service leaves a node started by hand running;
  - `install` while `agora` runs the node does not start a second one;
  - a symlinked `logs/node.log` is refused, and its target is untouched.

Not seen on a real machine: a task disabled by Group Policy (no domain here), and the absence of a console flash at a real logon (the live `win` task runs the same `conhost --headless` form).

## macOS and Linux

- **macOS, a real Mac (Darwin 27.2, arm64), #543 at 570ac016**, by macos:
  - the `bound` and `the_supervisor_restarts` tests pass;
  - a supervisor killed with `kill -9` takes its node within 2 s;
  - a crash is restarted after 30 s;
  - `agora down` leaves nothing.
  - Recorded on #543 (https://github.com/ersinkoc/Agora/pull/543).
- **macOS, `agora service` as a LaunchAgent**, by macos, on a scratch folder (`/tmp/agl`), with the label `agora-node`
  (first version, `KeepAlive = { Crashed = true }`):
  - `install` writes `~/Library/LaunchAgents/agora-node.plist` (owner-only, `plutil -lint` OK), loads it and starts supervisor → node;
    `status` → "starts when you log in … running now";
  - a second `install` changes nothing; after `launchctl disable`, `status` → "turned off in this system's settings";
  - `kill -9` of the supervisor takes its node with it, and launchd does **not** start it again (`Crashed` counts only crash
    signals); after SIGABRT it is back within 1 s; after SIGTERM (a clean stop) it stays ended. This led to the decision on #563:
    the supervisor exits 0 for every end it decides itself under a service (#564), and the job uses `SuccessfulExit = false`;
  - `remove` → "Stopped the node and its supervisor … removed": the job unloaded, the file gone, no processes; a second `remove`
    → "was not set to start".
  - Left behind: launchd keeps an `"agora-node" => enabled` line in its own override list after a `disable`/`enable`.
- **macOS, the LaunchAgent with `SuccessfulExit = false` and Agora's marker**, by macos, macOS 27.2, real launchd, scratch label
  `agora-handcheck-<pid>` running `/bin/sh` (the ignored test `launchd_restarts_a_failed_job_but_not_one_that_exited_0_and_leaves_a_hand_made_job_alone`
  in `agora-platform`, run with `AGORA_LAUNCHD_HANDCHECK=1`):
  - after `kill -9` the job was started again after 10.1 s (launchd's 10 s throttle); an exit 0 was not restarted;
  - turned off with `launchctl disable`: `status` said so, and `install` turned it on again;
  - a job file of the same label without the marker: `install`, `start` and `remove` each refused it (`NotOurs`), and the file was
    unchanged and never loaded;
  - afterwards no job, no file; launchd's override list keeps an `enabled` line for the scratch label.
- **macOS, the #563 follow-up** (Linux's review of #563), by macos, macOS 27.2, scratch labels, with the maintainer's OK given in the
  session for each run:
  - `bootout` (`agora service remove`) ends a child in the job's process group, but not one in its own session, as an agent in its
    terminal is: the agents keep running after the service is removed;
  - **Login Items:** the maintainer switched a scratch LaunchAgent off in System Settings > General > Login Items & Extensions
    ("Allow in the Background"). launchd unloaded it at once, `launchctl print-disabled` showed nothing, `sfltool dumpbtm` showed
    `disallowed`, and `+[SMAppService statusForLegacyURL:]` returned 2 (`requiresApproval`; 1 while it was on, 0 once its file was
    gone). `launchctl enable` did not change it; `launchctl bootstrap` loaded and ran it for that session while it stayed `disallowed`.
    So `status` reads `SMAppService`, and `install` refuses such a job and says only the user can switch it on there;
  - BTM marked each scratch job `notified` (the "Background Items Added" notice); its records went away with the job files;
  - `kill -9` → started again after 10.1 s; an exit 0 → not restarted (the ignored hand check, again).
  - Not measured: an SSH session with no login at the screen (`launchctl print gui/<uid>` failing → "no login session").
- **macOS, the LAN node `mac` moved to `agora service` on 0.3.12**, by macos with the maintainer's OK given in the session
  (`AGORA_HOME=~/.agora-lan`, a node-only `machine.toml`, hub `192.168.1.246:7444`):
  - `install` → "starts when you log in … started now"; `status` → running, the plist 0600 with the marker;
  - **Local Network privacy:** the node started by launchd got `No route to host (os error 65)` to the hub, while Terminal reached it.
    After the maintainer allowed `agora` (Privacy & Security > Local Network) the link came up within seconds. A node started
    from Terminal uses Terminal's permission, so this shows only under the service;
  - Not measured yet: whether a new binary from `agora update` is asked again.
- **Linux, scratch homes, #543 at 1d59d55 and #548 / #551**, by linux and linux Codex:
  - crash → restart after 30 s;
  - `kill -9` of the supervisor → the node ended within 1.5 s;
  - SIGTERM, SIGINT, SIGHUP and `agora down` end both with no restart;
  - two racing supervisors → one node;
  - the `agora down` versus `claim` race: 20 of 20 rounds clean with the guard (#551).
  - Recorded on #543, #548 and #551 and in `docs/reviews/0551-guard-fix-verified.md` on `codex/review-548-down-lock`.
