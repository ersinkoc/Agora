# Scripting: drive agents from a shell

Agora runs a **node** on each machine. The node starts **agents** (coding tools) in folders and keeps them running. The commands
here talk to the node on this machine through its control socket: `node.sock` in the Agora folder (`$AGORA_HOME`, else `~/.agora`).
`--socket PATH` picks another socket.

Only the person's own user can use the control socket, from their own shell. A confined agent cannot open it. An agent that runs
inside Agora talks to other agents through the mesh tools instead: see `agora manual mesh`.

## What is there

```sh
agora status
agora agent kinds
agora agent list
```

- `agora status` says whether the hub and the node on this machine are running.
- `agora agent kinds` prints one line per kind this node can start: `KIND<TAB>NAME<TAB>FOUND`, where FOUND is `installed` or
  `missing`.
- `agora agent list` prints one line per agent: `AGENT_ID<TAB>KIND<TAB>STATE<TAB>RUN`, where RUN is `running` or `stopped`. After
  the node restarted, an agent it found interrupted has `interrupted` in the fourth column and a fifth: `still-running`, `resume` or
  `start-again`.
- A state is one of these words: `working`, `blocked` (it waits for the person), `idle`, `done`, `unknown`.

## Start, prompt, wait, stop

```sh
agora agent start codex --cwd /home/me/src/app
agora agent start claude-code --cwd /home/me/src/app --worktree --setting effort=high
agora agent prompt agent-1 "fix the failing test" --wait --timeout 600
agora agent wait agent-1 --until blocked,idle --timeout 60
agora agent stop agent-1
agora agent resume agent-1
agora agent resume agent-1 --again
```

- `agora agent start KIND` starts an agent in `--cwd` (default: the current folder). `--worktree` gives it a new git worktree of
  that folder. `--setting ID=VALUE` sets a launch setting the kind declares (for example `model` or `effort`); the node refuses one
  the kind does not declare. It prints a sentence with the new agent id; `agora agent list` shows the id as data.
- `agora agent prompt AGENT_ID TEXT` sends the text as the agent's next prompt. With `--wait` it waits until the agent leaves
  `working`. An agent in `blocked` is not sent the prompt: the command exits with 6. `--force` sends it anyway (for example to type
  the answer to a terminal agent's question).
- `agora agent wait AGENT_ID` returns as soon as the agent is in one of the `--until` states (`working`, `blocked`, `idle`, `done`;
  `exited` waits only for its end). Without `--until` it waits for `idle,done,blocked`.
- Both print `AGENT_ID<TAB>STATE` on stdout (`exited` when its process ended). They say why on stderr when it is not a plain end.
- `agora agent resume AGENT_ID` continues the agent's last conversation in the same folder. `--again` starts a new conversation
  there instead.

Exit codes of `prompt --wait` and `wait`:

| Code | Meaning |
| --- | --- |
| 0 | idle or done, or the `--until` state was reached (`working` included) |
| 1 | an error: no node, an unknown agent, a refusal from the node |
| 2 | a usage error |
| 3 | blocked: the agent needs the person |
| 4 | exited: stopped, crashed, restarted, interrupted or no longer listed |
| 5 | `--timeout` ran out |
| 6 | the prompt was not sent, because the agent is blocked |

## Watch every change

```sh
agora events --json --agent agent-1
agora events --json
```

`agora events` prints a line for each agent when it starts (`present`), then one per change: `added`, `state`, `exited`,
`running`, `restarted`, `interrupted`, `removed`. With `--json` each line is one object with `event`, `agent_id`, `kind`, `state`,
`running`, `restarts`, `time_ms` and, for `state`, `previous`. It runs until it is stopped, exits with 1 when the node goes away, and
with 0 when its reader went away (`| head -1`).

## What to know

- These commands ask the node for its agent list every 200 ms. A state that lasted less than that may not be seen, and `events`
  lines have no sequence numbers.
- After a prompt, an agent that still shows its earlier state counts as finished after 5 s. An agent whose state Agora cannot tell
  (`unknown`) is waited for until it shows one, so give such a wait a `--timeout`.
- Ids, states and the columns above are fixed words. Sentences follow the person's language (`AGORA_LANG`, then `LC_ALL`,
  `LC_MESSAGES`, `LANG`); `AGORA_LANG=en` gives English.
- Answering an agent's request (Allow, Deny) is never a command: the person answers it on a device they trust.

## Other commands

```sh
agora doctor
agora agent explain claude-code --file screen.txt --size 120x40
agora manual files
```

- `agora doctor` checks this machine: terminals, git, the data folder, the node, the agent kinds, the policy file and the
  certificates.
- `agora agent explain KIND --file SCREEN` replays a captured terminal screen and says which rule of the kind's manifest decides its
  state, and why each other rule did not. It needs no node.
- `agora --help` lists the commands.

## Example

Give an agent a task, and act on how it ended:

```sh
agora agent prompt agent-1 "run the tests and fix what fails" --wait --timeout 1800
case $? in
  0) echo "done" ;;
  3) echo "it needs you" ;;
  5) echo "still working after 30 minutes" ;;
  *) echo "see the message above" ;;
esac
```
