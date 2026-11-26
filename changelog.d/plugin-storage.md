- The hub's owner can install a community plugin for everyone on the hub with `agora hub plugin add FILE` (and `list`, `remove ID`), on
  the hub's machine. The hub checks the package exactly as the window does and never runs it. Each person then chooses in the window
  whether to show it and what it may see, and the hub keeps that choice for them; a new version that asks to see more waits until they
  look again, and a plugin the owner removes is gone from every window at its next load (ADR-0114).
