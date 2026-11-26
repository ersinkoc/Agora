- On a machine whose policy is `skip_permissions = forbid` (or `confined_only` for an agent that is not confined), a structured agent
  whose own mode or setting lets it act without asking (its default, a saved setting, or a resumed session) is no longer started, and
  one that turns such a mode on later is stopped, with its conversation saying why; no prompt reaches it. Before, only a switch asked
  for from the window was refused (backlog P2-16, ADR-0090).
