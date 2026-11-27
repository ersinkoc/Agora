- Agents can stay reachable while idle: the new mesh tool `inbox_wait` returns as soon as another agent's message arrives (or an
  empty list after its timeout), for an agent asked to stay reachable (say so in its first prompt). Nothing is typed into a terminal, and typing into the agent or
  prompting it from the window ends the wait at once so it reads what you wrote.
