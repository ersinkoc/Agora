- A shared MCP server whose environment needs a secret is now given to agents on machines that allow it: list the secret in the
  machine's policy (`share.secrets = DOCS_TOKEN`) and store it there (`agora node secret set DOCS_TOKEN`). The machine fills it in only
  when the server starts, through its own launcher, and never writes it to a file (ADR-0122 Amendment A).
