- The terminal client's conversation with a structured agent works like the web window's: `/stop` ends the agent's turn (`/stop all`
  also drops the requests waiting behind it) while the agent keeps running, `/effort high` and `/set ID VALUE` change the agent's own
  settings while it runs, tool calls show their status and how long they took, a plan shows its steps, and the agent's thinking is one
  hidden line until `/thinking` shows it. "Start an agent in a folder" (f) asks for the model and effort the agent's kind offers. The
  console reads a keybindings file (`~/.agora/keybindings.json`, the file the web window exports) and uses its keys, saying what it
  leaves out (ADR-0090, ADR-0066, ADR-0068)
