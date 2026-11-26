# Get started

Three steps on one machine (Linux, macOS or Windows). The details are in ADR-0062.

1. **Get the `agora` binary** (a release, see the README, or `cargo build --release -p agora`) and put it on your `PATH`.
2. **Type `agora`** in a terminal, in the folder of a project if you like (a git checkout there is offered to your agents). The
   first time it asks one question:
   ```
   1) Start Agora here: your agents run on this machine and you open them from here
   2) Just this machine, alone
   3) Join an Agora that runs on another machine
   ```
   `1` asks once more whether other machines of yours may connect (`y` makes the hub listen on your network, with this machine's
   name and addresses in its certificate; the default keeps it on this machine only). `2` keeps it on this machine. `3` asks for the hub's address and joins it
   (the next section). The answer is kept in
   `~/.agora/machine.toml` (readable by you only, never a secret).
3. **Use it.** `agora` starts the hub and this machine's node in the background (if they are not running), connects the node to
   the hub, and opens the terminal client signed in; no code to copy. `agora web` opens the browser instead, signed in (the browser
   warns about the hub's own certificate until you trust it); over SSH, on Linux without a display, in CI or with `AGORA_NO_BROWSER`
   set it prints the address to open instead. On a machine that joined a hub elsewhere, `agora web` is `agora panel`: it opens the hub's
   window through this machine's node and runs until Ctrl+C. `agora --ui web|tui|none` opens that once; add `--save` to make it what plain `agora` opens from now on.

## Open it from your phone, or run it on a server

`agora remote tailscale --serve` (after `agora password set`) gives the hub a Tailscale address for your phone, and a server with
no screen runs it with `agora --ui none`. The steps, for a VPS with Tailscale or with a plain public address, are in
[docs/guides/remote-hub.md](guides/remote-hub.md).

## Add a second machine

On the machine that runs the hub, open it for new machines, then on the new machine type the hub's address and compare one number:

1. **On the hub's machine:** `agora pair open`. It says "Your hub is open for new machines for 10 minutes" and what to type next, and stays on your screen to
   answer. What it tells you to type is an address the hub listens on and its certificate names (and the others it also answers at); the certificate's names
   (such as this computer's name) are listed as names that may also reach it, depending on your network. A hub that listens on this computer only (`127.0.0.1`,
   when you answered no to "Let other machines of yours connect?") says that another machine cannot reach it and how to change that: listen on this computer's
   address on your network, for example `agora --listen 192.168.1.20:7443`, then `agora down` and `agora`, then `agora pair open` again. `0.0.0.0:7443` instead
   listens on every IPv4 interface, a public one too if the computer has one. Until then it offers only a join from this same computer. Run it in a terminal: it reads your answer from the keyboard only, never from a pipe or a script. `--minutes N` (1 to 10) opens it for less.
2. **On the new machine:** `agora join`. It listens for about three seconds for a hub that is open for new machines on your network and, when it finds one,
   asks you, at a terminal, `Found the hub "desk" at 192.168.1.20:7443. Its fingerprint is 566A D067 41A8 B995 CD07 C464. Compare it with what `agora pair open` shows on the hub`: answer `y` only
   if it is exactly the same as the fingerprint the hub's screen shows (`agora pair open` and `agora pair status` print it). With more than one hub it lists them and asks which one matches; without a
   terminal it stops and asks for the address. If it finds none, give the address yourself: `agora join HUB-ADDRESS`, for example
   `agora join desk.lan:7443` (the hub's name or address on your network, and its port; the port defaults to 7443). Some networks (guest Wi-Fi, some VPNs) do not
   carry the search; the address always works. What is found is only an address to try: nothing is trusted because a hub answered. The machine then joins only a hub whose certificate
   authority has the fingerprint you confirmed. That comparison is what protects you from a machine on your network that pretends to be your hub: the number in the next step
   cannot, because such a machine would simply approve itself. Choice 3 of `agora`'s first
   question asks for the address. The new machine prints one line, for example `Number 482-193. Approve this machine on the hub...`, and waits.
3. **Back on the hub's machine** the screen shows `New machine "laptop" (linux, 192.168.1.50) wants to join. Number 482-193.` Approve only if the two numbers are
   the same: look at both, then type `y` and Enter (a key pressed less than a second after the question is ignored). Anything else denies it. (If the hub already has a machine of that name it asks whether to replace it.)
4. The new machine says `Joined. This machine is "laptop"` with the fingerprint of the hub's certificate authority (the one `agora hub fingerprint` shows),
   and keeps its certificate in `~/.agora/link`, as `agora node enroll` does; its private key is made on that machine and never sent. Start it with `agora`.
   After one machine is approved the hub closes itself to new machines again; run `agora pair open` for the next one.

`agora pair status` says whether the hub is open and how many machines wait; `agora pair close` closes it. Nothing is stored on the new machine unless every
check passed: a request nobody approved, a hub that is closed, one that only offers TLS 1.2, or a hub whose certificate does not name the address you typed all
end with "Nothing was saved". The hub puts its own addresses in its certificate (its `--name` values, `localhost`, its host name and the addresses of its network
cards) when it starts and again each time you run `agora pair open`, so a new address (a DHCP change) is picked up without a restart. If it cannot (the system will
not list the addresses), `agora pair open` says so and stays closed: start the hub with `--name ADDRESS` for the address the new machine will use. A hub that listens
on `127.0.0.1` only (the default) names only that. The way with a code and a CA file (`agora hub code`, `agora node enroll`) still works.

Closing or killing the window changes nothing: the hub, the node and your agents keep running. Next time, `agora` only opens the
window again.

- `agora status`: what runs (process, version, address, data folder, connected hub, agents running, log files), and whether the
  hub or the node crashed and was started again.
- `agora down`: stops the node, then the hub. On Linux and macOS the agents keep running (a small keeper process, `agora hold`, owns them, ADR-0079); `agora` brings them back into the window, and `agora status` counts them while the node is down. `agora down --agents` stops them too. An agent the node confines (a sandbox policy), and every agent on Windows, runs inside the node and ends with it.
- If the hub or the node crashes, it is started again 30 seconds later, at most 5 times in 10 minutes; one that stops right
  after starting (its port is taken, its settings are wrong) is not retried, and its log says why. Each is run by a small
  `agora supervise` process that `agora` starts; stopping or killing it stops what it runs too, so nothing is left behind.
- The first run ends with "Start Agora when you log in? [Y/n]" where that is available (Windows, macOS, and Linux with systemd); Enter says yes.
- `agora service install`: start this machine's Agora when you log in (for you only, no administrator rights; it says what it
  registers first). `agora service status` says whether it is set up and running; `agora service remove` stops it and removes it.
  Windows registers a logon task per role (`agora-hub`, `agora-node`). It runs with the environment Windows gives you at login (your
  user and system `PATH`), not the `PATH` of the window that ran `install`: a program put on `PATH` only in that window is not found
  by the agents it starts. `install` on a task turned off in Task Scheduler turns it on again and says so. A task of the same name
  that you made yourself (or another tool did) is left exactly as it is: `agora service` neither replaces, starts nor removes it.
  **Windows Defender** may flag `agora.exe` shortly after `agora service install` (seen with 0.3.12; a guess from what the program
  does, not a known-malware match). It can quarantine the file, end the node and remove the login task, so the machine goes offline
  and `agora` stops working. First make sure the file is the release you meant to install (`agora update` checks its checksum and
  signature; a file you copied yourself can be checked with the `.sha256` and `.sig` published beside it). Then it is your choice in
  Windows Security > Virus & threat protection > Protection history: restore and allow that item on this device, or leave it and send
  the file to Microsoft as a false positive. After a restore, run `agora service install` again. Linux writes a systemd user unit per role into
  `~/.config/systemd/user/` (or `$XDG_CONFIG_HOME/systemd/user/`), owner-only, and runs `systemctl --user daemon-reload` and
  `enable`; it never uses root. A user unit runs while you are logged in: to start it at boot without logging in, run
  `loginctl enable-linger` yourself (Agora never does). It needs a session with your user's systemd running (a desktop login, or an SSH
  login while your user manager runs, for example with lingering on; not `su` or `sudo`, and it refuses to run as root); without
  one it says so and changes nothing. It also refuses when systemd would read other files for the same unit (drop-ins such as
  `agora-node.service.d/`), `status` and a repeated `install` name any drop-in added later, and
  `remove` names the drop-ins it leaves. `install` on a unit turned off with `systemctl --user disable` turns it on again and says so. The agents the node starts run inside the unit and end with it
  (`systemctl --user stop`, or a logout without lingering), unlike a node started by `agora`, whose agents outlive the window. A
  unit of the same name that you wrote yourself is left exactly
  as it is: `agora service` neither replaces, stops nor removes it. Without `systemctl` it is not available.
  macOS writes a LaunchAgent per role into `~/Library/LaunchAgents/` (owner-only) and loads it with `launchctl`; it never uses root
  (it refuses `sudo`) and needs your login session (over SSH, be logged in at the screen too). The job gets only the full-path
  entries of your shell's `PATH`. macOS may say "Background Items Added" and lists it under System Settings > General > Login Items &
  Extensions ("Allow in the Background"). If you switch it off there, `status` says it is off and `install` says only you can turn it
  on again there; one turned off with `launchctl disable` is turned on again by `install`. A LaunchAgent of the same name that you
  wrote yourself is left exactly as it is. A node that connects to a hub on another machine needs macOS's Local Network permission:
  started by launchd it does not get Terminal's, so allow `agora` when macOS asks, or in System Settings > Privacy & Security > Local
  Network; until then its log says `No route to host` and `agora status` says the node is not connected (and why), not "connected". macOS asks
  again after every `agora update` (the new program is another binary to it; measured with unsigned builds), and `agora update` says so
  when a login service is set up; after you allow it, restart the node once more (the running process keeps failing until then). Removing it ends the supervisor and the node, but not your agents: each runs in its own
  terminal session, which launchd does not end (as when the node is started by `agora`). On every system a service already set up is
  not registered again by `install` (after moving Agora or for another `PATH`: run `agora service remove`, then `install`). On Linux,
  `install` does rewrite a unit Agora wrote when a newer Agora would write it differently (such as the node's `KillMode=process`, which
  lets agents outlive a service restart): it reloads systemd, never restarts the service, and says to run `systemctl --user restart`. On Linux, stopping or restarting the node's
  service leaves the agents running (the keeper stays; systemd's "left-over process" note for it is expected).
- Logs: `~/.agora/logs/hub.log` and `node.log` (renamed to `.1` when over 5 MiB at a start).

Everything works as before by hand too (`agora hub run`, `agora node enroll`, `agora node run`, `agora tui`); `AGORA_HOME` moves
`~/.agora` elsewhere. A machine whose `~/.agora` already has a hub folder or a link made by hand is not taken over yet: `agora`
says so and changes nothing (use a new `AGORA_HOME`, or keep starting it by hand).

## Script agents from a shell

The node's commands also work from scripts on the same machine (the control socket, `--socket PATH` to choose one):

```sh
agora agent prompt agent-1 "fix the failing test" --wait --timeout 600   # sends, then waits for the turn to end
agora agent wait agent-1 --until blocked,idle --timeout 60                # waits for one of these states
agora events --json --agent agent-1                                       # one JSON object per line, until Ctrl-C
```

- `agora agent prompt AGENT_ID TEXT --wait [--timeout SECS]` sends the prompt and waits until the agent leaves Working. An agent that
  waits for you (blocked) is not sent the prompt: the command says `agent.blocked` and exits with 6; `--force` sends it anyway (for
  example to type the answer to a terminal agent's question). The check is the node's agent list just before the prompt goes out, not
  one step with it.
- `agora agent wait AGENT_ID [--until STATE[,STATE...]] [--timeout SECS]` returns as soon as the agent is in one of the states
  (`working`, `blocked`, `idle`, `done`; `exited` waits only for its end). Without `--until`: `idle,done,blocked`.
- Both print `AGENT_ID<TAB>STATE` on stdout (`exited` when its process ended) and say why on stderr when it is not a plain end.
  Exit codes: **0** idle or done (or the `--until` state, working included), **3** blocked (it needs you), **4** exited (stopped,
  crashed, restarted, interrupted or no longer listed; this ends every wait), **5** `--timeout` ran out, **6** prompt refused because
  the agent is blocked, **1** an error (no node, unknown agent, a refusal from the node), **2** a usage error.
- `agora events [--agent AGENT_ID] [--json]` prints a line for each agent when it starts (`present`), then one per change: `added`,
  `state` (with `previous`), `exited`, `running`, `restarted`, `interrupted`, `removed`. With `--json` each line is an object with
  `event`, `agent_id`, `kind`, `state`, `running`, `restarts`, `time_ms` and, for `state`, `previous`; the words are fixed tokens. It runs
  until interrupted, exits with 1 when the node goes away, and with 0 at its next line after the reader did (`| head -1`).

`agora manual scripting` prints this for a coding agent, with the other topics an agent needs to customize Agora for you (`agora
manual` lists them).

The control socket has no event stream, so these commands ask for the node's agent list every 200 ms: a state the agent held for
less than that may not be seen, and `events` lines have no sequence numbers. After a prompt, an agent still showing the state it had
before (its turn too quick to see, or not shown yet) counts as finished after 5 s; a prompt sent while the agent was working (queued, or
typed into its running turn) counts as finished once the agent stayed out of Working for 5 s. An agent whose state Agora cannot
tell (unknown) is waited for until it shows one, so give such waits a `--timeout`.

## Clean up workspaces

An agent started with its own worktree ("Separate worktree") leaves that folder behind when it ends. The node removes it a day later
when nothing would be lost (no uncommitted change, no commit that is not on another branch or pushed; the branch is kept), and keeps
the rest. `agora workspaces` lists them with their state (`in-use`, `waiting`, `cleanable`, `left-behind` and why) and size;
`agora workspaces clean --dry-run` shows what would go, and `agora workspaces clean [--older-than 7d]` removes it now. Folders you
selected yourself are never removed. `workspaces.clean = off` in the node policy stops the automatic removal.

## Certificates

Every certificate Agora issues lasts 90 days. A machine renews its own by itself when 30 days are left, while it is connected to a hub that is
up to date, and the hub replaces its own certificate by itself under 30 days left, so there is nothing to do. `agora doctor` shows the days left for
this machine's certificate and for the hub's, and says when a renewal needs you: if it says this machine must join again (the hub no longer accepts
its certificate, or it ended while the machine was off), run `agora hub code` on the hub and `agora node enroll` here with the new code; its agents
and waiting events are untouched. Update the hub before the machines.

## Upgrading the hub, and going back

The hub's database changes shape between some versions, and only forward: a hub binary that finds a database newer than it understands refuses it
cleanly and starts nothing (it never changes the file). The release that adds certificate renewal (ADR-0085) moves the hub database to schema 13, and the one that adds the community theme
catalog (ADR-0113) to schema 15.
Before you upgrade, copy the hub folder (`~/.agora/hub`, or `AGORA_HOME/hub`) or run `agora hub backup`; to go back to an older version, stop the
hub, put that copy back (`agora hub restore`) and start the older binary. Machines keep running through it: they buffer their events and
reconnect.
