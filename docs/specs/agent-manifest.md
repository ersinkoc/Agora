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
acp = { argv = ["example-agent", "--acp"] }   # present = L2 available (run through `agora acp-bridge`, see adapter-protocol.md)
# adapter = "example-agent-adapter"           # present = L3 package from the registry

[mcp]
# How the node hands the agent its own mesh MCP server (docs/specs/mesh-tools.md).
method = "config_file"                  # config_file | cli_flag | env
path = ".mcp.json"                      # config_file, cli_flag: inside the agent's worktree
format = "json"                         # only json for now
# flag = ["--mcp-config", "{config_file}"]   # cli_flag: appended to the launch command; `path` is optional (omit it when the
#                                              # flags carry the whole server and no file is needed)
# Placeholders: {worktree} {socket} {mcp_command} {config_file} and the quoted-TOML forms {socket_toml} {mcp_command_toml}
# (for agents that take `-c key=value` overrides, such as Codex).
# [mcp.env]                                  # env: variables set for the agent
# AGORA_MCP_SOCKET = "{socket}"

[hooks]
# Optional hook installation for session identity and state reports.
install = "builtin:example"             # or a script path from the package

[resume]
argv = ["example-agent", "--resume", "{session_id}"]

[usage]
source = "file"                         # file is read by the node today (ADR-0042); otel | adapter | none are reserved
quota_profiles = ["example-provider/pro", "example-provider/max"]

[usage.file]                            # ADR-0042: the file the agent writes its own usage to, one JSON object per line
glob = "{home}/.example/sessions/*/*.jsonl"   # must start with {home}/; `*` matches within one path component
format = "jsonl"                        # only jsonl
select = { path = "type", equals = "assistant" }   # a line must have this to count (optional)
cwd = "cwd"                             # dotted path of the working directory the file records
model = "message.model"                 # the latest value seen on any line is used for later usage lines
input = "message.usage.input_tokens"
output = "message.usage.output_tokens"
# cache_read = "..."; cache_write = "..."   # prompt-side tokens counted into input
# dedupe = "message.id"                 # a usage line whose id was seen before is skipped
mode = "delta"                          # delta: each line adds; total: each line is the session's running total

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
- A `contains` phrase of more than one word also matches when the region wraps it: runs of whitespace, line breaks included, count as one
  space (so "Yes, I trust this folder" is found when a narrow terminal puts "folder" on the next row). Only whitespace is collapsed;
  regexes still see the rows as they are.
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

Manifests in this repository: `claude-code` and `codex` (screens captured from the real agents, `manifests/agents/fixtures/`); `wrongstack` is planned and not shipped yet (it needs a machine that has it). `crates/agora-adapters/tests/shipped_manifests.rs` checks every shipped manifest against its captured screens.

## Usage files (ADR-0042)
The node follows the files of a running terminal agent whose kind has `[usage.file]`: a file belongs to the agent when it was touched after
the agent started and the directory it records (`cwd`) is the agent's worktree (until a line says it, usage lines wait, at most 1000). Limits:
4 files per agent, 1 MiB read per second per agent, lines over 256 KiB skipped, files looked for every 5 seconds. A line that does not
parse or lacks the numbers adds nothing. Usage is reported as `AgentUsageReported`. Shipped for `claude-code` (`delta`, de-duplicated by
`message.id`) and `codex` (`total`).

## `[mcp]` injection

Placeholders in `path`, `flag` and `env` values: `{worktree}`, `{socket}` (this agent's mesh socket), `{mcp_command}` (the
program serving it, `agora`) and `{config_file}` (the file written for `config_file` and `cli_flag`); an unknown placeholder
is an error. The written JSON is the usual `{"mcpServers": {"agora-mesh": {"command", "args"}}}` with
`args = ["mesh", "stdio", "--socket", <socket>]`.

Safety: a manifest is third-party input (threat T6), so files are written **only inside the agent's worktree**: `..` and
absolute paths outside it are refused, no symlink on the path is ever followed (a repository can contain
`.mcp.json -> ~/.ssh/authorized_keys`: a symlinked file is replaced by a real one, a symlinked directory refuses the start), and
the written file is added to the worktree's private `info/exclude` so it stays out of `git status`. Flags and variables are
added at every launch, restarts and resumes included; a resumed agent gets a fresh endpoint. An agent whose manifest has no
`[mcp]` still joins the mesh. A manifest whose injection fails does not start (it asked for something the node refused).

## `[sandbox]` (ADR-0047)

`[sandbox] read = ["~/.config/tool"]`, `write = ["~/.cache/tool"]` lists what the agent needs beyond its worktree when the node
confines agents (`~/` is the home folder; relative paths and `..` are ignored). These are requests: the node adds them only for
a kind its owner listed in `sandbox.trust_manifest` in the node's `policy` file, so a community manifest cannot widen anything by
asking.

## `[adapter]` (L3)

`[adapter] command = ["program", "args"...]` (and optional `env`) makes the kind an adapter agent: the node starts the
process in the agent's own worktree, runs `initialize` and `session/start`, and drives the agent through the adapter protocol
(`docs/specs/adapter-protocol.md`) instead of a terminal. The adapter's `state/update` is a reported state (it wins over
anything else); chat items, usage, approval requests and the native session id are passed on as supervisor events; a crash ends the
agent as `unknown` without touching others. Prompts and verified approval decisions are relayed by the node. Adapter agents have
no terminal, so terminal commands do not apply to them; restart with backoff is not done yet.
