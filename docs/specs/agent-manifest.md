# Agent manifest (L1) format

One file per agent kind: `manifests/agents/<kind>.toml`. Manifests are data; the engine in
`agora-adapters` interprets them. Detection rules must be evidence-based: captured from real screens and
covered by fixtures under `manifests/agents/fixtures/<kind>/`.
Fixtures are replayed at 120x40 unless their filename specifies the capture size:
`<name>@<cols>x<rows>.screen`, with a matching `<name>@<cols>x<rows>.expect`.
Keep the raw captured bytes and use the original terminal size so wrapping and clipping are tested.

**Where agent kinds come from.** The `agora` binary carries every `manifests/agents/*.toml` of the repository it was built from
as built-in defaults (`agora-node::manifests::BUILT_IN`, compiled in with `include_str!`; fixtures are not), so a fresh install
offers every shipped agent with nothing copied. It also has a built-in `shell` kind made on the machine itself, not from a
file: the user's shell (`$SHELL` when it names a file, else `sh`; on Windows `%ComSpec%`, else `cmd.exe`), so a fresh node can
always open a terminal. The node then reads its manifests folder (`<AGORA_HOME>/manifests`, or
`agora node run --manifests DIR`; `agora adapter install` writes there too): a file whose `kind` matches a built-in one replaces
it, a file with a new `kind` adds one. A built-in kind cannot be removed, only replaced. A file that does not parse is skipped
and named by `agora doctor`, and the built-in kind it meant to replace stays. The folder is read again before agents are listed
or started, so a new or changed file needs no restart. `agora doctor` lists each kind with where it came from (built in, or the
folder) and the program it looks for. A kind is "installed" when that program (the L3 adapter command, else the ACP `argv[0]`,
else `process.launch[0]`) is found as a file in the node's own `PATH` (the environment the node was started in, not the user's current
shell) or, when the manifest names a path, exists there. On Windows, a bare name with no extension that has no `.exe` but a
`.com`, `.bat` or `.cmd` of that name (`npx`, most npm global commands) is started by that file's full path, for all three
levels, looked up where the standard library looks and never in the agent's project folder; it is installed when that file
exists. A found file without the owner's execute bit is "broken" (`agent-broken`, ADR-0087 rung 1): there, but starting it would fail. Doctor runs nothing and reads nothing from the program, and shows no agent version because no manifest has a version command yet. A new shipped manifest must be added to `BUILT_IN`; a test fails until it is.

**The ACP registry's kinds (ADR-0092).** Under the built-in kinds the binary also carries the converted agents of the ACP registry,
`manifests/agents/registry/*.toml` (`agora-node::manifests::REGISTRY`), each of kind `registry-<registry id>`. They load first, so a
built-in or folder file of the same `kind` replaces one, and a registry kind whose id is the name of a hand-made kind (built in or
from the folder) is not offered at all (`antigravity-acp` covers `registry-antigravity-acp`). They are untested in Agora: the window
marks them "Untested" (a kind named `registry-*`; no manifest field and no protocol change), lists the ones a machine cannot find
under Settings → Agents, "Add an agent", with the version, licence and repository from the catalog built into the window, and leaves
those out of the Start window. Their program is the ACP `argv[0]`: `npx` or `uvx` for a package (which fetches the pinned version
when the agent first starts), else the registry's own program name. `agora doctor` counts them in one line (`registry-kinds`: how
many, and how many find their program) instead of one line per kind. No registry kind has a launch option, so none can skip
permission questions (ADR-0092 decision 5). A registry kind (by origin, not by name) whose program is a package runner (`npx`, `uvx`) is
started from the hub only by a signed start, and the node's policy can switch registry kinds off (`agents.registry = off`, `agents.registry.deny`): both in
`docs/specs/protocol.md`, "ACP registry kinds".

```toml
kind = "example-agent"          # stable id, lowercase, used in protocol and UI
display_name = "Example Agent"  # brand name, not translated
version = "2026.11.06.1"        # manifest version
min_engine = 2                  # 6 with `[structured] allow_always` (ADR-0101), 5 with `[quota]` (ADR-0091), 4 with `[accounts] credential_env` (ADR-0084), 3 with [[launch.setting]] (ADR-0066), 2 with [[launch.option]] (ADR-0059); 1 otherwise

[process]
names = ["example-agent", "exa"]        # foreground process names
launch = ["example-agent"]              # default argv
env = { EXAMPLE_NO_UPDATE_CHECK = "1" }

# ADR-0059: choices made when an agent starts (see "Launch options" below).
[[launch.option]]
id = "skip-permissions"                 # what a start names and a device signs: a-z, 0-9, `-`, at most 32
label = "option-skip-permissions"       # Fluent message id the windows show
args = ["--dangerously-skip-permissions"]   # added to the launch command (or the ACP server command)
skips_permissions = true                # the agent acts without asking; default false

# ADR-0066: values chosen when an agent starts (see "Launch settings" below).
[[launch.setting]]
id = "effort"                           # same rules as an option id, and never one an option has
label = "setting-effort"                # Fluent message id the windows show
args = ["--effort", "{value}"]          # `{value}` is replaced by the chosen value, inside that argument
values = ["low", "medium", "high"]      # optional: the values a start may choose; empty means any plain value
default = "medium"                      # optional: what the tool uses when nothing is chosen (display only)

[accounts]
config_dir_env = "EXAMPLE_HOME"         # lets the node isolate multiple accounts
login = ["example-agent", "login"]      # run in a terminal streamed to the user
status = ["example-agent", "whoami"]    # exit 0 = authenticated
# ADR-0084 (min_engine 4): the environment variables the agent reads for its login, the one `agora doctor` suggests first. Only
# names checked on a real version of the agent. Up to 8 names of at most 64 characters: a letter or `_`, then letters, digits or `_`;
# not a name the system needs (`PATH`, `HOME`, ...) and not `AGORA_*`; no duplicates. A manifest that breaks this is refused.
credential_env = ["EXAMPLE_API_TOKEN"]

[structured]
acp = { argv = ["example-agent", "--acp"] }   # present = L2 available (run through `agora acp-bridge`, see adapter-protocol.md)
# allow_always = false                        # ADR-0101 (min_engine 6): never offer the person "Allow always" for this kind. Set
#                                             # only for a kind a check on a real version found to keep such an answer beyond its
#                                             # session; absent = offered when the agent offers `allow_always` and the policy allows
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
  regexes still see the rows as they are. Whitespace at either end of a phrase is kept as one space and asks for a boundary there
  (`" build done "` is not found in "prebuild done"); a line break counts as one, and so do the start and end of the region, so a
  padded phrase, one word included, is still found when it wraps or stands at the start or end of the region.
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
require (`contains` in the rule, `all` or `any`; compared ignoring case and with every run of whitespace as one space, see
`mesh-tools.md`; a marker given only as a regex is not checked against the prompt), and the top-level
`input_box` is a multi-line regex over `bottom_non_empty_lines(12)` whose first capture group is the box's text: before pressing
Enter on a helper's first prompt the node needs both, with the box holding exactly that prompt. The `typed.screen` fixtures are
constructed from the captured idle screens with text in the box (a capture of that moment is not kept); the test checks that
`[[typed]]` matches them and nothing else and that `input_box` reads exactly the typed text.

Every `[[typed]]` rule must need a regex to match (#1150): its own `regex`, or a `regex` in `all`, not empty. A regex only in `any`
does not count (a phrase beside it can match instead), nor one only in `not` (it only excludes); a rule without one is refused as
`typed[i].regex`, and a manifest file with such a rule is not loaded. Why: phrases are found with whitespace collapsed, so a prompt that
holds a footer word the box breaks in its middle (`Entersend`, wrapped after `Enter`) reads as the footer `Enter send` once typed,
and the prompt check above does not see it (`Entersend` is not `Enter send`). Regexes see the rows as they are. Validation can only
check that the regex is there: write it so it needs the footer whole on one row, at its place right under the box, as the shipped
manifests do.

## Launch settings (ADR-0066)

`[[launch.setting]]` (engine 3) declares a value chosen when an agent starts: `model`, `effort`, `mode` or another id. A start gives
`(id, value)` pairs (`AgentStart`/`AgentLaunch.settings`); the node checks each against its own manifest and adds the setting's `args`,
with every `{value}` replaced by the value, after the options' arguments, in the manifest's order (so wherever the options go: the
launch command, or the end of `[structured] acp.argv`). The window gets the id, the label, the listed values and the default
(`AgentKind.settings`), never the arguments.

The value is never put through a shell and can never become a flag or a second argument: it must be a plain value (an ASCII letter
or digit first, then letters, digits and `. _ : / @ + -`, at most 128), and one of `values` when the manifest lists them; otherwise
`agent.setting_invalid {kind, setting}` and nothing starts. A setting the manifest does not declare, or one named twice, is
`agent.setting_unknown {kind, setting}`.

Validation: at most 8 settings; ids follow the option id rules and never repeat an option's id; `label` a Fluent message id (the
shipped manifests' labels must be in `locales/en/web.ftl` and `tui.ftl`, checked by `shipped_manifests.rs`); `args` from one to eight
non-empty arguments without NUL, at least one with `{value}`, none a known "do not ask" spelling; at most 32 `values`, each a plain
value and none repeated; `default` one a start could choose; unknown keys are an error. The shipped Claude Code and Codex manifests
declare `model` and `effort`, checked against those tools' own `--help`.

A setting chosen at the start lasts while the node runs (a restart after a crash keeps it). It is not in the session ledger, so an
agent resumed after the node itself restarted (ADR-0060) starts with the tool's defaults, as its launch options do.

## Manifests converted from the ACP registry (ADR-0092)
The package entry it writes is a draft of an entry in the signed agent-package list; the differences are in `docs/specs/agent-packages.md`.
`registry_convert` (`crates/agora-adapters`, run through `cargo run -p agora-adapters --example registry_convert -- <registry clone> <dir>`)
turns one registry `agent.json` into `registry-<id>.toml` (an L2 `[structured] acp` manifest, never a launch option) and a package entry
(`<id>.package.json`, ADR-0087 shape plus `tested = false`). `npx` becomes `npx --yes PKG@VERSION ARGS`, `uvx` becomes `uvx PKG==VERSION ARGS`,
`binary` becomes a `binary-download` installer with per-platform URL and SHA-256 (`installable = false` when one is missing). Skips are
reason codes (`registry.bad_token`, `registry.skips_permissions`, `registry.forbidden_executable` (a shell or interpreter as the executable), `registry.bad_field`, `registry.no_supported_distribution`, ...). Generated files start with a
header saying so and are never edited by hand. Nodes never read the registry.

## Launch options (ADR-0059)

`[[launch.option]]` (engine 2, so a manifest with one says `min_engine = 2`) declares named choices the start window offers. A start
names the ids it chose (`AgentStart`/`AgentLaunch.launch_options`); the node adds each option's `args`, in the manifest's order, after
the launch command (`[process] launch`, or `[resume] argv` when the agent is resumed) and before the `[mcp]` flags; for a structured
kind they are added to the end of `[structured] acp.argv`, the ACP server command. The arguments never leave the node: the window gets
the id, the label and `skips_permissions` (`AgentKind.launch_options`).

Validation: at most 8 options; ids unique, lowercase ASCII letters, digits and `-` (at most 32); `label` a Fluent message id (the
shipped manifests' labels must be in `locales/en/web.ftl` and `tui.ftl`, checked by `shipped_manifests.rs`); `args` from one to eight
non-empty arguments without NUL; an option whose arguments contain a known "do not ask" spelling (`--dangerously-*`, `--yolo`,
`--yolo-*`) must say `skips_permissions = true`; unknown keys are an error.

An option with `skips_permissions = true` makes the agent act without asking. The window never preselects it and asks before it is
used; the node allows it only as its owner's `skip_permissions` policy says (`forbid`, `allow`, `confined_only`; docs/specs/protocol.md)
and refuses it for helpers an agent starts (ADR-0057). A shipped manifest declares only a flag checked on a real version of the
agent, with that version in a comment; a flag that was not checked is not declared.

Manifests in this repository: `claude-code` and `codex` (screens captured from the real agents, `manifests/agents/fixtures/`); `wrongstack` and `wrongstack-acp` ship too (`manifests/agents/`). `crates/agora-adapters/tests/shipped_manifests.rs` checks every shipped manifest against its captured screens.

## Plan limits (`[quota]`, ADR-0091)
`[quota]` (`min_engine = 5`, only with `[usage.file]`) says where a selected usage line (the `select` of `[usage.file]` decides which) reports
how much of a plan window is used. Each `[[quota.window]]` (at most 4) has three dotted JSON paths, each at most 128 characters:
`percent` (a number, rounded and clamped to 0..100), `minutes` (the window's length, a whole number from 1 to 525 600) and `resets` (a Unix
time: seconds, or milliseconds when the value is 100 000 000 000 or more; both are read). All three must be numbers on the line, else the
window is skipped (unknown, never guessed). An optional `at` path (`[quota] at = "timestamp"`, an RFC 3339 string) names the line's own time:
when a file is read from its start (a resumed session), only the newest report of each window is sent, stamped with that time (else the file's
modification time), never later than now, so an old report is not shown as new; lines appended later are stamped with the read time. The node names the window from its length (`five_hour` for 300, `seven_day` for 10 080,
`minutes:<n>` otherwise), never from a key such as `primary`, bounds `resets` (not more than a day ago, not more than 31 days ahead; else
the observation is dropped) and sends `AgentQuotaObserved` when the numbers change, at most one per tool and window every 5 seconds. Nothing
else on the line is read for this: credits, balances, plan names and account ids stay in the file. Only the agent's own file counts (the
rule for tokens). A copy of the `codex` manifest in the node's manifests folder replaces the built-in one (see "Where agent kinds come from"), so a copy
made before `[quota]` existed gives no plan limits; the node prints one line at start for each folder file that replaces a built-in kind. Shipped for `codex` (`rate_limits.primary` and `.secondary`; the secondary may be missing).

## Usage files (ADR-0042)
The node follows the files of a running terminal agent whose kind has `[usage.file]`: a file belongs to the agent when it was touched after
the agent started and the directory it records (`cwd`) is the agent's worktree (until a line says it, usage lines wait, at most 1000).
A file whose tokens one agent counted is that agent's: no other agent counts it, and after its owner ended the claim holds against the
agents that were its peers then (an agent started later may reuse the path). When another live agent reads the same files (same glob)
in the same folder as written (ADR-0052), the folder cannot tell their sessions apart: then a file's tokens count for an agent only when
its name holds the agent's own session id (at least 8 characters; Claude Code's `<id>.jsonl`, Codex's `rollout-…-<id>.jsonl`). An
agent whose id is not known reads such files for the tool's plan windows only and counts no tokens; a file read that way stays so,
also after the peer has gone, until the agent's id names it (then it is read from its start, unless this agent counted it before:
nothing is counted twice). Two Codex agents in one folder get no id from
`[session.file]`, so their tokens are not counted: a session is counted for nobody rather than for two. Not seen as peers: two kinds
whose different globs match the same files, or one folder spelled two ways. Limits:
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
node's policy resumes by itself.

`[resume] continue_argv` and `[resume] continue` (ADR-0060 Amendment A) say how the node continues an agent that was **working** when the node
stopped, after it brought the agent back **by itself** (never when a person pressed Resume, and not for an agent that was waiting for a
person, idle, finished or unknown): `continue_argv` is the resume command with one more argument, `{continue}`, which stands for the sentence
in `continue` (that the session was cut off by a restart, to check the state of what it was doing first, then carry on); `{session_id}` is
replaced as in `argv`. The sentence is data: one plain line of at most 512 bytes, which does not begin with `-` and holds no `{`, `}` or
control character, and it is given as an argument of its own, never inside a shell string. The two fields go together: one without the
other, a `continue_argv` without `{session_id}`, or one in which `{continue}` is not an argument of its own makes the manifest invalid. A kind
without them is resumed as usual and gets no message. The node's policy line `resume.continue = on | off` (default on) decides, and no
command or message from the hub carries it; the ledger records when the message was sent, so it goes once for each interruption.

`[resume] fresh_argv` (optional) is the command that starts the session from nothing under the id the node holds (`claude --session-id
{session_id}`), for a tool that saves no session until it is first spoken to, so `argv` fails at once for an agent that never was. The node
uses it for one restart after a resume (its own, a person's or a crash restart) that failed within 30 seconds of starting, and then
goes back to `argv`. It needs a program (a plain name, no placeholder) and `{session_id}` in an argument, and takes no `{continue}`; otherwise the manifest is invalid. Only declare it
for a tool that refuses to start a new session over an id that already exists, so a failed resume of a real conversation is never
overwritten. A kind without it is resumed as before.

The node holds a session id only when it learned one:

- **`id_args`** (Claude Code, `--session-id <uuid>`, checked on 2.1.319): the node chooses a random v4 UUID at the agent's first launch,
  adds `id_args` with `{session_id}` replaced, and records the id (`AgentSessionReported`) right after the start. Restarts and
  resumes use `[resume]` instead and never add `id_args`.
- **`[session.file]`** (Codex, which has no flag for it: `~/.codex/sessions/YYYY/MM/DD/rollout-<time>-<id>.jsonl`, whose first line is
  `{"type":"session_meta","payload":{"id":...,"cwd":...}}`, checked on codex-cli 0.167.0 and 0.167.1): while a terminal agent runs
  without a session id, the node looks every 3 s, for at most 30 minutes, at the files the glob matches that were written after the
  agent's process started, and takes the id from the first line (at most 64 KiB) when it passes `select` and its `cwd` is the agent's folder.
  This is per process: a restart or a resume of an agent still without an id runs a new process, which waits (30 minutes, files newer
  than its own start) afresh, also when discovery gave up on the process before. A process's start is the time the node took right
  before spawning it, with no slack, so a restart moments after the earlier process wrote its file never gets that file; a file the
  new process writes within one tick of the file system's clock after its start can be missed. A process taken up from the terminal
  holder keeps that start as the earlier node recorded it in the ledger (`process_since_ms`); only a process whose start the node does
  not know counts files from 2 s before it first saw it. An id found for a process that has been replaced or stopped since is dropped,
  as is one another agent holds by the time it is set, and one found never replaces a session the agent already has (#925).
  It takes one only when exactly one new file fits and no other agent has that id. Two agents in one folder are rivals, and both get
  none (a file cannot say which is whose), when they are of one kind, or of kinds whose globs could match the same files: the fixed
  directory of one glob (before its first `*`, with `{home}` replaced, `.` and `..` resolved) is the other's or holds it, as written or
  with links resolved (a link is followed even before what it points to exists, and a `..` after a link steps out of
  where the link leads, as the filesystem does), compared without case (one folder on Windows and
  macOS; on Linux this only adds rivals). A root that cannot be resolved, for example through more than 40 links, makes its agent a
  rival of every other kind in the folder. They are not rivals when both `select`s ask for different values at the same `path`.
  This is decided from the manifests before any file is read, for every waiting agent at once, so kinds that share a session folder
  lose Resume in that folder rather than risk continuing the wrong conversation. Folders are looked at on disk only for agents that
  could contend (of one kind, or of two kinds whose `select`s are not exclusive), session roots only for two such kinds; two agents that both
  gave up already are not compared again, and each agent's folder and root are looked up again only after 30 s or when the path changed (a manifest reloaded under a running agent
  gives a new root; a workspace moved and its old path made a link is found the same as the new one at the next look). A root that exists is resolved by the OS's own rules; one not
  made yet is walked through its links. The lookups run on a thread of their own, never on the node's supervision thread, so a stalled mount cannot hold up state or approvals;
  until the answer arrives the agents concerned get no session (and are not given up on). An agent for which two files fit gets none
  either. The first line counts without its `\n` or `\r\n`. A session file's id must be a UUID. An agent without an id can only be
  started again in its folder, never resumed. Known limit: this assumes no link inside a wildcard directory of a session glob points
  into another kind's session root (`logs/*/*.jsonl` with `logs/linked` pointing at `store`, against `store/*.jsonl`); such a setup
  can attribute a session to the wrong agent.
- **Every id that goes on a command line** (`[resume]`, `id_args`) must start with a letter or digit and hold only letters, digits,
  `.`, `_` and `-`, at most 128 characters (`agora_node::session_files::argv_safe`), so an id can never be read as an option such as
  `--dangerously-bypass-approvals-and-sandbox`. It is checked where an id is found (session file), reported (a terminal agent's hook),
  read from the ledger, and right before the argv is built. An adapter's own id travels in JSON and may be anything, but one that
  fails the check is not kept in the ledger, so such an agent is offered Start again after a restart. `--` is not put before the id:
  it was not checked against the real `claude --resume` and `codex resume`.
- **Start again** of a kind with `id_args` gives it a new node-chosen id, so it can be resumed after the next restart.
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

## Login credentials (`[accounts] credential_env`, ADR-0084)

`credential_env` lists the environment variables the agent reads for its login. It does nothing by itself: the owner stores a value on the
node (`agora node secret set NAME`, value from standard input or a no-echo prompt, never an argument; `list` shows names only; `remove NAME`)
and names it for the kind in the node's policy file, `credentials.<kind> = NAME, NAME`. When an agent of that kind starts, the node puts
those names, valued from the store (else from the node's own environment of that name), into the agent's environment. Only names the
manifest declares are passed, only to that kind, and never to helpers of other kinds. Every name any loaded manifest declares is removed
from the environment the node inherited, for every agent whose kind is not listed for it. Shipped for Claude Code
(`CLAUDE_CODE_OAUTH_TOKEN`, `ANTHROPIC_API_KEY`) and Codex (`OPENAI_API_KEY`) only: names not checked on a real version are not declared.
A fault in the field (too many or malformed names, a reserved name) refuses the manifest (`accounts.credential_env`).
