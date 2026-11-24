# Real-machine checklist

One entry per row of [`docs/capabilities.md`](../capabilities.md), in the same order and sections (roadmap M1.1). Each entry says
which systems it applies to, what to do on a real machine with the released Agora 0.3.33, what to see, and what evidence is already
written down in this repository. When a row is added to `docs/capabilities.md`, add its entry here in the same pull request.

This is a first draft written from the docs and the code, not from a run. Where a step turns out to be wrong on a real machine, fix
the step in the same pull request as the run's note. A step that ends in **(unverified)** could not be proved from the code or the
docs (a manifest or probe written for this list, a system tool's menu): expect friction there, and the first run turns it into a
verified step. Words in quotes, and the names of buttons, menus and sections, are the window's own text from `locales/en/web.ftl`
(the terminal client's from `tui.ftl`, the commands' from `cli.ftl`); a `{ $count }` in that text is written here as `N`, and an
agent's or machine's name as `<agent>` or `<machine>` (the window gives agents names of their own; a kind is shown by its
`display_name`).

## How to run it

- **One system at a time.** Linux first (blackview, the merger session), then macOS and Windows through comms (the `macos` and
  `windows` sessions).
- **Write each run as a dated note:** `docs/reviews/YYYY-MM-DD-real-machine-<os>.md` (`<os>` is `linux`, `macos` or `windows`). Put
  in it the Agora version (`agora --version`), the machine (name, OS version, CPU), the browser and its display scale, and then one
  line per RM id: **pass**, **fail** or **skip**, with what was seen (numbers and times where the entry asks for them). A skip says
  why (not built, needs a person, no second machine).
- **Failures become issues.** One issue per failure, linked from the note. The fix PR updates the entry if the step was wrong.
- **Then cite the note** in the "Real machines" column of `docs/capabilities.md` for each row that passed.

## Safety rules

- **No real accounts or tokens.** Use `agora-fake-agent` or the Shell (setup S2). A step marked **human, dedicated account** is run
  only by a person, with an account made for testing, never by an agent session.
- **Disposable everything.** Every run uses its own Agora folder (`AGORA_HOME`) and its own project folders under one run folder
  (`$RM`), new for that run (S0). Every file a step reads, writes or tries to reach is one the run made under `$RM`, with made-up
  content. Leave the machine's own configuration, login items and PATH as they are. The confined Git check in RM-33 uses a test identity.
- **Confinement on 0.3.33.** Use a disposable OS account or VM for any step that needs a `sandbox` policy. Release 0.3.33 writes
  the generated Git configuration outside `$RM` (#900). With ADR-0098 it lives in the node's own data directory at
  `$AGORA_HOME/data/git-config/`. See [Sandbox recipes](../security/sandbox-recipes.md).
- **Denied-path probes live outside the worktree and outside `/tmp`.** On Linux the confinement grants agents the shared `/tmp`, so a
  probe there proves nothing; probes go in the run's own folder under the home (S0). Before any `agora down`, `agora service`, `agora update` or `kill`, check that `AGORA_HOME` is set to the run's folder and
  that `agora status` shows the run's data folder. Never touch `~/.agora` (`%USERPROFILE%\.agora`) or the machine's own services.
- **Do not stop agents that are running on that machine.** blackview, WHITE and the Macs run the maintainer's agents. Stop only what
  the run started, and only by its id or process id from the run's own `agora status`/`agora agent list`.
- **Never install a login service on a machine that has its own Agora service** (the names `agora-hub`/`agora-node` are shared):
  the `agora service` steps are for a machine without one, or are skipped.
- **Clean up, only this run:** stop the S3 node (Ctrl-C in its own shell), then, with `AGORA_HOME=$RM/home`, check that `agora
  status` names that folder and run `agora down --agents`; then delete exactly that one folder (`rm -rf -- "$RM"`, never a pattern or a
  parent). The build of `agora-fake-agent` in `$RM/src` goes with it.

## Which entries a platform session can run alone

- **macOS session:** RM-01, RM-03 to RM-07, RM-08 (a private `sshd` on loopback, as on 2026-11-09), RM-10, RM-11 to RM-16 (the
  fake-agent steps), RM-18, RM-19 to RM-21, RM-24, RM-25, RM-27, RM-29, RM-30, RM-31, RM-34 to RM-36, RM-39 (unconfined), RM-40,
  RM-44, RM-47, RM-49, RM-51, RM-53, RM-55, RM-58, RM-59, RM-60 (the CI disk image); in a disposable macOS account or VM only (#900):
  RM-23, RM-33, RM-37, RM-38.
- **Windows session:** RM-02, RM-03 to RM-07, RM-10, RM-11 to RM-16 (the fake-agent steps), RM-18, RM-19 to RM-21, RM-24, RM-25, RM-27, RM-29, RM-31, RM-32 (checking the signature only), RM-35,
  RM-36, RM-39, RM-40, RM-44, RM-49, RM-51, RM-53, RM-55, RM-58, RM-59, RM-60 (the CI installer); in a disposable Windows account or VM only (#900): RM-23
  (confinement is off by default: turned on for that node only).
- **Needs a person:** the real-agent steps of RM-11 to RM-13, RM-18's install (no person-facing way to make a signed list yet), RM-26 with a second person, RM-32
  (signing account), RM-45, RM-54 (a real phone), RM-56 (a tailnet), RM-57 (a real tunnel).

## Common setup

The commands are for a Unix shell; on Windows use PowerShell, with `$env:AGORA_HOME = Join-Path $RM 'home'` for `export
AGORA_HOME="$RM/home"` (always inside the run's own folder from S0, never a fixed path). Every path that starts with `$RM` stands
for the run's absolute folder: quote it when you type it (`"$RM/home"`), and never run a step in a shell where `RM` is not set
(check `echo "$RM"` first). In the window,
a node is a **Machine** and an approval is a card that says "… asks permission" with Allow and Deny (under "Waiting for your answer").

**S0. Run folder, new for each run.** `RM="$(mktemp -d "$HOME/agora-rm-$(date +%Y%m%dT%H%M%S)-XXXX")"` (a new, empty folder whose
name no other run has; on Windows `$RM = (New-Item -ItemType Directory (Join-Path $env:TEMP ("agora-rm-" + (Get-Date -Format
yyyyMMddTHHmmss)))).FullName`, which fails if it exists). It is under the home on purpose: on Linux a node folder under `/tmp` makes
confined starts refuse (`control_token_reachable`, RM-23). **Print it and write it down:** `echo "$RM"` (an absolute path; every
other shell of this run sets `RM` to exactly this text). Then `mkdir "$RM/home" "$RM/proj"`; `export AGORA_HOME="$RM/home"`
(Windows: `New-Item -ItemType Directory (Join-Path $RM 'home'), (Join-Path $RM 'proj')`; `$env:AGORA_HOME = Join-Path $RM 'home'`);
in `"$RM/proj"`: `git init -b main && git commit --allow-empty -m init` (it uses the identity you already have; set one with `-c user.name=rm -c
user.email=rm@example.invalid` on that one command if you have none, never with `git config --global`). Check `agora --version`
says 0.3.33.

**S1. Hub and machine here.** With `AGORA_HOME` set, go into the run folder first (`cd "$RM"`; PowerShell `Set-Location $RM`): `agora` adds the folder it runs in as the node's project, so started from a checkout it serves that checkout (seen on Windows, 2026-11-21). Then type `agora --ui none --resume ask` (`--resume ask` answers the question "Bring
agents back by themselves after a restart? [Y/n]" for this run, so it is not asked; the machine file records `resume = ask`). To the
first question answer `1` ("1) Start Agora here: …"); to "Let other machines of yours connect? [y/N]" answer `n` (S3 uses loopback;
`y` only for the entries that need a second computer). If "Start Agora when you log in? [Y/n]" is asked, type `n` and Enter: **never
answer it with Enter alone**, which means yes and registers a login service. `agora status` shows the hub's address (the first free
port from 7443, so not the machine's own hub) and the data folder. Without `--ui`, a desktop opens the web window instead. Where a later step says to start
again with `agora`, it means `agora --ui none` with the run's `AGORA_HOME`.
`agora web` opens the window signed in (accept the hub's own certificate for this run only).

**S2. Fake agents and the Shell.** `agora-fake-agent` is not in the release: build it from the same tag,
`git clone --depth 1 --branch v0.3.33 https://github.com/ersinkoc/Agora $RM/src && cargo build --release -p agora-fake-agent
--manifest-path $RM/src/Cargo.toml`, and use `FAKE=$RM/src/target/release/agora-fake-agent` (`.exe` on Windows). Write these
manifests into `$AGORA_HOME/manifests/` (the node reads the folder again before it lists or starts agents; on Windows write paths as
TOML literal strings, `'C:\...'`). Each manifest has `version = "1"`, `min_engine = 1` and a `display_name` equal to its `kind`, so
the window shows the same name as these steps. None of these manifests is in the repository; each is written for this list from the
manifest spec and the tests that use the fake agent. The Linux run of 2026-11-21 loaded and used each of them as written here
(with the corrections below), and the Windows run of 2026-11-21 the Windows forms (PowerShell, `.exe`, TOML literal paths):

- `shell.toml`: `kind = "shell"`, `display_name = "Shell"`, `version = "1"`, `min_engine = 1`, `[process]` `launch = ["sh"]`
  (`["powershell.exe", "-NoLogo"]` on Windows). Shell is also a built-in kind: this file replaces it (the node log says so at every
  start), which is harmless.
- `rm-screen.toml` (a terminal agent read from its screen): `launch = [FAKE, "--script", "$RM/screen.toml"]`, with three `[[rules]]`,
  each with its own `id`, `state` and `priority` (a rule without `id` or `priority` makes the whole file unreadable, and `agora
  doctor` says only "The agent description rm-screen.toml could not be read."): `working` (priority 800) for `contains = "esc to
  interrupt"` on `region = "bottom_non_empty_lines(1)"`; `blocked` (900) for `contains = "Allow Bash"` on
  `bottom_non_empty_lines(4)` (that frame's last line is "  2. No", so one line never shows it); `idle` (100) for `regex = '^>'` on
  `bottom_non_empty_lines(1)` (not `'^> '`: the row's trailing space is trimmed, so that never matches). `$RM/screen.toml` is
  `crates/agora-fake-agent/fixtures/working-blocked-idle.toml` with `delay_ms` of 20000 on the first two frames and 600000 on the
  last, so each state lasts long enough to see (verified on macOS 2026-11-21: Working, Needs you at about 23 s, Idle at
  about 41 s).
- `rm-acp.toml` (a structured agent over ACP): `[structured]` `acp = { argv = [FAKE, "--acp"] }`. Its prompts are listed at the top
  of `crates/agora-fake-agent/src/acp.rs` (`plan`, `ask`, `rich`, `usage`, `mcp TOOL JSON`, `slow`, anything else is echoed). The
  shape is the one `crates/agora/tests/acp.rs` writes. It says `loadSession: false`, so a Resume (RM-20) restarts it without its
  conversation.
- `rm-chat.toml` (a structured agent through an adapter): `[adapter]` `command = [FAKE, "--adapter"]` (prompts `ask`, `cwd`, `crash`). The shape is
  the one `scripts/browser-harness.mjs` writes. Its approval answer is `approved`.
- `rm-probe.toml`: `launch = [FAKE, "--probe-directory", "$RM/probe-{pid}.json"]`, with the `[[launch.option]]` and
  `[[launch.setting]]` blocks of `crates/agora-node/tests/agent_spawn.rs` (`skip-permissions`, `model`, `effort`). The probe writes
  its folder and its arguments to that file and waits.
- `rm-helper.toml` (for RM-21's "Up to N" permission): the `rm-probe` launch line with its own file (`$RM/helper-{pid}.json`) and an
  `[mcp]` section: `method = "cli_flag"`, `path = ".mcp.json"`, `flag = ["--mcp-config", "{config_file}"]`. Only a kind with an
  `[mcp]` section (an agent) can be allowed more than once; a plain terminal program such as `rm-screen` is always "Only this one".

`agora agent kinds` lists every kind; one that is missing is named by `agora doctor`.

**S3. A second machine on the same computer** (when no second computer is at hand). In the first shell, a committed checkout for
it, the same project as `"$RM/proj"`: `git clone -q "$RM/proj" "$RM/proj2"`, then `git -C "$RM/proj2" remote remove origin` (the
source has no remote, so the clone keeps the same project identity without one); note the hub's port (PORT) from `agora status`.
Then **open a second shell** and keep it for this machine only. A new terminal does not inherit the first one's variables, so set
the run folder again by hand, to the absolute path S0 printed, and check it before any `agora` command:
`RM='/the/path/S0/printed'` (PowerShell: `$RM = 'C:\the\path\S0\printed'`); `[ -d "$RM/home/hub" ] || echo "WRONG RM: stop"`
(PowerShell: `Test-Path (Join-Path $RM 'home\hub')` must say `True`); only then `export AGORA_HOME="$RM/two"` and `echo
"$AGORA_HOME"` (on Windows also `$env:COMPUTERNAME = 'RM-TWO'`: a node names its machine after the computer, so
otherwise both nodes show as the same name in the window). In that shell: `agora hub code --hub-dir "$RM/home/hub"` gives CODE; `agora node enroll --hub wss://localhost:PORT
--ca "$RM/home/hub/ca.pem" --code CODE --id rm-two`; then `agora node run --project "$RM/proj2"` in the foreground. Check it with
`agora hub machines --hub-dir "$RM/home/hub"` (rm-two online) or `agora agent list` in this shell, not `agora status` (which reads a
machine file only `agora` writes). Stop it with Ctrl-C in this shell before cleanup.

**S4. Trust this browser on the machine.** If a start says this browser is not trusted: in the window, Settings > This device shows
its fingerprint; on the machine run `agora node trust --from-hub`, compare the fingerprint, answer `y`.

**S5. Speak as an agent.** Whoever connects to an agent's mesh socket is that agent. `agora agent list` gives the id; then
`agora mesh stdio --socket $AGORA_HOME/m/<agent id>.sock` and paste one JSON line per call: first
`{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"rm","version":"1"}}}`,
then calls such as `{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"task_create","arguments":{"title":"RM task"}}}`
(tools: `docs/specs/mesh-tools.md`).

---

## Machines and the hub

### RM-01 Node on Linux and macOS: terminals, agents, git, files
- **OS:** Linux, macOS.
- **Do:** S0, S1, S2. In the window: Start an agent > Where: this machine, Working folder `$RM/proj` > Which agent: Shell > Start.
  In its terminal run `pwd`, `echo hello > a.txt`, `git status`. Open its Files and Changes tabs. Close agent.
- **See:** the terminal echoes at once; `pwd` is `$RM/proj`; Files lists `a.txt`; Changes shows `a.txt` as Added; the agent goes to
  Closed with "Left behind" naming the change.
- **Evidence so far:** [LAN run 2026-11-11](../backlog/lan-test-findings.md): Linux and macOS nodes;
  [macOS 2026-11-14](2026-11-14-macos-confinement-and-terminal-client.md): a scratch node with shell, fake and Claude Code agents;
  [macOS 2026-11-21](2026-11-21-macos-update-0333.md): on the LAN Mac's old Agora folder, `data/`, `m/` and `link/` became
  `drwx------` at the first 0.3.33 start (#879).

### RM-02 Node on Windows: ConPTY, job objects, named pipe
- **OS:** Windows.
- **Do:** S0, S1, S2 on Windows. Start the Shell (PowerShell) in `$RM\proj`; run `Get-Location`, `"hello" > a.txt`, `git status`; resize
  the window. Start `rm-screen`. Run `agora agent list` (it reaches the node over its named pipe). Then `agora down` and look in Task
  Manager for the agents' processes.
- **See:** the Shell draws correctly through ConPTY and redraws after a resize; `rm-screen` goes Working, Needs you, Idle; `agora agent
  list` answers; after `agora down` no process of the run's agents is left (the job object ends them).
- **Evidence so far:** LAN run 2026-11-11 (lan-test-findings): a Windows 11 node with real Claude Code and Codex; outages on Windows,
  2026-11-14 (lan-test-findings R1 to R5); [supervisor and login service 2026-11-14](2026-11-14-supervisor-and-login-service.md) on
  WHITE; [dispatch through ConPTY 2026-11-21](2026-11-21-windows-dispatch-conpty.md) on WHITE. [Windows 2026-11-21](2026-11-21-real-machine-windows.md): pass.

### RM-03 Hub with the web window, many machines
- **OS:** Linux, macOS, Windows (the hub on each; machines of any system).
- **Do:** S1 with `y` for other machines; add a second machine (RM-07, or S3). Open the window; start a Shell on each machine.
- **See:** My machines lists both as Online with their system; both agents are in Your agents, each "on <machine>"; typing in either
  terminal works.
- **Evidence so far:** LAN run 2026-11-11 (lan-test-findings): a Linux hub with Linux, macOS and Windows nodes; the Windows web window
  pass (L14 to L20). [Windows 2026-11-21](2026-11-21-real-machine-windows.md): pass (#944: the system shown raw).

### RM-04 A machine is online while its link is (silence noticed in 25 s)
- **OS:** Linux, macOS, Windows.
- **Do:** S1, S3. Start a Shell on `rm-two`. Freeze `rm-two`'s node process for 60 s (`kill -STOP <pid>` on Linux and macOS; on
  Windows, Resource Monitor > CPU > right-click the node's `agora.exe` > Suspend process (unverified; the Windows run used
  `NtSuspendProcess`)), then let it go (`kill -CONT`; on Windows, Resume process (unverified)). Note the times.
- **See:** `rm-two` shows Offline with "last seen" (the Agents list: "offline · 20 sec") within about 15 to 25 s of the freeze (25 s after its last frame; the node pings
  every 10 s); Online again within a few seconds of the
  resume, by itself; its Shell agent is still there and answers.
- **Evidence so far:** lan-test-findings R1 and R5 measured earlier builds (the first version failed on `754a7b1`, the hub restart
  catch-up after ADR-0070 on macOS and Windows); the ADR-0070 silence check itself: none recorded. [Windows 2026-11-21](2026-11-21-real-machine-windows.md): pass (Offline at 18 s).

### RM-05 Old machines shown apart; forget one, clean up old ones
- **OS:** Linux, macOS, Windows.
- **Do:** S1, S3. Stop `rm-two`'s node. Old means not seen for 24 hours: either leave it 24 h, or only check Offline now. Then Your
  machines > the machine > Forget…; and `agora hub machines` and `agora hub machines forget --old` with `--hub-dir $RM/home/hub`.
- **See:** after 24 h the machine is in its own folded section, "One old machine" ("N old machines" for more), with "Clean up…"
  under it, and is not offered in the start window; Forget… asks "Forget <machine>?" with Keep and Forget; Clean up… asks "Clean up
  one old machine?"; then the machine is gone from the window and from `agora hub machines`; a machine that is Online again is not
  forgotten ("It is connected again, so it was not forgotten.").
- **Evidence so far:** [Windows 2026-11-21](2026-11-21-real-machine-windows.md): pass, without the 24-hour section.

### RM-06 `agora` on a fresh machine starts a hub and a node here
- **OS:** Linux, macOS, Windows.
- **Do:** S0 (an empty `AGORA_HOME` is a fresh machine to Agora). In `$RM` (S1: `agora` adds the folder it runs in as the node's
  project), type `agora --ui tui`: answer `1`, `n` to other machines, the
  question "Bring agents back by themselves after a restart? [Y/n]" (asked here, since `--resume` is not given: answer `n` and note
  what it says), and `n` then Enter to "Start Agora when you log in? [Y/n]" (never Enter alone). Then
  `agora status`; wait more than a minute (a node that ends in its first minute is taken for a failed start and not started
  again: "The node ended right after starting …, so it was not started again."), then kill the node process; wait 40 s; `agora
  status`; `agora down`. On a machine with **no** Agora service of its own:
  `agora service install`, `status`, `remove`.
- **See:** the first question appears once; the terminal client opens signed in; `status` shows hub and node with versions; the killed
  node is back about 30 s later and `status` says "restarted once in the last N minutes" with the last crash; `agora down` leaves no
  Agora process of the run; `agora service status` says `The node starts when you log in ("agora-node").` and "running now (process
  N)"; `remove` says `"agora-node" is removed: the node no longer starts when you log in.` and leaves nothing.
- **Evidence so far:** [supervisor and login service 2026-11-14](2026-11-14-supervisor-and-login-service.md): restart after a crash
  and start at login on Windows 11, macOS and Linux with scratch homes (0.3.11 to 0.3.12 era); joining a hub by its address: none. [Windows 2026-11-21](2026-11-21-real-machine-windows.md): pass (service part skipped: the machine has its own).

### RM-07 Pairing: find a hub on the network, approve on the hub
- **OS:** Linux, macOS, Windows (either side). IPv4 broadcast only.
- **Do:** on the hub machine S1 with `y`, then `agora pair open` (in a terminal). On a second machine (or a second `AGORA_HOME` on the
  same one) `agora join`, then `agora join HUB-ADDRESS`. Answer on the hub's terminal once. The hub closes itself after one approval,
  so for the second answer run `agora pair open` again and join from another disposable machine (its own `AGORA_HOME` and `--id`), then
  answer that one in the window (A new machine wants to join > Numbers match… > "Do the two numbers match exactly?" > Yes, the numbers match).
  Wait a second or two before each answer: a key
  typed right after a question is refused ("That key came too soon after the question to be an answer. …"). Join by a name the
  hub's certificate holds (the hub's own suggestion, `agora join <hub name>:PORT`); then join once by an address the certificate does
  not name (on the hub's own computer, `127.0.0.1:PORT` when the hub listens on all addresses).
- **See:** `join` says `Found the hub "…" at …. Its fingerprint is …` with the same fingerprint as `pair open`; the new machine says
  `Number N. Approve this machine on the hub: …` and the hub `New machine "…" (…) wants to join. Number N.`, the same number; after `y`
  the new machine says `Joined. This machine is "…"` and is Online in My machines; a denied or timed-out request ends with "Nothing was
  saved"; the hub closes itself after one approval, from the terminal or from the window (`agora pair status` says "Closed to new machines"), and a
  `pair open` that was asking about a machine answered in the window stops asking and says "Answered in a window: …" (#942). The join by an address the certificate does not name says at once "The hub at … could
  not be verified, so nothing was saved." with no number, the owner gets no card, and a later join with that name is not offered
  "Replace the existing machine".
- **Evidence so far:** [Linux 2026-11-21](2026-11-21-real-machine-linux.md): the terminal answer passed; the window's approval of a join by `127.0.0.1` failed (#901). [Windows 2026-11-21](2026-11-21-real-machine-windows.md): terminal answer passed; after a window approval the hub stays open (#942); the join hint on a loopback hub (#943).

### RM-08 Machines reachable only over SSH
- **OS:** hub on Linux, macOS or Windows; target Linux or macOS (a systemd or launchd user service, or the `tmux` fallback).
- **Do:** a throwaway target only: a VM, a test user account, or a private `sshd` on loopback with a scratch `HOME` (as on
  2026-11-09). `agora hub ssh add rm-ssh USER@HOST --hub-dir $RM/home/hub`, then `agora hub ssh install rm-ssh --hub-dir
  $RM/home/hub`.
- **See:** the install says each step (check, binary, checksum and signature, enroll); the target appears as a machine and stays
  Online through the tunnel; an agent started there works; a failed step says which and changes nothing.
- **Evidence so far:** [real SSH run 2026-11-09](2026-11-09-real-ssh-run.md): a private loopback `sshd`, with a published release's
  signature; no other system as the target.

### RM-09 Direct client ↔ node path for terminals
not built: nothing to check.

### RM-10 `agora update` (self-update)
- **OS:** Linux, macOS, Windows.
- **Do:** copy the program to `$RM/bin/agora` and use only that copy, with the release key from the repository:
  `$RM/bin/agora update --check --release-key $RM/src/docs/security/release-key.pub`, then `$RM/bin/agora update --version 0.3.32
  --release-key …` and `--version 0.3.33` (going back a version this way is unverified).
- **See:** `--check` says "Agora 0.3.33 is the newest release." (or which is available); each update says "Updated to Agora …"
  only after the checksum and signature checked out ("Nothing was changed." otherwise);
  `$RM/bin/agora --version` follows; the machine's own `agora` is untouched. On Windows, note any Microsoft Defender verdict. On
  0.3.33 `--check` failed on Windows ("could not find the newest release": `curl -o /dev/null`, #939); `--version` works. Fixed by #952: [2026-11-22 follow-ups](2026-11-22-real-machine-followups.md).
- **Evidence so far:** [macOS 2026-11-21](2026-11-21-macos-update-0333.md): the LAN Mac updated to 0.3.33 with `--version` and
  `--release-key`, the checksum equal to the published one, Local Network asked again after the update; Windows: none recorded
  (lan-test-findings L13 lists the self-replace among the suspects of a Defender verdict). [Windows 2026-11-21](2026-11-21-real-machine-windows.md): `--version` passed, Defender silent; `--check` failed (#939). [2026-11-22 follow-ups](2026-11-22-real-machine-followups.md): `--check` passes on Windows after #952 (exit 0; a wrong address exits 1); all three machines updated to 0.3.34.

## Agents

### RM-11 L1: terminal agents from manifests (built-in: claude-code, codex, wrongstack)
- **OS:** Linux, macOS, Windows.
- **Do:** S2. Start `rm-screen` in `$RM/proj`; watch it 60 s; in its full view hover the state badge: its title is "Why is this
  shown? …". Then **human, dedicated account:** start
  Claude Code, Codex and WrongStack each in a disposable folder; answer the folder-trust prompt; give a prompt; wait for the end.
- **See:** `rm-screen` goes Working, then Needs you, then Idle, in the list and on its tile, with "Read from what is on the agent's
  screen."; the real agents show Needs you on the trust prompt (wide and narrow window), Working during the turn, Idle after.
- **Evidence so far:** [phase 4](../backlog/phase-4.md) P4-05 (Claude Code and Codex manifests done, WrongStack open); LAN run
  2026-11-11, L7 and L8 (trust prompts on Windows); [#279](0279-narrow-trust-prompts.md): Claude Code's trust prompt at both sizes on
  a real macOS node; [macOS 2026-11-14](2026-11-14-macos-confinement-and-terminal-client.md): a real Claude Code agent showing Needs
  you. [Windows 2026-11-21](2026-11-21-real-machine-windows.md): pass (fake agent).

### RM-12 L2: ACP agents (built-in: opencode-acp, wrongstack-acp, antigravity-acp)
- **OS:** Linux, macOS, Windows.
- **Do:** S2. Start `rm-acp`; send `plan`, then `ask`, then `hello`. Then **human, dedicated account:** start opencode, WrongStack and
  Antigravity (structured) and give each one prompt.
- **See:** the agent opens as a chat (no terminal); `plan` shows a plan and a change; `ask` shows "asks permission" with Allow / Deny,
  and the agent says `allowed` after Allow; `hello` is echoed; the real agents answer, or a failed start shows the agent's own reason.
- **Evidence so far:** [Windows 2026-11-21](2026-11-21-real-machine-windows.md): pass (fake agent).

### RM-13 L3: adapter processes, installed from the signed registry
- **OS:** Linux, macOS, Windows.
- **Do:** S2. Start `rm-chat`; send `ask`, then `cwd`, then `crash`. Registry: there is no default registry; if one is published, run
  `agora adapter install NAME --registry URL`, `agora adapter list`, `agora adapter remove NAME` (**human, dedicated account** for an
  adapter that needs a login).
- **See:** the chat answers; `ask` brings an approval that Allow resolves (the agent says `approved`); `cwd` says the folder; one
  `crash` is restarted by the node with backoff (adapter protocol: 5 restarts, from 1 s up to 60 s) and the conversation says it is
  started again (attempt N), so send `crash` again after each restart: after the sixth (`agora agent list` says `stopped`) the agent
  shows "Ended with an error (exit code 3)", its summary (and the conversation in `agora tui`) says Agora stopped starting it again
  after 5 restarts, and its input box is off, also after a reload (on 0.3.33 it stays "Unknown", #902; with a holder the exit code may
  be missing until #951); an installed adapter shows its version in `list` and its kind in the start window.
- **Evidence so far:** [Linux 2026-11-21](2026-11-21-real-machine-linux.md): fail (#902). [Windows 2026-11-21](2026-11-21-real-machine-windows.md): fail (#902).

### RM-14 Agents run in the selected folder; worktrees optional
- **OS:** Linux, macOS, Windows.
- **Do:** Start an agent > Working folder > Browse folders > `$RM/proj` > Use this folder > Shell > Start; run `pwd` and `git branch
  --show-current`. Again with "Create an isolated working copy" checked. Close agent on both.
- **See:** the first runs in `$RM/proj` on `main` (the start window: "main · The agent will work in this folder. Existing files and
  the current branch are kept."; the Changes panel: "Shared folder · changes belong to the workspace"); the second in
  `$AGORA_HOME/worktrees/<kind>-<id>` on an `agent/<kind>-<id>` branch; after Close both worktrees and branches stay: removing a
  never-changed one is ADR-0064 (RM-22, not built), the node removes a worktree only to roll back a failed start (macOS 2026-11-21: an unchanged and a changed worktree
  both stayed after `agora agent stop`).
- **Evidence so far:** LAN run 2026-11-11, L20 and L22 (lan-test-findings): branch rollback and selected checkouts under confinement. [Windows 2026-11-21](2026-11-21-real-machine-windows.md): pass.

### RM-15 Launch options, including "do not ask" modes
- **OS:** Linux, macOS, Windows.
- **Do:** S2. Start an agent > `rm-probe` > How it acts > Don't ask > Start; confirm "Start without asking". Read `$RM/probe-*.json`.
- **See:** the confirmation appears before the start; the agent is marked with the red bolt (no words; its tooltip, on hover, focus or tap, says "Runs without asking: …"); the probe's
  arguments end with `--fake-skip-permissions`; without the option they do not.
- **Evidence so far:** [Windows 2026-11-21](2026-11-21-real-machine-windows.md): pass.

### RM-16 Start with model, effort and mode
- **OS:** Linux, macOS, Windows (the window; the terminal client's picker is not built).
- **Do:** S2. Start an agent > `rm-probe`, Model `m1`, Effort `high` > Start. Then from `rm-acp` (RM-21) send
  `mcp agent_start {"kind":"rm-probe","settings":{"model":"m1"}}`.
- **See:** the start window says "rm-probe will start with …" naming both; the probe's arguments hold `--fake-model m1 --fake-effort
  high`; the helper's card lists the model and says "Only this one can be allowed: a start with its own model or effort is always
  asked."
- **Evidence so far:** [Windows 2026-11-21](2026-11-21-real-machine-windows.md): pass.

### RM-17 Agent login and accounts on a node
not built: nothing to check.

### RM-18 Install and update coding agents on a machine from the window
- **OS:** Linux, macOS, Windows. Built: `agora doctor`'s agent checks and `agora agent install` (binary download); from the window:
  not built.
- **Do (a session can run it):** `agora doctor` with the run's `AGORA_HOME`.
- **Do (human):** there is no person-facing way to make a signed agent list yet. No `agora` command makes a list key or signs a
  list, no first-party list is published, and `docs/specs/agent-packages.md` defines only the format and the check (the first-party
  list is to be signed by the release key, ADR-0087, not done yet). So the install is run by the maintainer, by hand, with a key made
  for this test only and a disposable `AGORA_HOME`:
  1. Make an Ed25519 key pair with any tool you trust; keep the private key off every machine Agora runs on. The root key Agora takes
     is the raw 32-byte public key as 64 hex characters (unverified: no doc names a tool for this).
  2. Write the payload as `docs/specs/agent-packages.md` says: `version`, `expires_ms`, one publisher (`id`, `key`), and one
     `binary-download` entry for a small program for this system (an `https://` address on a public DNS name, its SHA-256,
     `provides`, `verify` with `--version` and a regex that matches its output). The `name` matches a manifest kind on the machine.
  3. Sign the payload's exact bytes with the private key (Ed25519, the signature as hex) and save
     `{"payload": "<the payload text>", "sig": "<hex>"}` as `$RM/list.json`.
  4. `agora agent install NAME --list $RM/list.json --root-key <64 hex>`; type the name to approve. Again with the input from a pipe
     (`echo NAME | agora agent install …`). To check going back: install a newer version from a list with a higher `version`, then
     the older entry again with `--go-back` (it asks for the version to be typed too).
- **See:** `doctor` names each agent kind as installed or not; the install prints the exact plan (version, address, SHA-256,
  publisher, target folder) and asks for the name; input that is not a terminal is refused; a wrong root key or a changed payload is
  refused before any download; nothing outside `$AGORA_HOME/agents` changes; the installed agent is "Installed" under Settings >
  Agents.
- **Evidence so far:** [Windows 2026-11-21](2026-11-21-real-machine-windows.md): pass for `doctor`.

### RM-19 A missing agent kind says why
- **OS:** Linux, macOS, Windows.
- **Do:** add a manifest whose `launch` names a program that does not exist (`["rm-no-such-program"]`). Open Start an agent; open
  Settings > Agents; run `agora agent start` for that kind; open `agora tui` and its start list.
- **See:** the kind is shown as "not installed"; Settings > Agents says "This machine cannot find rm-no-such-program in its PATH."; the
  CLI and the terminal client say why in words, not a code: `agora agent start` prints `Error agent.program_missing: rm-missing did not
  start: this machine cannot find rm-no-such-program in its PATH. ...` (#903; also for an ACP kind such as `opencode-acp` without
  `opencode`).
- **Evidence so far:** [Windows 2026-11-21](2026-11-21-real-machine-windows.md): fail (#903; nine kinds in the terminal client, #910).

### RM-20 Resume after the node restarts (session ledger)
- **OS:** Linux, macOS, Windows.
- **Do:** start `rm-acp` (and on Windows the Shell too). End the node in a way that ends its agents: on Windows `agora down`; on Linux
  and macOS `kill -9` of the run's node and of its holder (by the process ids the run's `agora status` gives), or a confined agent
  with RM-23's policy (only in a disposable OS account or VM, #900). Start again with `agora`; then `agora down` and start again
  with `agora --resume auto` (given while Agora already runs, `--resume auto` changes nothing and says nothing; it applies to a
  start).
- **See:** the agents are under "Stopped when the machine shut down" (the Agents list: "Stopped") with Resume or Start again; Resume starts the same agent in
  the same folder (the conversation comes back only for an agent that can load a session: not the fake, which says
  `loadSession: false`, and the window keeps no chat history of its own, RM-43); with `--resume auto` they come back by themselves (asked once), except an agent the node brought back by itself less than 10
  minutes ago (ADR-0062: a second restart within 10 minutes offers it with Resume instead); one still running from before says "Stop it".
- **Evidence so far:** [macOS 2026-11-14](2026-11-14-macos-confinement-and-terminal-client.md): a confined agent resumed after `kill
  -9` with `resume = auto`; [supervisor 2026-11-14](2026-11-14-supervisor-and-login-service.md): on Windows a crashed node marks its
  agents interrupted. [Windows 2026-11-21](2026-11-21-real-machine-windows.md): pass.

### RM-21 An agent starts helpers, with the user's signed approval
- **OS:** Linux, macOS, Windows.
- **Do:** S2, S4. Start `rm-acp`; send `mcp agent_start {"kind":"rm-screen","prompt":"hello"}` and answer Allow (`rm-screen` is a
  plain terminal program: its card says "Only this one can be allowed: this kind runs any program it is given, …"); send it again
  and answer Deny. Then `mcp agent_start {"kind":"rm-helper"}` (S2): its card offers "What to allow: Only this one / Up to 3
  rm-helper agents in this folder while rm-acp runs, without asking again"; choose the second, Allow, and send it once more (it
  starts without a card); then Revoke under Helper permissions ("Can still start N rm-helper without asking in …"), which is in the
  parent's pane on Home under its agent address (not in the full view), and send it again. An agent has at most 3 helpers at once
  (a fourth gets `mesh.helper_limit`): stop one first. The "An agent needs you" window may open over the card a moment after it
  appears and take the click: answer through its Review.
- **See:** a card says "<agent> asks to start a rm-screen agent" (with "on <machine>" when it names one), the Folder and "Its first
  prompt"; Only this one starts the helper, shown "started by <agent>" under its parent; Deny starts nothing and the agent is told; after Revoke the next start asks
  again; nothing starts without a card.
- **Evidence so far:** [Linux 2026-11-21](2026-11-21-real-machine-linux.md): pass. [Windows 2026-11-21](2026-11-21-real-machine-windows.md): pass (#945: helper rows unreadable at 125 %).

### RM-22 Workspace clean-up and disk limits
not built: nothing to check.

### RM-23 Agent confinement (sandbox)
- **OS:** Linux, macOS; Windows built but off by default (turn on with `AGORA_WINDOWS_CONFINEMENT=1` for the run's node only).
  One level per node; `worktree-offline` on macOS only.
- **Isolation:** follow the confinement rule in [Safety rules](#safety-rules) for the version under test.
- **Do:** write `sandbox = worktree` into `$AGORA_HOME/data/policy`; restart the node. Start a Shell with "Create an isolated working
  copy" and run `pwd` in it (its worktree, W; it must not be under `/tmp`, which Linux confinement grants to every agent: with
  S0's folder under the home it is not). In your own terminal make the probes, all synthetic: `mkdir "$W/../rm-sibling" && echo
  rm-synthetic > "$W/../rm-sibling/secret.txt"`, and `echo rm-synthetic > $RM/outside.txt`. In the confined Shell try `cat
  ../rm-sibling/secret.txt`, `touch ../rm-sibling/out`, `cat $RM/outside.txt` and `touch $RM/home/x` (type these two paths out in full: `$RM` is not set in the agent); then `touch
  in-worktree.txt` and `git add -A && git commit -m t` (only if you have a global identity, RM-33).
- **See:** reading and writing the sibling folder, reading `$RM/outside.txt` and writing the Agora folder are refused; the file in
  the worktree is made and the commit works; `agora doctor` says the machine confines agents (unverified: which folders around the
  worktree each system leaves readable).
- **Evidence so far:** [macOS 2026-11-14](2026-11-14-macos-confinement-and-terminal-client.md): every way out refused; Claude Code and
  Codex confined; LAN run L10, L11, L21, L22, L23 (Linux and Windows findings).

## People, approvals and coordination

### RM-24 Approvals signed by a device the node trusts
- **OS:** Linux, macOS, Windows.
- **Do:** S3 without S4 for `rm-two`: start an agent on it from the window; then S4 for `rm-two` and start again. Start `rm-chat`
  there, send `ask`, answer Allow.
- **See:** before trust, the start window says "<machine> has to trust this browser once before it starts agents from it, …" with
  the command and Copy, and Settings > This device says the machine "does not trust this browser yet"; after trust, the start and
  the Allow work; Activity (`#/audit`, or "Activity" in Ctrl K) lists "Started an agent in a folder" and "Answered an approval".
- **Evidence so far:** [Linux 2026-11-21](2026-11-21-real-machine-linux.md): pass. [Windows 2026-11-21](2026-11-21-real-machine-windows.md): pass.

### RM-25 Trusting your devices in one step
- **OS:** Linux, macOS, Windows.
- **Do:** S1 with browser A trusted (S4). Sign in from browser B (another profile). In B: Settings > This device > Ask another device to
  approve. In A: "<device> asks to be trusted" > compare the fingerprint > Approve. Then in A: Your other devices > B > Remove.
- **See:** B's fingerprint in A matches B's own; after Approve, B "trusts this browser" on the machines in scope and can start an agent
  with no command typed; after Remove, B cannot.
- **Evidence so far:** [Windows 2026-11-21](2026-11-21-real-machine-windows.md): pass.

### RM-26 Team mode: people, roles, audit
- **OS:** Linux, macOS, Windows (the hub). Partly built (P7-09).
- **Do:** `agora hub user add rm-viewer --role viewer --hub-dir $RM/home/hub`. Sign in with its code in browser B (a second
  person: **human**; otherwise a second browser profile). Then `agora hub user projects --hub-dir $RM/home/hub` and grant a project
  by the names the window shows: `agora hub user grant rm-viewer MACHINE/proj --hub-dir $RM/home/hub` (#904); also try
  `MACHINE/nosuch`. As the viewer try to start an agent. Then `agora hub user role rm-viewer member --hub-dir $RM/home/hub` and, in
  browser B, open Start an agent on that machine, browse the project's folder and a folder in it, and start a shell there; type
  another project's folder or `/` and choose it (#1049). As the member in `agora tui`, press `f` on that machine (#1086). With that `agora tui` still open, `agora hub user role rm-viewer owner --hub-dir $RM/home/hub` and press `f` again, then make them a member again (#1099). Make them a viewer, and in `agora tui` and browser B look at the footer and the start actions and press `f`, `s` and `x` (ADR-0106). As the owner open Activity.
- **See:** without a grant the viewer sees no machine and is told no project is theirs yet (no "Connect your first machine");
  `projects` lists `MACHINE/proj` with its folder; `MACHINE/nosuch` is refused with a reason (exit 1); with the grant, the project's
  agents, and a start is refused; as a member the project's folders list (no way up from the project's own folder) and the start
  works, while a folder outside the project is refused in words for the role; in `agora tui` the picker opens at the project's folder (its projects to choose from when several are granted there) and ⌫ goes no higher, and after the change to owner it opens at the home folder (the hub ends the connection when the role changes, and the next one names the owner; ADR-0106); as a viewer the terminal client's footer lists only the keys that read, `f`, `s` and `x` say the viewer can watch and send nothing, and the window offers no start, new terminal, prompt or Give; Activity (Person column) lists both people's actions in words, each row with "session <id>", and no refused `pair.status`, `UsageReport` or first-folder row from the member's or viewer's windows.
- **Evidence so far:** LAN run 2026-11-11, L16 and L17 (lan-test-findings): the audit page on the Windows web window pass.

### RM-27 Agents message each other across machines (mesh)
- **OS:** Linux, macOS, Windows, in every direction.
- **Do:** S3; a Shell on each machine. With S5 as the first agent call `send_message` to `rm-two/<agent id>`; as the second call
  `inbox`. Open the Messages tab in the window.
- **See:** the message arrives once with `provenance.node`; the window lists it under "Recent messages", marked "between machines"; a message to an offline machine
  fails with a reason.
- **Evidence so far:** LAN run 2026-11-11, L12 (lan-test-findings): six directions over the LAN hub; R3.
  [Linux](2026-11-21-real-machine-linux.md) and [Windows](2026-11-21-real-machine-windows.md) 2026-11-21: delivery and `node.offline` passed, but Messages listed each
  message twice, once with a doubly qualified sender (#905: both machines record the message; the window now folds the two copies).
  [macOS 2026-11-21](2026-11-21-real-machine-macos.md): a message to the stopped `rm-two` got no `delivery_failed` (#915, not
  reproduced in tests; a post the hub never answers now fails with `node.timeout` after 40 s on the sender's machine). Check again:
  the notice `node.offline` (or `node.timeout`) comes back within 40 s, also right after the other machine's Ctrl-C.
  [2026-11-22 follow-ups](2026-11-22-real-machine-followups.md): macOS, a stopped recipient: the sender got one `delivery_failed` with `node.offline` and the original `reply_to`, and no late
  delivery after resuming (#915 closed).

### RM-28 A reply keeps its link across machines
not built: nothing to check.

### RM-29 Shared project board (tasks, leases)
- **OS:** Linux, macOS, Windows.
- **Do:** S3 (`$RM/proj2` is a clone of `$RM/proj`, so the same project is on both machines). With S5: `task_create`; `task_claim` from both agents;
  `lease_acquire {"paths":["README.md"],"ttl_s":60,"reason":"rm"}` from both; stop `rm-two`'s node; create a task; start it again.
- **See:** one claim wins, the other gets `task.not_open`; one lease holder, the other `lease.held` with its end time; the board stays
  on the hub while one machine is down and the returning machine sees every task.
- **Evidence so far:** LAN run 2026-11-11, L12 and B1 to B7 (lan-test-findings). [Windows 2026-11-21](2026-11-21-real-machine-windows.md): pass.

## Updates, secrets and confinement (ADR-0075 to ADR-0084)

### RM-30 The node updates in place without stopping agents
- **OS:** Linux, macOS (step 1 only: the mesh bridge reconnects; the later steps are not built).
- **Do:** start a Shell; open its bridge with S5 and call `workspace_status`; `agora down` (the agent stays with its holder), then
  `agora`; within 15 minutes call `workspace_status` again on the same bridge (unverified: a bridge run by hand, not by an agent).
- **See:** the second call answers without restarting the bridge (during the gap a call may fail with `mesh.unavailable`).
- **Evidence so far:** [2026-11-22 follow-ups](2026-11-22-real-machine-followups.md): macOS, a kept agent read the messages posted after `agora down` and `agora` (#965), each call over a fresh `agora mesh stdio`; the same running bridge reconnecting (this entry) is still unverified. The 2026-11-21 failure (#906) is in [macOS 2026-11-21](2026-11-21-real-machine-macos.md).

### RM-31 A read-only metrics token
- **OS:** Linux, macOS, Windows.
- **Do:** a separate hub: `agora hub run --listen 127.0.0.1:17450 --hub-dir $RM/mhub --metrics`; `agora hub metrics-token create rm
  --hub-dir $RM/mhub`; `curl --cacert $RM/mhub/ca.pem -H "Authorization: Bearer TOKEN" https://localhost:17450/api/metrics`
  (`curl.exe --ssl-no-revoke` on Windows, L4); then the same token on the protected path `https://localhost:17450/api/audit`; then
  `revoke rm`.
- **See:** the metrics in Prometheus text; the same token on a protected path (`/api/audit`) answers 401 (an unknown path such as
  `/api/state` answers 200 with the web app, with or without a token, so it proves nothing); after `revoke` the metrics answer 401.
- **Evidence so far:** [Windows 2026-11-21](2026-11-21-real-machine-windows.md): pass.

### RM-32 The Windows binary is code-signed
- **OS:** Windows.
- **Do:** download `agora-x86_64-pc-windows-msvc.exe` of 0.3.33; in PowerShell `Get-AuthenticodeSignature
  .\agora-x86_64-pc-windows-msvc.exe`.
  Signing a release needs the SignPath account (**human**).
- **See:** today: `NotSigned` (expected until SignPath is set up). After setup: `Valid`, with a timestamp.
- **Evidence so far:** none recorded (lan-test-findings L13: Defender flagged the unsigned 0.3.12). [Windows 2026-11-21](2026-11-21-real-machine-windows.md): `NotSigned`, as expected today.

### RM-33 A confined agent gets a git configuration the node wrote
- **OS:** Linux, macOS; Windows with confinement on.
- **Isolation:** follow the confinement rule in [Safety rules](#safety-rules) for the version under test.
- **Do:** in a disposable OS account or VM, set a made-up `user.name` and `user.email` in that account's global Git configuration,
  then apply RM-23's policy. In the confined Shell, print `GIT_CONFIG_GLOBAL` (`echo "$GIT_CONFIG_GLOBAL"` or, in PowerShell,
  `$env:GIT_CONFIG_GLOBAL`), run `git config --global --list` and make a commit with `git commit --allow-empty -m t`.
- **See:** with ADR-0098, `GIT_CONFIG_GLOBAL` names a file in the run's `$AGORA_HOME/data/git-config/`. The file holds the
  allow-listed values from the test identity (see [Sandbox recipes](../security/sandbox-recipes.md)). The commit carries the
  test identity's name.
- **Evidence so far:** [macOS 2026-11-14](2026-11-14-macos-confinement-and-terminal-client.md) (before ADR-0078: a confined commit
  needed `~/.gitconfig` readable); LAN run L10 (Windows commit under the profile).

### RM-34 A terminal holder keeps agents alive when the node stops
- **OS:** Linux, macOS (unconfined agents started through the holder); Windows: not built.
- **Do:** start a Shell; in it run a counter (`i=0; while :; do i=$((i+1)); echo $i; sleep 1; done`). `agora down`; `agora status`;
  wait 30 s; `agora`.
- **See:** `status` counts the agent while the node is down; after `agora` the same agent (same id, same process) is back with the
  counter still counting, no gap in its numbers, and the window and the terminal client show it running with its terminal, not
  "Finished" (#907); `agora down --agents` would end it.
- **Evidence so far:** `blackview`, 2026-11-20 (ADR-0093 "Origin", cited in `docs/capabilities.md`): a 0.3.26 node restart kept a
  Claude Code terminal agent in the same holder.

### RM-35 The file browser does not show secret files
- **OS:** Linux, macOS, Windows.
- **Do:** in `$RM/proj` make `.env` (`KEY=rm-fake`) and `id_rsa` (any text). Open Files; open Changes; in `agora tui` open the file
  browser (`o`) and the diff (`r`).
- **See:** in the window both are listed as "…, kept private" and opening one says "Kept private: it may hold a secret"; the terminal
  client marks them "private" with the same sentence; Changes says "Changed, content hidden"; their content is never shown.
- **Evidence so far:** [Windows 2026-11-21](2026-11-21-real-machine-windows.md): pass (#940: UTF-16 text shown as not text; #941: `r` on a repository with no commit).

### RM-36 A replayed revocation does not undo the owner's re-trust
- **OS:** Linux, macOS, Windows.
- **Do:** RM-25 up to Remove of browser B; then trust B again on the machine with `agora node trust <device id> <key>`, the command
  B's Settings > This device shows (`agora node trust --from-hub` does not offer a removed device again); restart the node.
  (The replay itself is checked by the node tests; by hand only the order is checked.)
- **See:** after the restart B is still trusted on that machine (Settings > This device in B: "trusts this browser").
- **Evidence so far:** [Linux 2026-11-21](2026-11-21-real-machine-linux.md): pass. [Windows 2026-11-21](2026-11-21-real-machine-windows.md): pass.

### RM-37 A confined agent cannot use the node's control socket
- **OS:** Linux, macOS; Windows with confinement on.
- **Isolation:** follow the confinement rule in [Safety rules](#safety-rules) for the version under test.
- **Do:** RM-23's policy. In a confined Shell: `agora agent list --socket $AGORA_HOME/node.sock` (and `agora agent list`).
- **See:** refused (no list of agents); the same command in your own terminal works.
- **Evidence so far:** [macOS 2026-11-14](2026-11-14-macos-confinement-and-terminal-client.md): the control socket refused under
  `sandbox-exec`.

### RM-38 A confined agent reaches only the unix sockets it is given
- **OS:** macOS (allow-list), Linux (abstract sockets, Landlock ABI 6 or newer; pathname sockets not restricted); Windows: not
  established.
- **Isolation:** follow the confinement rule in [Safety rules](#safety-rules) for the version under test.
- **Do:** RM-23's policy. In a confined Shell: `env | grep -E 'SSH_AUTH_SOCK|DBUS|TMUX|DOCKER'`; on macOS, first make a synthetic
  socket in your own terminal (`nc -lU $RM/rm.sock`), then from the confined Shell `echo hi | nc -U <that path typed out>`
  (unverified); run `agora doctor`.
- **See:** none of those variables; on macOS the outside socket is refused, the agent's own mesh socket works; `doctor` says what this
  machine refuses.
- **Evidence so far:** macOS 27.2.1, 2026-11-17 (ADR-0083 "Built", cited in `docs/capabilities.md`); LAN run L21 (Linux, before the
  fix).

### RM-39 A confined agent gets its login from a secret the node holds
- **OS:** Linux, macOS, Windows.
- **Do:** a manifest `rm-env` with `launch = [FAKE, "--print-env"]` and `[accounts]` `credential_env = ["RM_TOKEN"]` (unverified;
  if the node gives the secret only to confined agents, add RM-23's policy, and then only in a disposable OS account or VM, #900).
  `agora node secret set RM_TOKEN` (type `rm-fake-value`); policy line `credentials.rm-env = RM_TOKEN`; restart the node. Start
  `rm-env` and, for comparison, the same program under another kind name.
- **See:** `rm-env`'s screen shows `ENV RM_TOKEN=rm-fake-value`; the other kind does not get it; `agora node secret list` shows the name
  only; `agora doctor` names what each kind would get.
- **Evidence so far:** [Windows 2026-11-21](2026-11-21-real-machine-windows.md): pass.

## Since ADR-0085

### RM-40 Machine and hub certificates renew themselves
- **OS:** Linux, macOS, Windows.
- **Do:** `agora doctor` on the hub machine and on a joined machine. (A machine's certificate renews itself when a third of its life
  is left, the hub's own when 30 days are left; seeing one needs a machine kept 60 days or more: note the dates and check again then.)
- **See:** `doctor` shows the days left for this machine's certificate and the hub's; after the 60th day, a new end date with no step
  taken.
- **Evidence so far:** [macOS 2026-11-21](2026-11-21-macos-update-0333.md): `agora doctor` on the LAN Mac, "valid for 79 more days.
  It renews itself …"; a renewal itself: none recorded. [Windows 2026-11-21](2026-11-21-real-machine-windows.md): pass for the days left.

### RM-41 Any machine can be the hub, with takeover
not built: nothing to check.

### RM-42 Preview an agent's local web server
not built (schema only): nothing to check.

### RM-43 A structured agent's chat history survives the node, read in pages
not built: nothing to check.

### RM-44 A structured agent's conversation is complete (thinking, tools, plan, effort)
- **OS:** Linux, macOS, Windows (the window; the terminal client's view is not built).
- **Do:** S2 with a second kind `rm-acp2` whose argv is `rm-acp`'s with `--config-options` and `--bypass-mode` (the two flags work
  together). Send `rich`, `plan`, `slow`, two prompts while `slow` runs, then Stop turn. Under the input, Mode > "Turbo (does not
  ask)" (the list is Default, Turbo (does not ask), Plan).
- **See:** "Thought", the tool call with its path, and the Plan with "N of M done"; the waiting prompts show "Waiting" and "Sent when
  this turn ends"; Stop turn ends only the turn and they go in order; after Stop turn the "sleep" call reads "Stopped" (no spinner), the
  composer no longer says "Waiting for this turn to end." once the agent is Idle, and each waiting prompt is shown above its own answer,
  each answer its own message ("queued one", then "queued two", never "queued onequeued two"; #908); the mode switch opens a card at once (Setting: Approvals,
  Switch to: Turbo, "This covers this agent's session only.", Allow / Deny) and Mode shows "Waiting for your approval" ("Answer the
  request to switch. Nothing has changed yet.") until Allow; then the red bolt, whose tooltip names the mode ("Because of: Mode: Turbo").
- **Evidence so far:** [Linux 2026-11-21](2026-11-21-real-machine-linux.md): fail (#908: after Stop turn the tool call keeps spinning and the answers are glued). [Windows 2026-11-21](2026-11-21-real-machine-windows.md): fail (#908).

### RM-45 Plan limits of subscription agents
- **OS:** Linux, macOS, Windows. Codex only.
- **Do:** **human, dedicated account:** start Codex, give it a prompt, open the Usage tab.
- **See:** Plan limits shows "Five hours" and "Seven days" with "N %" and "Resets in …"; the agent shows "Limit N %"; other tools are
  named in "No limit information from this tool: …".
- **Evidence so far:** none recorded.

### RM-46 The ACP registry as a source of agent kinds
- **OS:** none in the release: step 1 is a developer tool (`cargo run -p agora-adapters --example registry_convert`), not part of
  the program. Nothing to check on a machine until a later step ships.
- **Evidence so far:** none recorded.

### RM-47 Structured agents survive a node restart
- **OS:** Linux, macOS (unconfined agents through a holder of link version 2); Windows: not built.
- **Do:** start `rm-acp`; send `hello`; `agora down`; `agora`; send `hello again`.
- **See:** the same agent and session, shown running in the window and the terminal client (not "Finished", #907) with the mode
  and settings it had (#932), not under
  "Stopped when the machine shut down"; the second prompt is answered; the earlier turns are not shown again (the chat history is in
  memory until ADR-0089, #931); a confined `rm-acp` (only in a disposable OS account or VM, #900) instead ends with the node and comes back
  only as the resume policy allows.
- **Evidence so far:** none for the built steps (the 2026-11-20 run on `blackview` in ADR-0093 "Origin" is the case that led to it).

### RM-48 A plain terminal shows the agent program running in it
not built: nothing to check.

### RM-49 A project's kanban board; a task dispatched to an agent
- **OS:** Linux, macOS, Windows (the window and the terminal client).
- **Builds:** part A runs on the release 0.3.33. Part B needs a build newer than 0.3.33 that has #889 ("Start a new agent for this
  task") and #893 (the terminal client's board and give): the first release after 0.3.33, or a release build of `main` from after
  both (write its commit in the note). On 0.3.33, record part B as **skip** ("not in 0.3.33"); a run on a newer build does both.
- **Do, part A (0.3.33):** S2 plus a probe terminal agent `rm-paste`. The probe used on WHITE on 2026-11-21 is not in the repository, so write one that
  does this (unverified, on every system): put its terminal in raw mode (no echo, no line editing, so every byte is seen as it
  arrives); turn bracketed paste on (DECSET 2004, `ESC [ ? 2004 h`); print a working line that the manifest's `working` rule matches
  (for example `* Working (esc to interrupt)`) and keep it as the last line for 20 s; then print an idle line that the `idle` rule
  matches (`> `); from its start to its end, log every byte read from its input with the time since start, to a file in `$RM`. On
  Linux a 25-line Python script did it (`tty.setraw`, `select`, `os.read`, launched as `["/usr/bin/python3", "-I", script, log,
  "paste"]`); its manifest has `rm-screen`'s `working` and `idle` rules. A second probe, `rm-nopaste`, is the same without DECSET 2004.
  The board is in the right rail (the board icon) > "Open board (N)". Starting the probe with `agora agent start` while the board is
  already open makes the 5 s easy. The tasks can also be made with `task_create` on an agent's mesh socket (S5).
  Start `rm-paste` and `rm-acp` in `$RM/proj`. To `rm-acp` send `mcp task_create {"title":"RM task 1"}`, then `RM task 2` and `RM task
  3`. Within 5 s of `rm-paste`'s start: Open board (N) > the "RM task 1" card in Backlog > Give to an agent > `rm-paste` > Give the
  task. Give "RM task 2" to `rm-acp`. Then start `rm-nopaste` and give it a task.
- **Do, part B (newer build):** On "RM task 3": Give to an agent > Start a new agent for this task. In the terminal client (`agora
  tui`), create "RM task 4" the same way, select an agent of the project, press `b`,
  select "RM task 4", press `g`, choose `rm-acp`, hold Enter for a second, then press `g` once; resize the terminal to 80x6 and try
  again on "RM task 5" (unverified: the labels are the terminal client's own text from `tui.ftl`).
- **See, part A:** "Give this task to <agent>?" names Task, Agent, Kind, Machine and Project; the answer is "Sent to <agent>. It waits until
  the agent's current turn ends."; the probe's log has no byte before its idle line, then one bracketed paste (`ESC [ 200 ~` … `ESC
  [ 201 ~`) and an Enter (`\r`) shortly after; the card moves to Doing with "Agent: <agent>"; `rm-nopaste` gets "This agent's terminal
  program cannot take a pasted task. Choose another agent, or type the task into it yourself."
- **See, part B:** Start a new agent for this task opens the start window and then the same confirmation for the new agent; in the
  terminal client the board shows Backlog, Doing and
  Done, held Enter gives nothing, the single `g` gives "RM task 4" (it moves to Doing), and at 80x6 the confirmation says the terminal
  is too small and `g` gives nothing.
- **Evidence so far:** [Windows 11 2026-11-21](2026-11-21-windows-dispatch-conpty.md): through ConPTY, given at about 5.5 s while
  working, no byte before idle at 20.0 s, one 768-byte bracketed paste 1.9 s after idle, Enter about 120 ms later; the refusal path
  and real agents not tested. [Windows 2026-11-21](2026-11-21-real-machine-windows.md): part A pass.

### RM-50 A ready helper agent
not built: nothing to check.

## The window

### RM-51 Review screen and file browser
- **OS:** Linux, macOS, Windows (any browser).
- **Do:** in `$RM/proj` change a file, add one, delete one, rename one. Open Changes: Show (All, Changed, Added, Deleted, Renamed), Layout
  > Side by side, Wrap lines, Next file; open Files: "Filter files by name", "Find in the file", "Copy the whole file"; in the File
  manager (Ctrl K > "Open the File manager beside"), Download.
- **See:** each change is shown with its kind (Changed, Added, Deleted, Renamed, "was <name>" for the rename) and its lines; removed
  lines red with `-`, added green with `+`; Files marks lines "Added since the last commit" and "Changed since the last commit".
- **Evidence so far:** [Windows 2026-11-21](2026-11-21-real-machine-windows.md): pass.

### RM-52 A diff names every changed file even when cut
not built (the window says how many files are not shown): nothing to check.

### RM-53 A shared terminal takes the smallest window's size
- **OS:** Linux, macOS, Windows.
- **Do:** open the same Shell in two browser windows of different sizes; in it run `stty size` (Windows: `$Host.UI.RawUI.WindowSize`);
  close the smaller window; run it again.
- **See:** the size is the smaller window's while both are open, and the remaining window's after the smaller one closes.
- **Evidence so far:** [Windows 2026-11-21](2026-11-21-real-machine-windows.md): pass.

### RM-54 Phone-sized window
- **OS:** a real phone (iOS, Android) against a hub on any system.
- **Do:** **human:** open the window on the phone over the same network (or RM-56); use Start, Machines, "Needs you · N", an
  agent's terminal with its key bar (Esc, Tab, Ctrl, Up, Down, Left, Right, Paste).
- **See:** no page scrolls sideways; targets are easy to hit; typing reaches the agent.
- **Evidence so far:** not run on a real phone.

### RM-55 Screen password for remote sign-in
- **OS:** Linux, macOS, Windows.
- **Do:** `agora password set --hub-dir $RM/home/hub` (a throwaway password; it signs out every open window, the run's too); open
  the hub in a fresh browser profile by an address that is not loopback (the machine's LAN address, which the hub's certificate
  names when S1 answered `y`; this machine's own browser at `localhost` is never asked); Sign in with the code and the Screen
  password, first wrong, then right; Lock; in Settings, Screen password > Change password; then `agora
  password clear --hub-dir $RM/home/hub`.
- **See:** a wrong password is refused ("That did not work. The code may be wrong, expired or already used, or the password may be
  wrong."), the right one signs in; Lock signs this window out while
  the agents keep running; a changed password says "Saved. Every other window was signed out."
- **Evidence so far:** [Linux 2026-11-21](2026-11-21-real-machine-linux.md): fail (#909: the local Settings cannot change a set password). [Windows 2026-11-21](2026-11-21-real-machine-windows.md): pass on one machine (the sign-in prompt needs a second; #946).

### RM-56 Tailscale (rung 3): `agora remote tailscale|status|off`, public-address rule, doctor checks
- **OS:** Linux, macOS, Windows.
- **Do:** **human:** on a machine in a test tailnet, `agora remote tailscale --serve --hub-dir $RM/home/hub`, `agora remote status`,
  `agora doctor`; open the address from another tailnet device; `agora remote off`.
- **See:** the window opens over the tailnet address with the screen password; `status` and `doctor` say what is on; `off` closes it.
- **Evidence so far:** never run against a real tailnet or a real VPS.

### RM-57 Trusted reverse proxy or tunnel (rung 4): `agora remote proxy`, Host/Origin checks, real client address
- **OS:** Linux, macOS, Windows.
- **Do:** a local reverse proxy in front of the run's hub that sets one header itself on every request (for example `X-Real-IP`;
  never `X-Forwarded-For`, as `docs/guides/remote-hub.md` part C says), then `agora remote proxy --trust-header X-Real-IP --host
  <name>` with the run's `--hub-dir`; `agora remote status`. Which proxy, and its settings, are the runner's choice (unverified). A
  real tunnel (Cloudflare Tunnel, part C) is **human**.
- **See:** the window opens through the proxy and asks for the screen password and the code; a request with another `Host` or
  `Origin` answers `421` / `403`; wrong passwords through the proxy lock out only that visitor address.
- **Evidence so far:** never run against a real tunnel.

### RM-58 Layouts and themes
- **OS:** Linux, macOS, Windows (Chrome, Firefox, Safari; Windows at 125 % and 150 %).
- **Do:** Layout > each preset; Split right, Close pane; Save layout as…, reload, Saved layouts… > Use; Export and Import…; Reset
  layout; the colour scheme switch: Dark, Light, Follow the system; Edit theme > Customize a colour.
- **See:** each layout keeps its panes after a reload; themes switch at once with every text readable ("All text and controls are easy
  to see (WCAG AA)."; before #1054 it said "All text is readable (WCAG AA)."); a colour that would be hard to read says "Not applied: some text would be hard to read."
- **Evidence so far:** [Windows 2026-11-21](2026-11-21-real-machine-windows.md): pass (Chrome, 125 % and 150 %).

### RM-59 Terminal client (TUI)
- **OS:** Linux, macOS, Windows (Windows Terminal).
- **Do:** S1, S2; `agora tui` at 80×24 and at a large size. Open the list, an agent's tab (Enter), its files (`o`) and its changes
  (`r`); answer an approval of `rm-chat` (`ask`): `y` in the list, Alt+Y in the agent's tab (there `y` goes into the draft); resize;
  Ctrl-] back to the list; `s` lists every installed kind (on 0.3.33 only the first 9, #910).
- **See:** agents named by kind and colour; "<agent> is waiting for you" and "N waiting" while it asks; the tab redraws after a
  resize; readers say when they wait.
- **Evidence so far:** [macOS 2026-11-14](2026-11-14-macos-confinement-and-terminal-client.md): list, tabs, readers, a real Claude Code
  agent; LAN run L3 (`agora tui` by IP address, fixed). [Windows 2026-11-21](2026-11-21-real-machine-windows.md): pass (a pseudoconsole).

### RM-60 Desktop app
- **OS:** Linux, macOS, Windows. Partly built (P7-06); builds are unsigned CI artifacts (`desktop.yml`: the NSIS installer, the disk
  image).
- **Do:** install the newest `desktop` workflow artifact (it is built on `main` and on pull requests, not on tags: note its commit;
  unverified); connect it to the run's hub (S1 with `y`) from the same machine and from another on
  the LAN; on macOS allow the local network when asked.
- **See:** the app opens the window signed in; it refuses a hub it cannot verify and says so; on macOS it reaches a hub on the LAN.
- **Evidence so far:** LAN run 2026-11-11, L1 and L2 (lan-test-findings): the macOS app on the LAN and the desktop shell's hub check,
  both fixed. [Windows 2026-11-21](2026-11-21-real-machine-windows.md): the app (unpacked from the 0.3.33 installer, not installed:
  a 0.3.3 install was on the machine): the hub check refuses another hub's CA, the page opens and stays remembered.
