# Agent manifest (L1) format

One file per agent kind: `manifests/agents/<kind>.toml`. Manifests are data; the engine in
`agora-adapters` interprets them. Detection rules must be evidence-based: captured from real screens and
covered by fixtures under `manifests/agents/fixtures/<kind>/`.

```toml
kind = "example-agent"          # stable id, lowercase, used in protocol and UI
display_name = "Example Agent"  # brand name, not translated
version = "2026.11.06.1"        # manifest version
min_engine = 1

[process]
names = ["example-agent", "exa"]        # foreground process names
launch = ["example-agent"]              # default argv
env = { EXAMPLE_NO_UPDATE_CHECK = "1" }

[accounts]
config_dir_env = "EXAMPLE_HOME"         # lets the node isolate multiple accounts
login = ["example-agent", "login"]      # run in a terminal streamed to the user
status = ["example-agent", "whoami"]    # exit 0 = authenticated

[structured]
acp = { argv = ["example-agent", "--acp"] }   # present = L2 available
# adapter = "example-agent-adapter"           # present = L3 package from the registry

[mcp]
# How the node injects the mesh MCP server.
method = "config_file"                  # config_file | cli_flag | env
path = "{config_dir}/mcp.json"
format = "json"

[hooks]
# Optional hook installation for session identity and state reports.
install = "builtin:example"             # or a script path from the package

[resume]
argv = ["example-agent", "--resume", "{session_id}"]

[usage]
source = "otel"                         # otel | file | adapter | none
# file_glob = "{config_dir}/usage/*.jsonl"
quota_profiles = ["example-provider/pro", "example-provider/max"]

[usage.model_map]
# agent-reported model id → models.dev provider/model id (ADR-0021)
"example-large" = "example-provider/example-large-2026"

# Screen rules: fallback state detection from the live bottom of the buffer.
[[rules]]
id = "permission_prompt"
state = "blocked"
priority = 900
region = "bottom_lines(8)"
all = [{ contains = "Allow" }, { regex = '\[y/n\]' }]

[[rules]]
id = "spinner"
state = "working"
priority = 800
region = "title"
regex = '^[⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏] '
```

Engine details (`agora-adapters`):
- A rule matches when its own `contains`/`regex` (if present) match, every `all` condition matches, at least one
  `any` condition matches (if `any` is non-empty), and no `not` condition matches. A rule needs at least one
  positive condition; a rule with only `not` is rejected.
- `bottom_lines(n)` is the literal bottom `n` rows of the screen grid (agents that draw at the bottom fill it);
  `bottom_non_empty_lines(n)` is the last `n` rows that contain text. Neither ever includes scrollback.
- Regexes match per line (`^`/`$` are line anchors), use a linear-time engine (no catastrophic backtracking),
  and are size-limited.
- Ties in priority are broken by file order. No matching rule means "no opinion", not `idle`.
- Sections the engine does not interpret yet are ignored, so newer manifests still load; unknown keys inside
  `[[rules]]` are an error. `min_engine` above the engine's version is rejected.
- Detection interval is adaptive (`Pacer`): faster while visible, slower when hidden, backing off while the state
  is stable. Hidden terminals are still checked.

Rule semantics: highest-priority matching rule wins; `all` = AND, `any` = OR, `not` = exclusion; regions are
`title`, `bottom_lines(n)`, `bottom_non_empty_lines(n)`. Reported state (hooks, ACP, adapter) always overrides
rule results.

Manifests in this repository (Phase 2): `claude-code`, `codex`, `wrongstack`.
