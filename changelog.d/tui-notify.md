- The terminal client (`agora tui`) now tells you when an agent finishes or needs you, with the web window's rules: the terminal's bell
  (on by default; "Turn the bell off" in the command palette), and optionally a desktop notification through the terminal (`OSC 9` or
  `OSC 777`, off by default, set in `tui-settings` in the Agora folder) that names the agent. Never for the agent you are looking at,
  never when the output is not a terminal, at most one every 2 seconds. An agent that finishes while it is not on screen keeps a green
  `✓` in the agent list until you open it (M6.7, G13)
