# Real-machine checklist

One entry per row of [`docs/capabilities.md`](../capabilities.md), in the same order and sections (roadmap M1.1). Each entry says
which systems it applies to, what to do on a real machine with the released Agora 0.3.33, what to see, and what evidence is already
written down in this repository. When a row is added to `docs/capabilities.md`, add its entry here in the same pull request.

This is a first draft written from the docs and the code, not from a run. Where a step turns out to be wrong on a real machine, fix
the step in the same pull request as the run's note.

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
  (`$RM`). Before any `agora down`, `agora service`, `agora update` or `kill`, check that `AGORA_HOME` is set to the run's folder and
  that `agora status` shows the run's data folder. Never touch `~/.agora` (`%USERPROFILE%\.agora`) or the machine's own services.
- **Do not stop agents that are running on that machine.** blackview, WHITE and the Macs run the maintainer's agents. Stop only what
  the run started, and only by its id or process id from the run's own `agora status`/`agora agent list`.
- **Never install a login service on a machine that has its own Agora service** (the names `agora-hub`/`agora-node` are shared):
  the `agora service` steps are for a machine without one, or are skipped.
- **Clean up:** `agora down --agents` with the run's `AGORA_HOME`, then delete `$RM`. Release builds of `agora-fake-agent` go with it.

## Which entries a platform session can run alone

- **macOS session:** RM-01, RM-03 to RM-07, RM-08 (a private `sshd` on loopback, as on 2026-11-09), RM-10, RM-11 to RM-16 (the
  fake-agent steps), RM-18, RM-19 to RM-21, RM-23, RM-24, RM-25, RM-27, RM-29, RM-30, RM-31, RM-33 to RM-40, RM-44, RM-47, RM-49,
  RM-51, RM-53, RM-55, RM-58, RM-59, RM-60 (the CI disk image).
- **Windows session:** RM-02, RM-03 to RM-07, RM-10, RM-11 to RM-16 (the fake-agent steps), RM-18, RM-19 to RM-21, RM-23 (confinement
  is off by default: turned on for the run only), RM-24, RM-25, RM-27, RM-29, RM-31, RM-32 (checking the signature only), RM-35,
  RM-36, RM-39, RM-40, RM-44, RM-49, RM-51, RM-53, RM-55, RM-58, RM-59, RM-60 (the CI installer).
- **Needs a person:** the real-agent steps of RM-11 to RM-13, RM-18 with a published list, RM-26 with a second person, RM-32
  (signing account), RM-45, RM-54 (a real phone), RM-56 (a tailnet), RM-57 (a real tunnel).

## Common setup

The commands are for a Unix shell; on Windows use PowerShell (`$env:AGORA_HOME = "$env:TEMP\rm\home"` for `export`). In the window,
the words are the ones it shows (`docs/ux/visual-language.md`): a node is a **Machine**, an approval is **Needs your OK**.

**S0. Run folder.** `RM=$HOME/rm-$(date +%F)`; `mkdir -p $RM/home $RM/proj`; `export AGORA_HOME=$RM/home`; in `$RM/proj`:
`git init -b main && git commit --allow-empty -m init`. Check `agora --version` says 0.3.33.

**S1. Hub and machine here.** With `AGORA_HOME` set, type `agora --ui none`. To the first question answer `1` ("Start Agora here");
to "other machines may connect" answer `N` (or `y` for the entries that need a second machine); to "Start Agora when you log in?"
answer `n`. `agora status` shows the hub's address (the first free port from 7443, so not the machine's own hub) and the data folder.
`agora web` opens the window signed in (accept the hub's own certificate for this run only).

**S2. Fake agents and the Shell.** `agora-fake-agent` is not in the release: build it from the same tag,
`git clone --depth 1 --branch v0.3.33 https://github.com/ersinkoc/Agora $RM/src && cargo build --release -p agora-fake-agent
--manifest-path $RM/src/Cargo.toml`, and use `FAKE=$RM/src/target/release/agora-fake-agent` (`.exe` on Windows). Write these
manifests into `$AGORA_HOME/manifests/` (the node reads the folder again before it lists or starts agents; on Windows write paths as
TOML literal strings, `'C:\...'`):

- `shell.toml`: `kind = "shell"`, `display_name = "Shell"`, `version = "1"`, `min_engine = 1`, `[process]` `launch = ["sh"]`
  (`["powershell.exe", "-NoLogo"]` on Windows).
- `rm-screen.toml` (a terminal agent read from its screen): `launch = [FAKE, "--script", "$RM/screen.toml"]`, with three `[[rules]]`
  on `region = "bottom_non_empty_lines(1)"`: `working` for `contains = "esc to interrupt"`, `blocked` for `contains = "Allow Bash"`,
  `idle` for `regex = '^> '`. `$RM/screen.toml` is `crates/agora-fake-agent/fixtures/working-blocked-idle.toml` with `delay_ms`
  of 20000 on the first two frames and 600000 on the last, so each state lasts long enough to see.
- `rm-acp.toml` (a structured agent over ACP): `[structured]` `acp = { argv = [FAKE, "--acp"] }`. Its prompts are listed at the top
  of `crates/agora-fake-agent/src/acp.rs` (`plan`, `ask`, `rich`, `usage`, `mcp TOOL JSON`, `slow`, anything else is echoed).
- `rm-chat.toml` (a structured agent through an adapter): `[adapter]` `command = [FAKE, "--adapter"]` (prompts `ask`, `cwd`, `crash`).
- `rm-probe.toml`: `launch = [FAKE, "--probe-directory", "$RM/probe-{pid}.json"]`, with the `[[launch.option]]` and
  `[[launch.setting]]` blocks of `crates/agora-node/tests/agent_spawn.rs` (`skip-permissions`, `model`, `effort`). The probe writes
  its folder and its arguments to that file and waits.

`agora agent kinds` lists every kind; one that is missing is named by `agora doctor`.

**S3. A second machine on the same computer** (when no second computer is at hand): `export AGORA_HOME=$RM/two`,
`agora hub code --hub-dir $RM/home/hub` (with the first home), then `agora node enroll --hub wss://localhost:PORT --ca
$RM/home/hub/ca.pem --code CODE --id rm-two` and `agora node run --project $RM/proj2` in its own terminal.

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
  [macOS 2026-11-14](2026-11-14-macos-confinement-and-terminal-client.md): a scratch node with shell, fake and Claude Code agents.

### RM-02 Node on Windows: ConPTY, job objects, named pipe
- **OS:** Windows.
- **Do:** S0, S1, S2 on Windows. Start the Shell (PowerShell) in `$RM\proj`; run `Get-Location`, `"hello" > a.txt`, `git status`; resize
  the window. Start `rm-screen`. Run `agora agent list` (it reaches the node over its named pipe). Then `agora down` and look in Task
  Manager for the agents' processes.
- **See:** the Shell draws correctly through ConPTY and redraws after a resize; `rm-screen` goes Working, Needs you, Idle; `agora agent
  list` answers; after `agora down` no process of the run's agents is left (the job object ends them).
- **Evidence so far:** LAN run 2026-11-11 (lan-test-findings): a Windows 11 node with real Claude Code and Codex; outages on Windows,
  2026-11-14 (lan-test-findings R1 to R5); [supervisor and login service 2026-11-14](2026-11-14-supervisor-and-login-service.md) on
  WHITE; [dispatch through ConPTY 2026-11-21](2026-11-21-windows-dispatch-conpty.md) on WHITE.

### RM-03 Hub with the web window, many machines
- **OS:** Linux, macOS, Windows (the hub on each; machines of any system).
- **Do:** S1 with `y` for other machines; add a second machine (RM-07, or S3). Open the window; start a Shell on each machine.
- **See:** Your machines lists both as Online with their system; both agents are in Your agents, each "on <machine>"; typing in either
  terminal works.
- **Evidence so far:** LAN run 2026-11-11 (lan-test-findings): a Linux hub with Linux, macOS and Windows nodes; the Windows web window
  pass (L14 to L20).

### RM-04 A machine is online while its link is (silence noticed in 25 s)
- **OS:** Linux, macOS, Windows.
- **Do:** S1, S3. Start a Shell on `rm-two`. Freeze `rm-two`'s node process for 60 s (`kill -STOP <pid>` on Linux and macOS; on
  Windows, Resource Monitor > the process > Suspend process), then let it go (`kill -CONT`, Resume process). Note the times.
- **See:** `rm-two` shows Offline with "last seen" within about 25 to 35 s of the freeze; Online again within a few seconds of the
  resume, by itself; its Shell agent is still there and answers.
- **Evidence so far:** lan-test-findings R1 and R5 measured earlier builds (the first version failed on `754a7b1`, the hub restart
  catch-up after ADR-0070 on macOS and Windows); the ADR-0070 silence check itself: none recorded.

### RM-05 Old machines shown apart; forget one, clean up old ones
- **OS:** Linux, macOS, Windows.
- **Do:** S1, S3. Stop `rm-two`'s node. Old means not seen for 24 hours: either leave it 24 h, or only check Offline now. Then Your
  machines > the machine > Forget…; and `agora hub machines` and `agora hub machines forget --old` with `--hub-dir $RM/home/hub`.
- **See:** after 24 h the machine is under Old machines (n), not offered in the start window; Forget… asks first (Keep / confirm), then
  the machine is gone from the window and from `agora hub machines`; a machine that is Online again is not forgotten.
- **Evidence so far:** none recorded.

### RM-06 `agora` on a fresh machine starts a hub and a node here
- **OS:** Linux, macOS, Windows.
- **Do:** S0 (an empty `AGORA_HOME` is a fresh machine to Agora). Type `agora`: answer the first question, `n` to start at login. Then
  `agora status`; kill the node process; wait 40 s; `agora status`; `agora down`. On a machine with **no** Agora service of its own:
  `agora service install`, `status`, `remove`.
- **See:** the first question appears once; the terminal client opens signed in; `status` shows hub and node with versions; the killed
  node is back about 30 s later and `status` says it was started again; `agora down` leaves no Agora process of the run;
  `service status` says "starts when you log in … running now", `remove` leaves nothing.
- **Evidence so far:** [supervisor and login service 2026-11-14](2026-11-14-supervisor-and-login-service.md): restart after a crash
  and start at login on Windows 11, macOS and Linux with scratch homes (0.3.11 to 0.3.12 era); joining a hub by its address: none.

### RM-07 Pairing: find a hub on the network, approve on the hub
- **OS:** Linux, macOS, Windows (either side). IPv4 broadcast only.
- **Do:** on the hub machine S1 with `y`, then `agora pair open` (in a terminal). On a second machine (or a second `AGORA_HOME` on the
  same one) `agora join`, then `agora join HUB-ADDRESS`. Answer on the hub's terminal once, and once in the window (A new machine wants
  to join > Yes, the numbers match > Approve).
- **See:** `join` finds the hub and shows the same fingerprint as `pair open`; both sides show the same number; after `y` the new
  machine says `Joined. This machine is "…"` and is Online in Your machines; a denied or timed-out request ends with "Nothing was
  saved"; the hub closes itself after one approval.
- **Evidence so far:** none recorded.

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
- **Do:** copy the program to `$RM/bin/agora` and use only that copy: `$RM/bin/agora update --check`, then
  `$RM/bin/agora update --version 0.3.32` and `$RM/bin/agora update --version 0.3.33`.
- **See:** `--check` names the newest release; each update checks the checksum and signature before it replaces the copy;
  `$RM/bin/agora --version` follows; the machine's own `agora` is untouched. On Windows, note any Microsoft Defender verdict.
- **Evidence so far:** none recorded (lan-test-findings L13 lists the self-replace among the suspects of a Defender verdict).

## Agents

### RM-11 L1: terminal agents from manifests (built-in: claude-code, codex, wrongstack)
- **OS:** Linux, macOS, Windows.
- **Do:** S2. Start `rm-screen` in `$RM/proj`; watch it 60 s; open "Why is this shown?". Then **human, dedicated account:** start
  Claude Code, Codex and WrongStack each in a disposable folder; answer the folder-trust prompt; give a prompt; wait for the end.
- **See:** `rm-screen` goes Working, then Needs you, then Idle, in the list and on its tile, with "Read from what is on the agent's
  screen."; the real agents show Needs you on the trust prompt (wide and narrow window), Working during the turn, Idle after.
- **Evidence so far:** [phase 4](../backlog/phase-4.md) P4-05 (Claude Code and Codex manifests done, WrongStack open); LAN run
  2026-11-11, L7 and L8 (trust prompts on Windows); [#279](0279-narrow-trust-prompts.md): Claude Code's trust prompt at both sizes on
  a real macOS node; [macOS 2026-11-14](2026-11-14-macos-confinement-and-terminal-client.md): a real Claude Code agent showing Needs
  you.

### RM-12 L2: ACP agents (built-in: opencode-acp, wrongstack-acp, antigravity-acp)
- **OS:** Linux, macOS, Windows.
- **Do:** S2. Start `rm-acp`; send `plan`, then `ask`, then `hello`. Then **human, dedicated account:** start opencode, WrongStack and
  Antigravity (structured) and give each one prompt.
- **See:** the agent opens as a chat (no terminal); `plan` shows a plan and a change; `ask` shows "asks permission" with Allow / Deny,
  and the agent says `allowed` after Allow; `hello` is echoed; the real agents answer, or a failed start shows the agent's own reason.
- **Evidence so far:** none recorded.

### RM-13 L3: adapter processes, installed from the signed registry
- **OS:** Linux, macOS, Windows.
- **Do:** S2. Start `rm-chat`; send `ask`, then `cwd`, then `crash`. Registry: there is no default registry; if one is published, run
  `agora adapter install NAME --registry URL`, `agora adapter list`, `agora adapter remove NAME` (**human, dedicated account** for an
  adapter that needs a login).
- **See:** the chat answers; `ask` brings an approval that Allow resolves; `cwd` says `$RM/proj`; `crash` shows the agent ended with
  an error and its reason; an installed adapter shows its version in `list` and its kind in the start window.
- **Evidence so far:** none recorded.

### RM-14 Agents run in the selected folder; worktrees optional
- **OS:** Linux, macOS, Windows.
- **Do:** Start an agent > Working folder > Browse folders > `$RM/proj` > Use this folder > Shell > Start; run `pwd` and `git branch
  --show-current`. Again with "Create an isolated working copy" checked. Close both.
- **See:** the first runs in `$RM/proj` on `main` ("Shared folder · changes belong to the workspace"); the second in its own folder
  on an `agent/…` branch; after Close, a worktree that was never changed and its branch are removed, a changed one is kept.
- **Evidence so far:** LAN run 2026-11-11, L20 and L22 (lan-test-findings): branch rollback and selected checkouts under confinement.

### RM-15 Launch options, including "do not ask" modes
- **OS:** Linux, macOS, Windows.
- **Do:** S2. Start an agent > `rm-probe` > How it acts > Don't ask > Start; confirm "Start without asking". Read `$RM/probe-*.json`.
- **See:** the confirmation appears before the start; the agent is marked "Runs without asking" ("Acts freely"); the probe's
  arguments end with `--fake-skip-permissions`; without the option they do not.
- **Evidence so far:** none recorded.

### RM-16 Start with model, effort and mode
- **OS:** Linux, macOS, Windows (the window; the terminal client's picker is not built).
- **Do:** S2. Start `rm-probe` with Model `m1` and Effort `high`. On a second machine (S3), start the same there, so the start is
  signed.
- **See:** the start window says "rm-probe will start with …"; the probe's arguments hold `--fake-model m1 --fake-effort high`; the
  agent's settings show them; a signed start's approval card lists them.
- **Evidence so far:** none recorded.

### RM-17 Agent login and accounts on a node
not built: nothing to check.

### RM-18 Install and update coding agents on a machine from the window
- **OS:** Linux, macOS, Windows. Built: `agora doctor`'s agent checks and `agora agent install` (binary download); from the window:
  not built.
- **Do:** `agora doctor` with the run's `AGORA_HOME`. `agora agent install fake-goose --list
  $RM/src/crates/agora-adapters/tests/fixtures/agent-list/fake-list-v1.signed.json --root-key
  8a88e3dd7409f195fd52db2d3cba5d72ca6709bf1d94121bf3748801b40f6f5c` (the FAKE test list and its public test key, from
  `crates/agora-adapters/tests/agent_packages.rs`): type the name; it must stop at the download, since the list's addresses are fake.
  Run it once more with its input from a pipe (`echo fake-goose | agora agent install …`). With a published first-party list (**human, dedicated account**):
  install one agent, type its name to approve, then `--go-back`.
- **See:** `doctor` names each agent kind as installed or not; the install prints the exact plan (version, address, SHA-256,
  publisher, target folder), asks for the name and then fails at the download with a reason in words (or says this system is not
  offered); input that is not a terminal is refused; nothing outside `$AGORA_HOME/agents` changes; the installed agent is "Installed" under Settings > Agents.
- **Evidence so far:** none recorded.

### RM-19 A missing agent kind says why
- **OS:** Linux, macOS, Windows.
- **Do:** add a manifest whose `launch` names a program that does not exist (`["rm-no-such-program"]`). Open Start an agent; open
  Settings > Agents; run `agora agent start` for that kind; open `agora tui` and its start list.
- **See:** the kind is shown as "not installed"; Settings > Agents says "This machine cannot find rm-no-such-program in its PATH."; the
  CLI and the terminal client say why in words, not a code.
- **Evidence so far:** none recorded.

### RM-20 Resume after the node restarts (session ledger)
- **OS:** Linux, macOS, Windows.
- **Do:** start `rm-acp` (and on Windows the Shell too). End the node in a way that ends its agents: on Windows `agora down`; on Linux
  and macOS a confined agent (RM-23's policy) or `kill -9` of the node and its holder. Start again with `agora`; then with `agora
  --resume auto`.
- **See:** the agents are under "Stopped when the machine shut down" with Resume or Start again; Resume brings the conversation back in
  the same folder; with `--resume auto` they come back by themselves (asked once); one still running from before says "Stop it".
- **Evidence so far:** [macOS 2026-11-14](2026-11-14-macos-confinement-and-terminal-client.md): a confined agent resumed after `kill
  -9` with `resume = auto`; [supervisor 2026-11-14](2026-11-14-supervisor-and-login-service.md): on Windows a crashed node marks its
  agents interrupted.

### RM-21 An agent starts helpers, with the user's signed approval
- **OS:** Linux, macOS, Windows.
- **Do:** S2, S4. Start `rm-acp`; send `mcp agent_start {"kind":"rm-screen","prompt":"hello"}`. Answer the card; then the same with
  Deny. Settings > Helper permissions: Revoke.
- **See:** a card says rm-acp "asks to start a rm-screen agent", with the folder and "Its first prompt"; Only this one starts the
  helper, shown "started by rm-acp" under its parent; Deny starts nothing and the agent is told; nothing starts without a card.
- **Evidence so far:** none recorded.

### RM-22 Workspace clean-up and disk limits
not built: nothing to check.

### RM-23 Agent confinement (sandbox)
- **OS:** Linux, macOS; Windows built but off by default (turn on with `AGORA_WINDOWS_CONFINEMENT=1` for the run's node only).
  One level per node; `worktree-offline` on macOS only.
- **Do:** write `sandbox = worktree` into `$AGORA_HOME/data/policy`; restart the node. Start a Shell with "Create an isolated working
  copy"; in it try `cat ~/.ssh/known_hosts`, `ls ~`, `touch ~/rm-out`, `touch $AGORA_HOME/x`, then `git commit --allow-empty -m t`.
- **See:** reading the home, writing outside the worktree and touching the Agora folder are refused; the commit works; `agora doctor`
  says the machine confines agents.
- **Evidence so far:** [macOS 2026-11-14](2026-11-14-macos-confinement-and-terminal-client.md): every way out refused; Claude Code and
  Codex confined; LAN run L10, L11, L21, L22, L23 (Linux and Windows findings).

## People, approvals and coordination

### RM-24 Approvals signed by a device the node trusts
- **OS:** Linux, macOS, Windows.
- **Do:** S3 without S4 for `rm-two`: start an agent on it from the window; then S4 for `rm-two` and start again. Start `rm-chat`
  there, send `ask`, answer Allow.
- **See:** before trust, the start is refused and the window says this browser is not trusted there; after trust, the start and the
  Allow work; the answer is in Activity.
- **Evidence so far:** none recorded.

### RM-25 Trusting your devices in one step
- **OS:** Linux, macOS, Windows.
- **Do:** S1 with browser A trusted (S4). Sign in from browser B (another profile). In B: Settings > This device > Ask another device to
  approve. In A: the request > Approve. Then in A: Your other devices > B > Remove.
- **See:** B's fingerprint in A matches B's own; after Approve, B "trusts this browser" on the machines in scope and can start an agent
  with no command typed; after Remove, B cannot.
- **Evidence so far:** none recorded.

### RM-26 Team mode: people, roles, audit
- **OS:** Linux, macOS, Windows (the hub). Partly built (P7-09).
- **Do:** `agora hub user add rm-viewer --role viewer --hub-dir $RM/home/hub`; sign in with its code in browser B (a second person:
  **human**; otherwise a second browser profile). As the viewer try to start an agent. As the owner open Activity.
- **See:** the viewer sees the agents but cannot start one; Activity lists both people's actions with a session id each.
- **Evidence so far:** LAN run 2026-11-11, L16 and L17 (lan-test-findings): the audit page on the Windows web window pass.

### RM-27 Agents message each other across machines (mesh)
- **OS:** Linux, macOS, Windows, in every direction.
- **Do:** S3; a Shell on each machine. With S5 as the first agent call `send_message` to `rm-two/<agent id>`; as the second call
  `inbox`. Open Messages in the window.
- **See:** the message arrives once with `provenance.node`; the window shows it "between machines"; a message to an offline machine
  fails with a reason.
- **Evidence so far:** LAN run 2026-11-11, L12 (lan-test-findings): six directions over the LAN hub; R3.

### RM-28 A reply keeps its link across machines
not built: nothing to check.

### RM-29 Shared project board (tasks, leases)
- **OS:** Linux, macOS, Windows.
- **Do:** S3 with the same project on both machines (a clone of `$RM/proj`). With S5: `task_create`; `task_claim` from both agents;
  `lease_acquire {"paths":["README.md"],"ttl_s":60,"reason":"rm"}` from both; stop `rm-two`'s node; create a task; start it again.
- **See:** one claim wins, the other gets `task.not_open`; one lease holder, the other `lease.held` with its end time; the board stays
  on the hub while one machine is down and the returning machine sees every task.
- **Evidence so far:** LAN run 2026-11-11, L12 and B1 to B7 (lan-test-findings).

## Updates, secrets and confinement (ADR-0075 to ADR-0084)

### RM-30 The node updates in place without stopping agents
- **OS:** Linux, macOS (step 1 only: the mesh bridge reconnects; the later steps are not built).
- **Do:** start a Shell; open its bridge with S5 and call `workspace_status`; `agora down` (the agent stays with its holder), then
  `agora`; within 15 minutes call `workspace_status` again on the same bridge.
- **See:** the second call answers without restarting the bridge (during the gap a call may fail with `mesh.unavailable`).
- **Evidence so far:** none recorded.

### RM-31 A read-only metrics token
- **OS:** Linux, macOS, Windows.
- **Do:** a separate hub: `agora hub run --listen 127.0.0.1:17450 --hub-dir $RM/mhub --metrics`; `agora hub metrics-token create rm
  --hub-dir $RM/mhub`; `curl --cacert $RM/mhub/ca.pem -H "Authorization: Bearer TOKEN" https://localhost:17450/api/metrics`
  (`curl.exe --ssl-no-revoke` on Windows, L4); then the same token on another path; then `revoke rm`.
- **See:** the metrics in Prometheus text; another path answers 401; after `revoke` the metrics answer 401.
- **Evidence so far:** none recorded.

### RM-32 The Windows binary is code-signed
- **OS:** Windows.
- **Do:** download `agora-x86_64-pc-windows-msvc` of 0.3.33; in PowerShell `Get-AuthenticodeSignature .\agora.exe`.
  Signing a release needs the SignPath account (**human**).
- **See:** today: not signed (expected until SignPath is set up). After setup: `Valid`, with a timestamp.
- **Evidence so far:** none recorded (lan-test-findings L13: Defender flagged the unsigned 0.3.12).

### RM-33 A confined agent gets a git configuration the node wrote
- **OS:** Linux, macOS; Windows with confinement on.
- **Do:** RM-23's policy; set `user.name` in your global git config. In a confined Shell: `echo $GIT_CONFIG_GLOBAL`, `git config
  --global --list`, `git commit --allow-empty -m t`.
- **See:** `GIT_CONFIG_GLOBAL` names the node's file; it holds only the copied keys (no `credential.*`, `url.*`, `include`); the commit
  works with your name.
- **Evidence so far:** [macOS 2026-11-14](2026-11-14-macos-confinement-and-terminal-client.md) (before ADR-0078: a confined commit
  needed `~/.gitconfig` readable); LAN run L10 (Windows commit under the profile).

### RM-34 A terminal holder keeps agents alive when the node stops
- **OS:** Linux, macOS (unconfined agents started through the holder); Windows: not built.
- **Do:** start a Shell; in it run a counter (`i=0; while :; do i=$((i+1)); echo $i; sleep 1; done`). `agora down`; `agora status`;
  wait 30 s; `agora`.
- **See:** `status` counts the agent while the node is down; after `agora` the same agent (same id, same process) is back with the
  counter still counting, no gap in its numbers; `agora down --agents` would end it.
- **Evidence so far:** `blackview`, 2026-11-20 (ADR-0093 "Origin", cited in `docs/capabilities.md`): a 0.3.26 node restart kept a
  Claude Code terminal agent in the same holder.

### RM-35 The file browser does not show secret files
- **OS:** Linux, macOS, Windows.
- **Do:** in `$RM/proj` make `.env` (`KEY=rm-fake`) and `id_rsa` (any text). Open Files; open Changes; in `agora tui` open the file
  browser (`o`) and the diff (`r`).
- **See:** both are listed "kept private" and their content is never shown, in the window and in the terminal client; Changes says
  "Changed, content hidden".
- **Evidence so far:** none recorded.

### RM-36 A replayed revocation does not undo the owner's re-trust
- **OS:** Linux, macOS, Windows.
- **Do:** RM-25 up to Remove of browser B; then on the machine `agora node trust --from-hub` and trust B again; restart the node.
  (The replay itself is checked by the node tests; by hand only the order is checked.)
- **See:** after the restart B is still trusted on that machine (Settings > This device in B: "trusts this browser").
- **Evidence so far:** none recorded.

### RM-37 A confined agent cannot use the node's control socket
- **OS:** Linux, macOS; Windows with confinement on.
- **Do:** RM-23's policy. In a confined Shell: `agora agent list --socket $AGORA_HOME/node.sock` (and `agora agent list`).
- **See:** refused (no list of agents); the same command in your own terminal works.
- **Evidence so far:** [macOS 2026-11-14](2026-11-14-macos-confinement-and-terminal-client.md): the control socket refused under
  `sandbox-exec`.

### RM-38 A confined agent reaches only the unix sockets it is given
- **OS:** macOS (allow-list), Linux (abstract sockets, Landlock ABI 6 or newer; pathname sockets not restricted); Windows: not
  established.
- **Do:** RM-23's policy. In a confined Shell: `env | grep -E 'SSH_AUTH_SOCK|DBUS|TMUX|DOCKER'`; on macOS connect to a socket of your
  own outside the worktree (`nc -U`); run `agora doctor`.
- **See:** none of those variables; on macOS the outside socket is refused, the agent's own mesh socket works; `doctor` says what this
  machine refuses.
- **Evidence so far:** macOS 27.2.1, 2026-11-17 (ADR-0083 "Built", cited in `docs/capabilities.md`); LAN run L21 (Linux, before the
  fix).

### RM-39 A confined agent gets its login from a secret the node holds
- **OS:** Linux, macOS, Windows.
- **Do:** a manifest `rm-env` with `launch = [FAKE, "--print-env"]` and `[accounts]` `credential_env = ["RM_TOKEN"]`.
  `agora node secret set RM_TOKEN` (type `rm-fake-value`); policy line `credentials.rm-env = RM_TOKEN`; restart the node. Start
  `rm-env` and, for comparison, the same program under another kind name.
- **See:** `rm-env`'s screen shows `ENV RM_TOKEN=rm-fake-value`; the other kind does not get it; `agora node secret list` shows the name
  only; `agora doctor` names what each kind would get.
- **Evidence so far:** none recorded.

## Since ADR-0085

### RM-40 Machine and hub certificates renew themselves
- **OS:** Linux, macOS, Windows.
- **Do:** `agora doctor` on the hub machine and on a joined machine. (A renewal happens under 30 days left; seeing one needs a machine
  kept 60 days or more: note the dates and check again then.)
- **See:** `doctor` shows the days left for this machine's certificate and the hub's; after the 60th day, a new end date with no step
  taken.
- **Evidence so far:** none recorded.

### RM-41 Any machine can be the hub, with takeover
not built: nothing to check.

### RM-42 Preview an agent's local web server
not built (schema only): nothing to check.

### RM-43 A structured agent's chat history survives the node, read in pages
not built: nothing to check.

### RM-44 A structured agent's conversation is complete (thinking, tools, plan, effort)
- **OS:** Linux, macOS, Windows (the window; the terminal client's view is not built).
- **Do:** S2 with `rm-acp`'s argv extended with `--config-options` and `--bypass-mode`. Send `rich`, `plan`, `slow` (then Stop turn),
  and two prompts while one is running. Switch to a mode that does not ask.
- **See:** thinking, the tool call with its path and output, and the plan with each step's status; Stop turn ends only the turn; the
  queued prompts are sent in order when the turn ends; the switch asks "Let the agent act without asking?" and waits for a signed
  approval.
- **Evidence so far:** none recorded.

### RM-45 Plan limits of subscription agents
- **OS:** Linux, macOS, Windows. Codex only.
- **Do:** **human, dedicated account:** start Codex, give it a prompt, open Usage.
- **See:** Plan limits shows the five-hour and seven-day windows with a percentage and a reset time; other tools are listed as having
  no limit information.
- **Evidence so far:** none recorded.

### RM-46 The ACP registry as a source of agent kinds
- **OS:** none in the release: step 1 is a developer tool (`cargo run -p agora-adapters --example registry_convert`), not part of
  the program. Nothing to check on a machine until a later step ships.
- **Evidence so far:** none recorded.

### RM-47 Structured agents survive a node restart
- **OS:** Linux, macOS (unconfined agents through a holder of link version 2); Windows: not built.
- **Do:** start `rm-acp`; send `hello`; `agora down`; `agora`; send `hello again`.
- **See:** the same agent and conversation, not under "Stopped when the machine shut down"; the second prompt is answered; a confined
  `rm-acp` instead ends with the node and comes back only as the resume policy allows.
- **Evidence so far:** none for the built steps (the 2026-11-20 run on `blackview` in ADR-0093 "Origin" is the case that led to it).

### RM-48 A plain terminal shows the agent program running in it
not built: nothing to check.

### RM-49 A project's kanban board; a task dispatched to an agent
- **OS:** Linux, macOS, Windows (the window; the terminal client's dispatch is not built).
- **Do:** S2 plus a manifest `rm-paste` (Linux and macOS: `launch = ["sh", "-c", "printf '\\033[?2004h* Working (esc to interrupt)\\n';
  sleep 20; printf '> '; exec cat -v"]`, rules as `rm-screen`; Windows: the same in PowerShell). Start `rm-paste` and `rm-acp` in
  `$RM/proj`; with S5 call `task_create` twice. Open board > a Backlog card > Give to an agent > `rm-paste` > Give the task, within 5 s
  of its start. Give the second task to `rm-acp`. Then "Start a new agent for this task" on a third task. Then the same with a manifest
  that never turns bracketed paste on.
- **See:** the confirmation names task, agent, kind, machine and project; the answer is "It waits until the agent's current turn ends";
  nothing reaches `rm-paste` before it is idle, then one bracketed paste (`^[[200~` … `^[[201~` in its `cat -v`) and Enter; the card
  moves to Doing with the agent; the new-agent path opens the start window and then the same confirmation; without bracketed paste the
  answer is "This agent's terminal program cannot take a pasted task…".
- **Evidence so far:** [Windows 11 2026-11-21](2026-11-21-windows-dispatch-conpty.md): through ConPTY, given at about 5.5 s while
  working, no byte before idle at 20.0 s, one 768-byte bracketed paste 1.9 s after idle, Enter about 120 ms later; the refusal path
  and real agents not tested.

### RM-50 A ready helper agent
not built: nothing to check.

## The window

### RM-51 Review screen and file browser
- **OS:** Linux, macOS, Windows (any browser).
- **Do:** in `$RM/proj` change a file, add one, delete one, rename one. Open Changes: filters, Side by side, Wrap lines, Next file; open
  Files: filter, Find in the file, Copy the whole file, Download.
- **See:** each change is shown with its kind (Changed, Added, Deleted, Renamed) and its lines; removed lines red with `-`, added green
  with `+`; Files marks the changed lines.
- **Evidence so far:** none recorded.

### RM-52 A diff names every changed file even when cut
not built (the window says how many files are not shown): nothing to check.

### RM-53 A shared terminal takes the smallest window's size
- **OS:** Linux, macOS, Windows.
- **Do:** open the same Shell in two browser windows of different sizes; in it run `stty size` (Windows: `$Host.UI.RawUI.WindowSize`);
  close the smaller window; run it again.
- **See:** the size is the smaller window's while both are open, and the remaining window's after the smaller one closes.
- **Evidence so far:** none recorded.

### RM-54 Phone-sized window
- **OS:** a real phone (iOS, Android) against a hub on any system.
- **Do:** **human:** open the window on the phone over the same network (or RM-56); use Start, Machines, Needs you · n, an agent's
  terminal with the Terminal keys (Esc, Tab, Ctrl, arrows, Paste).
- **See:** no page scrolls sideways; targets are easy to hit; typing reaches the agent.
- **Evidence so far:** not run on a real phone.

### RM-55 Screen password for remote sign-in
- **OS:** Linux, macOS, Windows.
- **Do:** `agora password set --hub-dir $RM/home/hub` (a throwaway password); open the hub in a fresh browser profile; sign in with
  Screen password, wrong and then right; Lock; `agora password clear`.
- **See:** a wrong password is refused ("That password is not right."), the right one signs in; Lock signs out with agents still
  running; setting a new password signs every other window out.
- **Evidence so far:** none recorded.

### RM-56 Tailscale (rung 3): `agora remote tailscale|status|off`, public-address rule, doctor checks
- **OS:** Linux, macOS, Windows.
- **Do:** **human:** on a machine in a test tailnet, `agora remote tailscale --serve --hub-dir $RM/home/hub`, `agora remote status`,
  `agora doctor`; open the address from another tailnet device; `agora remote off`.
- **See:** the window opens over the tailnet address with the screen password; `status` and `doctor` say what is on; `off` closes it.
- **Evidence so far:** never run against a real tailnet or a real VPS.

### RM-57 Trusted reverse proxy or tunnel (rung 4): `agora remote proxy`, Host/Origin checks, real client address
- **OS:** Linux, macOS, Windows.
- **Do:** a local reverse proxy in front of the run's hub (any proxy the session can run on loopback), then `agora remote proxy
  --trust-header X-Forwarded-For --host <name>` (see `docs/guides/remote-hub.md` part C). A real tunnel (Cloudflare Tunnel) is
  **human**.
- **See:** the window opens through the proxy; a request with another Host answers 421/403; Activity shows the real client address.
- **Evidence so far:** never run against a real tunnel.

### RM-58 Layouts and themes
- **OS:** Linux, macOS, Windows (Chrome, Firefox, Safari; Windows at 125 % and 150 %).
- **Do:** Layout > each preset; Split right, Close pane; Save layout as…, reload, Saved layouts…; Export and Import; Colour scheme >
  Dark, Light, Follow the system; Edit theme > Customize a colour.
- **See:** each layout keeps its panes after a reload; themes switch at once with every text readable; a colour that would be hard to
  read is "Not applied".
- **Evidence so far:** none recorded.

### RM-59 Terminal client (TUI)
- **OS:** Linux, macOS, Windows (Windows Terminal).
- **Do:** S1, S2; `agora tui` at 80×24 and at a large size. Open the list, an agent's tab, the file browser (`o`) and the diff (`r`);
  answer an approval of `rm-chat` (`ask`); resize.
- **See:** agents named by kind and colour; Needs you marked with the number waiting; the tab redraws after a resize; readers say when
  they wait.
- **Evidence so far:** [macOS 2026-11-14](2026-11-14-macos-confinement-and-terminal-client.md): list, tabs, readers, a real Claude Code
  agent; LAN run L3 (`agora tui` by IP address, fixed).

### RM-60 Desktop app
- **OS:** Linux, macOS, Windows. Partly built (P7-06); builds are unsigned CI artifacts (`desktop.yml`: the NSIS installer, the disk
  image).
- **Do:** install the CI build of the 0.3.33 tag; connect it to the run's hub (S1 with `y`) from the same machine and from another on
  the LAN; on macOS allow the local network when asked.
- **See:** the app opens the window signed in; it refuses a hub it cannot verify and says so; on macOS it reaches a hub on the LAN.
- **Evidence so far:** LAN run 2026-11-11, L1 and L2 (lan-test-findings): the macOS app on the LAN and the desktop shell's hub check,
  both fixed.
