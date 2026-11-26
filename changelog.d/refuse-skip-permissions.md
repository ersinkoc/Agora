- On a machine whose policy is `skip_permissions = forbid` (or `confined_only` for an agent that is not confined), a structured agent
  whose own mode or setting lets it act without asking (its default, a saved setting, a resumed session, or a switch it makes itself)
  is now stopped, and its conversation says why. Before, only a switch asked for from the window was refused (backlog P2-16, ADR-0090).
