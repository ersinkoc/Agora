- `agora node trust --from-hub` run where nobody can answer (a script, Claude Code's `!`) now prints the ready command that trusts
  exactly the devices it listed, to run after comparing their fingerprints. The web window no longer logs a Content-Security-Policy
  error at every load (a probe for WebAssembly in a bundled library is gone; the page policy is unchanged).
