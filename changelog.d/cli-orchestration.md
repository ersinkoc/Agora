- Agents can be driven from shell scripts: `agora agent prompt ID TEXT --wait [--timeout SECS]` waits until the turn is over,
  `agora agent wait ID --until STATE,...` waits for a state, and `agora events [--agent ID] [--json]` prints each agent change as a
  line; exit codes say idle or done (0), needs you (3), exited (4), timeout (5). A prompt to an agent that is waiting for you is no
  longer sent: it is refused with exit code 6 unless `--force`
