- A community theme catalog, off until the hub's owner turns it on: `agora hub themes update --url URL` fetches a catalog's index and
  its themes on the hub (never in the browser), keeps only files with the right SHA-256 that pass the theme rules, and reports every
  one it refused; `agora hub themes on|off|status` switches it. The theme editor then shows a Catalog section to everyone signed in:
  preview a theme in its own colours, install a copy into your themes, and see "Version N available" on one you installed, which
  changes only when you choose Update. The project's catalog repository does not exist yet, so the owner gives its address
  (ADR-0113)
