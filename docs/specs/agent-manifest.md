# Agent manifest (L1) format

One file per agent kind: `manifests/agents/<kind>.toml`. Manifests are data; the engine in
`agora-adapters` interprets them. Detection rules must be evidence-based: captured from real screens and
covered by fixtures under `manifests/agents/fixtures/<kind>/`.
Fixtures are replayed at 120x40 unless their filename specifies the capture size:
`<name>@<cols>x<rows>.screen`, with a matching `<name>@<cols>x<rows>.expect`.
Keep the raw captured bytes and use the original terminal size so wrapping and clipping are tested.

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

[session]                               # ADR-0060: how the node learns the agent's own session id (one of the two ways)
id_args = ["--session-id", "{session_id}"]   # the node chooses the id (a v4 UUID) and adds these to the first launch
# [session.file]                        # or: read it from the first line of the session file the agent writes
# glob = "{home}/.example/sessions/*.jsonl"
# select = { path = "type", equals = "session_meta" }
# id = "payload.id"
# cwd = "payload.cwd"

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

`[[ready]]` (ADR-0057, optional): rules of the same shape with `state = "idle"` that match only an empty input box waiting for a
request, stricter than an `idle` rule (whose prompt marker can also be the selected row of a menu or dialog). The node types a helper's
first prompt only when a fresh look at the live screen matches them and the kind's rules detect idle; a kind without them never gets
a first prompt typed for it (the prompt is left for the user). Every shipped terminal manifest has them, and the fixture test checks
that they match the captured idle and answered screens and nothing else (including a `menu.screen` that an `idle` rule matches).
`[[typed]]` (same shape, `state = "idle"`) match an input box that holds typed text with no dialog or menu on screen, with the
footer anchored to the lines right under the box (not anywhere on screen); the node refuses a prompt that contains a text these rules
require (`contains` in the rule, `all` or `any`), and the top-level
`input_box` is a multi-line regex over `bottom_non_empty_lines(12)` whose first capture group is the box's text: before pressing
Enter on a helper's first prompt the node needs both, with the box holding exactly that prompt. The `typed.screen` fixtures are
constructed from the captured idle screens with text in the box (a capture of that moment is not kept); the test checks that
`[[typed]]` matches them and nothing else and that `input_box` reads exactly the typed text.

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
`[mcp]` is for terminal agents: a structured agent (`[adapter]` or `[structured] acp`) needs none, the node passes its mesh server in
`session/start` (`adapter-protocol.md`).

## `[resume]` and `[session]` (ADR-0060)

`[resume] argv` is how a terminal agent continues its own session: `{session_id}` is replaced by the session id the node holds for
the agent. The node uses it after a crash (restart), and after its own restart when someone chooses Resume (`agent.resume`) or the
node's policy resumes by itself. The node holds a session id only when it learned one:

- **`id_args`** (Claude Code, `--session-id <uuid>`, checked on 2.1.319): the node chooses a random v4 UUID at the agent's first launch,
  adds `id_args` with `{session_id}` replaced, and records the id (`AgentSessionReported`) right after the start. Restarts and
  resumes use `[resume]` instead and never add `id_args`.
- **`[session.file]`** (Codex, which has no flag for it: `~/.codex/sessions/YYYY/MM/DD/rollout-<time>-<id>.jsonl`, whose first line is
  `{"type":"session_meta","payload":{"id":...,"cwd":...}}`, checked on codex-cli 0.167.0 and 0.167.1): while a terminal agent runs
  without a session id, the node looks every 3 s, for at most 30 minutes, at the files the glob matches that were written after the
  agent started, and takes the id from the first line (at most 64 KiB) when it passes `select` and its `cwd` is the agent's folder.
  It takes one only when exactly one new file fits and no other agent has that id. Two agents of one kind waiting in one folder at
  once get none (a file cannot say which is whose); neither does an agent for which two files fit. An id must be at most 200
  letters, digits, `-`, `_` or `.`. An agent without an id can only be started again in its folder, never resumed.
- An adapter or ACP agent reports its own session (`session/reported`); it needs neither.

The table is new in this engine version; older nodes ignore it. When the node itself runs inside a Claude Code session (it inherited
`CLAUDECODE` or `CLAUDE_CODE_CHILD_SESSION`), it leaves `CLAUDECODE` and every `CLAUDE_CODE_*` variable out of the terminal agents it
starts: Claude Code that inherits them takes itself for a child session and saves no transcript, so `--resume` would have nothing to
load. A node started normally passes them on.

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
