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
   name and addresses in its certificate; the default keeps it on this machine only). `2` keeps it on this machine. `3` is not
   built yet: it points to `agora hub code` and `agora node enroll` (see `agora --help`). The answer is kept in
   `~/.agora/machine.toml` (readable by you only, never a secret).
3. **Use it.** `agora` starts the hub and this machine's node in the background (if they are not running), connects the node to
   the hub, and opens the terminal client signed in; no code to copy. `agora web` opens the browser instead, signed in (the browser
   warns about the hub's own certificate until you trust it). `agora --ui web|tui|none` opens that once; add `--save` to make it what plain `agora` opens from now on.

Closing or killing the window changes nothing: the hub, the node and your agents keep running. Next time, `agora` only opens the
window again.

- `agora status`: what runs (process, version, address, data folder, connected hub, agents running, log files), and whether the
  hub or the node crashed and was started again.
- `agora down`: stops the node, then the hub (agents end with the node, as when it is stopped any other way).
- If the hub or the node crashes, it is started again 30 seconds later, at most 5 times in 10 minutes; one that stops right
  after starting (its port is taken, its settings are wrong) is not retried, and its log says why. Each is run by a small
  `agora supervise` process that `agora` starts; stopping or killing it stops what it runs too, so nothing is left behind.
- The first run ends with "Start Agora when you log in? [Y/n]" where that is available (Windows, macOS, and Linux with systemd); Enter says yes.
- `agora service install`: start this machine's Agora when you log in (for you only, no administrator rights; it says what it
  registers first). `agora service status` says whether it is set up and running; `agora service remove` stops it and removes it.
  Windows registers a logon task per role (`agora-hub`, `agora-node`). It runs with the environment Windows gives you at login (your
  user and system `PATH`), not the `PATH` of the window that ran `install`: a program put on `PATH` only in that window is not found
  by the agents it starts. `install` on a task turned off in Task Scheduler turns it on again and says so. A task of the same name
  that you made yourself (or another tool did) is left exactly as it is: `agora service` neither replaces, starts nor removes it. Linux writes a systemd user unit per role into
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
  (it refuses `sudo`). macOS may say "Background Items Added" and lists it under Login Items; `install` on one turned off there turns it
  on again and says so. A LaunchAgent of the same name that you wrote yourself is left exactly as it is. When it is removed, launchd
  ends what the node started with it.
- Logs: `~/.agora/logs/hub.log` and `node.log` (renamed to `.1` when over 5 MiB at a start).

Everything works as before by hand too (`agora hub run`, `agora node enroll`, `agora node run`, `agora tui`); `AGORA_HOME` moves
`~/.agora` elsewhere. A machine whose `~/.agora` already has a hub folder or a link made by hand is not taken over yet: `agora`
says so and changes nothing (use a new `AGORA_HOME`, or keep starting it by hand).
