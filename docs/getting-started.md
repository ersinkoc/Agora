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
   warns about the hub's own certificate until you trust it). `agora --ui web|tui|none` chooses what plain `agora` opens from now on.

Closing or killing the window changes nothing: the hub, the node and your agents keep running. Next time, `agora` only opens the
window again.

- `agora status`: what runs (process, version, address, data folder, connected hub, agents running, log files).
- `agora down`: stops the node, then the hub (agents end with the node, as when it is stopped any other way).
- Logs: `~/.agora/logs/hub.log` and `node.log` (renamed to `.1` when over 5 MiB at a start).

Everything works as before by hand too (`agora hub run`, `agora node enroll`, `agora node run`, `agora tui`); `AGORA_HOME` moves
`~/.agora` elsewhere. A machine whose `~/.agora` already has a hub folder or a link made by hand is not taken over yet: `agora`
says so and changes nothing (use a new `AGORA_HOME`, or keep starting it by hand).
