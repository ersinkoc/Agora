- Machines verify the owner's shared MCP servers themselves (ADR-0122): each item must be signed by a device the machine trusts,
  match its content and be newer than what it saw. Claude Code agents then get them at start, as a second MCP file the machine keeps
  outside the repository, readable only by its user, with secrets filled from the machine's own store. A machine can say no with
  `share.items = off`, limit the programs with `share.mcp_commands`, or forbid servers at a URL with `share.mcp_urls = forbid`.
  Publishing them from the window comes next.
