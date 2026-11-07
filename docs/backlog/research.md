# Research backlog (ongoing)

Research tasks produce data files and notes with sources, not code. They can run in parallel with any phase.

### R-01 Usage sources per agent
- **For each agent kind** (claude-code, codex, wrongstack, gemini-cli, cursor, opencode, then others):
  where usage can be read (ACP, adapter, OpenTelemetry, local logs and their formats), what fields exist,
  and how stable they are across versions.
- **Method:** survey open-source projects that already read these sources; read agent docs; verify on a real
  install.
- **Output:** `docs/research/usage-sources.md` with sources per claim; manifest `[usage]` sections.

### R-02 Quota profiles per provider and plan
- **Output:** quota profile files with `sources`, `confidence`, and `last_verified`.
- **Method:** official plan pages, open-source tools that track limits, observed limit events.
- **Rule:** never invent a limit; `limit = 0` (unknown) is acceptable.

### R-03 Model id mapping
- Map each agent's reported model ids to models.dev ids; send missing models upstream to models.dev.

### R-04 Re-verification cadence
- A scheduled check that flags quota profiles older than 60 days and manifests whose agent released a new
  major version.
