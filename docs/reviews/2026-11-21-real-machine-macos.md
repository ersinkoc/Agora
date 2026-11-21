# Real-machine checklist run: macOS, 2026-11-21

Run of [the real-machine checklist](real-machine-checklist.md) (M1.1) by the `macos` session.

| | |
|---|---|
| Agora | `Agora 0.3.33` (release binary, sha256 `d668f503…b2c2fe`) |
| Machine | `Mac`: Apple M4, macOS 27.2.1 |
| Fake agent | `agora-fake-agent` built from tag `v0.3.33` (S2) |
| Browser | none |

**No browser in this run.** The session had no browser tools, so every step done in the window is **not checked**. Each entry below
says which part ran: the CLI, the mesh (S5) or the node's files. The window parts are left for a run with a person at the browser.

**Setup.**
- S0 and S1 were followed: `$RM` under the home and `agora --ui none --resume ask`. The questions were answered by their text, and the login-service question with `n`.
- The run's hub listened on `127.0.0.1:7443`, apart from the Mac's own Agora (`~/.agora-lan`, which was not touched).
- S3 gave the second machine `rm-two` on the same Mac.
- Clean-up: `agora down --agents` with the run's `AGORA_HOME`, then `rm -rf -- "$RM"`. No process of the run was left, and the Mac's own node stayed on 0.3.33 and connected.

**Issues opened:** #906 (mesh lost after a node restart; my #912 closed as its duplicate), #903 (`agent start` of a missing program says `internal`; my #914 closed as its duplicate), #915 (a message to an offline machine is silently lost).

## Results

- **RM-01 skip (window).** Shell starts, typing and the Files/Changes tabs are window steps. RM-14 and RM-34 cover a Shell started and driven from the CLI. Earlier evidence: [macOS 2026-11-21](2026-11-21-macos-update-0333.md), where `data/`, `m/` and `link/` became `drwx------`.
- **RM-03 skip (window).** `agora hub machines` listed `mac · online` and `rm-two · online`.
- **RM-04 pass (CLI), timing differs.**
  - `kill -STOP` of `rm-two`'s node at 21:15:36 UTC: `agora hub machines` showed `rm-two · offline, last seen 2026-11-21 21:15 UTC` at **+19 s**. The entry says 25 to 35 s.
  - `kill -CONT` at +60 s: online again within 1 s, by itself.
  - Its Shell agent was still running, and its mesh answered.
  - The window's Offline / "last seen" was not checked.
- **RM-05 partial pass (CLI).**
  - Without a terminal: `Nothing was forgotten: this terminal cannot answer. Run it in a terminal, or add --yes.`
  - `forget mac --yes` while online: `mac was not forgotten (node.online)`.
  - `forget rm-two --yes` while offline: `rm-two is forgotten.`, and it was gone from the list.
  - `forget --old`: `No machine is old: every machine was seen in the last 24 hours.`
  - The 24-hour folded section and the window dialogs were not checked.
- **RM-06 partial pass.** Fresh `AGORA_HOME`, `agora --ui none` (no tty for `--ui tui` here).
  - "Bring agents back by themselves after a restart? [Y/n]" was asked once. To `n` it said `Agents will not come back by themselves after a restart; the window offers to bring each one back. `agora --resume auto --save` turns it on again.`
  - `status` showed hub and node 0.3.33.
  - A node killed about 40 s after its start was **not** started again: `The node ended right after starting (signal 9, less than a minute ago), so it was not started again.`
  - Killed after 70 s, it was back about 30 s later: `restarted once in the last 10 minutes; the last crash: signal 9, less than a minute ago`. The step is fixed to wait over a minute.
  - `agora down` left no process.
  - `agora service install` was skipped: this Mac has its own `agora-node` service.
- **RM-07 skip.** Not run in this session: it needs the hub listening on the LAN.
- **RM-08 skip.** Not run in this session: it needs a private loopback `sshd`.
- **RM-10 pass.** On a copy at `$RM/bin/agora`, with the repository's release key:
  - `update --check` → `Agora 0.3.33 is the newest release.`
  - `--version 0.3.32` → `Updated to Agora 0.3.32. …`, then back with `--version 0.3.33`. `--version` followed each time.
  - A wrong key → `The download's signature is not valid for the pinned release key. Nothing was changed.`
  - The machine's own `agora` was unchanged.
- **RM-11 pass (CLI), with the manifest fixed.** `agora agent list` showed `rm-screen` Working at +1 s, Needs you at +23 s and Idle at +41 s.
  - With S2's `rm-screen` as written, the manifest did not load: `[note] The agent description rm-screen.toml could not be read`. Each `[[rules]]` needs `id` and `priority`.
  - With those added, it stayed Working for good, for two reasons:
    - the `blocked` rule on `bottom_non_empty_lines(1)` sees only `  2. No`;
    - `'^> '` never matches, because the trailing space is trimmed from the line.
  - Fixed in S2.
  - "Why is this shown?" was not checked (window).
- **RM-12 partial (CLI).** `rm-acp` loads and starts idle. `agora agent prompt` `plan` and `hello` returned it to idle. `ask` left it **blocked**, waiting for an approval. The chat text and Allow/Deny were not checked (window).
- **RM-13 partial, inconclusive.** `rm-chat` loads and starts, with capabilities chat, approvals, usage, cancel and resume. After `prompt cwd` then `prompt crash`, the adapter process started with the agent still ran 15 s later, and no restart was seen. Without the window I cannot tell whether the prompts were queued. Not counted as a failure.
- **RM-14 partial; step fixed.**
  - A shared Shell: `pwd` = `$RM/proj`, branch `main`.
  - A Shell with `--worktree`: `pwd` = `$RM/home/worktrees/shell-<id>`, branch `agent/shell-<id>`.
  - After `agora agent stop`, **both** the unchanged and the changed worktree, and their branches, were kept.
  - The node removes a worktree only on a failed start (`StartWorktree::rollback`). Removal after close is RM-22, which is not built, so the step now says both are kept.
- **RM-15 skip (window).** The CLI has no flag for a launch option.
- **RM-16 partial (CLI).** `agora agent start rm-probe --setting model=m1 --setting effort=high`: the probe's arguments end in `--fake-model m1 --fake-effort high`, and `cwd` is `$RM/proj`. The start window's text and the helper card were not checked.
- **RM-18 partial pass.** `agora doctor` names each kind with its program found, or `cannot find … in its PATH`. The install from a signed list is a human step and was not run.
- **RM-19 fail (#903).**
  - `agora agent kinds` → `missing`.
  - `doctor` says why in words.
  - But `agora agent start rm-missing` → `Error internal: Something went wrong. Reference: unassigned.`, and `node.log` has `agent start failed: "No such file or directory (os error 2)"`.
  - Settings > Agents and the terminal client were not checked.
- **RM-20 partial pass (CLI).** `kill -9` of the run's node and holder: the bridge ended, and `status` said `crashed (signal 9) … (restart 1 of 5)`.
  - The node was back after about 30 s. With `resume = ask`, `rm-acp` stayed interrupted (the ledger keeps its session) and was not listed.
  - `agora down`, then `agora --ui none --resume auto`: the agent came back by itself with a new bridge, and a prompt was answered.
  - `agora --resume auto` while Agora already runs does nothing and says nothing (`machine.toml` keeps `resume = "ask"`). The step now says to stop first.
  - The window's "Stopped when the machine shut down", Resume and Start again were not checked.
- **RM-21 skip (window).**
- **RM-23 skip (#900).** No disposable OS account on this Mac.
- **RM-24 skip (window).**
- **RM-25 skip (window).**
- **RM-27 pass for delivery, fail for the offline case (#915).**
  - `mac/agent-12` → `rm-two/agent-1` (S5): the message arrived once, with `provenance.node = "mac"`. The other direction also worked.
  - To an unknown machine: the sender's inbox got `delivery_failed` / `node.unknown`.
  - To `rm-two` while it was offline: a `message_id` and nothing more. There was no `delivery_failed`, and the message was never delivered.
  - The Messages tab was not checked.
- **RM-29 pass (S5).**
  - `task_claim` of `t-1`: one side ok, the other `task.not_open`.
  - `lease_acquire README.md`: one lease, the other `lease.held` with `holder_agent_id` and `expires_at`.
  - A task created while `rm-two` was stopped was seen with the first one after it returned.
- **RM-30 fail (#906).** A hand-run bridge on a Shell's mesh socket stayed alive across `agora down` / `agora`, but no call got an answer afterwards, not even `mesh.unavailable`. A new `agora mesh stdio` said `No node is listening at …/m/agent-15.sock`. An agent kept by the holder loses its mesh at every node restart.
- **RM-31 pass; step fixed.**
  - A separate hub on `127.0.0.1:17450 --metrics`: the token read `/api/metrics` (200, Prometheus text) and got 401 on `/api/audit`.
  - After `revoke`, `/api/metrics` answered 401.
  - An *unknown* path (`/api/state`, `/ws`) answers 200 with the web app, with or without a token, so the step now names `/api/audit`.
- **RM-33 skip (#900).**
- **RM-34 pass.** A Shell wrote a counter to a file.
  - `agora down` said `1 agent keeps running. …`. `status` said `agents still running without the node: 1`.
  - 35 s later, `agora`: the same agent was back, with the same pid under the same holder (its parent was launchd meanwhile).
  - The counter ran from 1 to 45 with no gap.
- **RM-35 skip (window; terminal client not run).**
- **RM-36 skip (window).**
- **RM-37 skip (#900).**
- **RM-38 skip (#900).**
- **RM-39 pass (unconfined).** `agora node secret set RM_TOKEN` (from stdin) and the policy line `credentials.rm-env = RM_TOKEN`:
  - `rm-env`'s screen shows `ENV RM_TOKEN=rm-fake-value`;
  - the same program as `rm-env2` does not get it;
  - `secret list` shows the name only;
  - `doctor` says `rm-env receives RM_TOKEN …`, and for `rm-env2` that there is no `credentials.rm-env2` line.
- **RM-40 pass (today's part, hub machine).**
  - `[ok] This machine's hub certificate is valid for 89 more days. It renews itself …`
  - `[ok] The hub's own certificate is valid for 89 more days. The hub replaces it by itself when 30 days are left, without a restart.`
  - The renewal itself needs 60 days. The LAN Mac's joined node shows 79 days ([macOS 2026-11-21](2026-11-21-macos-update-0333.md)).
- **RM-44 skip (window).**
- **RM-47 pass (CLI).** `rm-acp` with `hello`, then `agora down` (`1 agent keeps running`), then `agora`: the same agent, with the same `acp-bridge` pid under the same holder. `hello again` was answered. It showed `unknown` right after the reattach until the next turn. The window was not checked.
- **RM-49 skip (window).**
- **RM-51 skip (window).**
- **RM-53 skip (window).**
- **RM-55 skip.** Not run in this session (the window part needs a browser).
- **RM-58 skip (window).**
- **RM-59 skip.** Not run in this session (it needs a tty).
- **RM-60 skip.** Not run in this session.
