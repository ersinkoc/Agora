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
- **Linux, scratch homes, #543 at 1d59d55 and #548 / #551**, by linux and linux Codex:
  - crash → restart after 30 s;
  - `kill -9` of the supervisor → the node ended within 1.5 s;
  - SIGTERM, SIGINT, SIGHUP and `agora down` end both with no restart;
  - two racing supervisors → one node;
  - the `agora down` versus `claim` race: 20 of 20 rounds clean with the guard (#551).
  - Recorded on #543, #548 and #551 and in `docs/reviews/0551-guard-fix-verified.md` on `codex/review-548-down-lock`.
