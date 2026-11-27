//! L1 agent manifests: parsing and validation (docs/specs/agent-manifest.md).

use std::collections::{BTreeMap, HashSet};

use serde::Deserialize;

/// Highest manifest `min_engine` this engine can run. 2: `[[launch.option]]` (ADR-0059). 3: `[[launch.setting]]` (ADR-0066). 4: `[accounts] credential_env` (ADR-0084). 5: `[quota]` (ADR-0091).
/// 6: `[structured] allow_always` (ADR-0101).
pub const ENGINE_VERSION: u32 = 7;

/// Longest stable agent-kind id. Kinds are ASCII, so this is both bytes and characters.
pub const MAX_KIND_BYTES: usize = 64;

/// ADR-0066: most `[[launch.setting]]`s one manifest may declare, most allowed values one may list, and most arguments one may add.
pub const MAX_LAUNCH_SETTINGS: usize = 8;
const MAX_SETTING_VALUES: usize = 32;

/// Most `[[launch.option]]`s one manifest may declare, and most arguments one option may add.
pub const MAX_LAUNCH_OPTIONS: usize = 8;
const MAX_OPTION_ARGS: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    /// Not valid TOML for this schema.
    Parse(String),
    /// A field is present but unusable; `field` names it (never user-facing text).
    Invalid { field: String },
    /// The manifest needs a newer engine.
    EngineTooOld { required: u32, supported: u32 },
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "manifest parse error: {e}"),
            Self::Invalid { field } => write!(f, "manifest field is invalid: {field}"),
            Self::EngineTooOld {
                required,
                supported,
            } => {
                write!(f, "manifest needs engine {required}, this is {supported}")
            }
        }
    }
}

impl std::error::Error for ManifestError {}

/// State a rule reports. Mirrors the protocol states a screen can prove.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleState {
    Working,
    Blocked,
    Idle,
    Done,
}

/// One condition on a region's text.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Condition {
    pub contains: Option<String>,
    pub regex: Option<String>,
}

/// A screen rule. A rule matches when its own `contains`/`regex` (if any) match, every `all` condition
/// matches, at least one `any` condition matches (if `any` is non-empty), and no `not` condition matches.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub state: RuleState,
    pub priority: i32,
    pub region: String,
    pub contains: Option<String>,
    pub regex: Option<String>,
    #[serde(default)]
    pub all: Vec<Condition>,
    #[serde(default)]
    pub any: Vec<Condition>,
    #[serde(default)]
    pub not: Vec<Condition>,
}

/// The most `[process] names` a manifest may list, and the longest one in bytes.
pub const MAX_PROCESS_NAMES: usize = 8;
pub const MAX_PROCESS_NAME_BYTES: usize = 64;

/// `[process] names` (ADR-0094): file names, never paths or text to show: 1 to [`MAX_PROCESS_NAME_BYTES`] bytes with no separator,
/// space, control or hidden character, and at most [`MAX_PROCESS_NAMES`] of them.
fn foreground_names(names: &[String]) -> bool {
    names.len() <= MAX_PROCESS_NAMES
        && names.iter().all(|n| {
            !n.is_empty()
                && n.len() <= MAX_PROCESS_NAME_BYTES
                && n != "."
                && n != ".."
                && !n.chars().any(|c| {
                    c == '/'
                        || c == '\\'
                        || c.is_whitespace()
                        || c.is_control()
                        || agora_core::text::is_hidden(c)
                })
        })
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct Process {
    /// ADR-0094: the program names this kind runs as (the executable's file name the system reports for a process, `claude`,
    /// `claude.exe`). A plain terminal whose foreground program has one of them runs this kind inside it, and this kind's screen
    /// rules read it. Compared exactly (ignoring ASCII case on Windows, `agora_platform::same_program_name`), never with arguments or
    /// a path. None listed: [`Manifest::program_names`] takes the kind's own ACP program.
    #[serde(default)]
    pub names: Vec<String>,
    #[serde(default)]
    pub launch: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Resume {
    pub argv: Vec<String>,
    /// ADR-0060 Amendment A: the resume command with a first message, for a tool that takes one (`claude --resume {session_id}
    /// "{continue}"`): `{session_id}` and `{continue}` are replaced, and the message is one argument, never part of a shell string. Used
    /// only when the node brings back by itself an agent that was working. Empty: the kind is resumed as usual and gets no message.
    #[serde(default)]
    pub continue_argv: Vec<String>,
    /// A command that starts the session under the same id from nothing (`claude --session-id {session_id}`), for a tool that writes
    /// no session until the first message, so `argv` finds nothing to resume in an agent that was never spoken to. The node uses it
    /// only for the next restart after a resume that failed within [`FRESH_WINDOW_SECS`] seconds of starting. Empty: never. Only for a
    /// tool that refuses to start a new session over an existing id, so a failed resume of a real conversation is never overwritten.
    #[serde(default)]
    pub fresh_argv: Vec<String>,
    /// The sentence `{continue}` stands for: that the session was cut off by a restart, to check the state of what it was doing first
    /// and then carry on. Words are data, so they live here and not in the node's Rust.
    #[serde(default, rename = "continue")]
    pub continue_text: Option<String>,
}

/// How soon after a resume starts its failure counts as "there was nothing to resume" (`[resume] fresh_argv`).
pub const FRESH_WINDOW_SECS: u64 = 30;

/// The longest `[resume] continue` sentence, in bytes.
pub const MAX_CONTINUE_BYTES: usize = 512;

/// `[session]` (ADR-0060): how the node learns a terminal agent's own session id, which `[resume]` needs after a restart. Either the
/// node chooses it (`id_args`) or it reads it from the file the agent writes (`[session.file]`), not both.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionSpec {
    /// Arguments added to the first launch, with `{session_id}` replaced by an id the node chooses (a random v4 UUID) and records
    /// before anything else is said about the agent, for example `["--session-id", "{session_id}"]`. Not added to a restart or a resume.
    #[serde(default)]
    pub id_args: Vec<String>,
    pub file: Option<SessionFile>,
}

/// `[session.file]`: the session file an agent writes, whose first line names the session and its working directory.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionFile {
    /// Starts with `{home}/`; `*` matches within one path component.
    pub glob: String,
    /// The first line must have this value at this path.
    pub select: Option<UsageSelect>,
    /// Dotted paths in the first line (a JSON object): the session id, and the working directory it was started in.
    pub id: String,
    pub cwd: String,
}

/// An L3 adapter process that speaks the adapter protocol for this agent kind (docs/specs/adapter-protocol.md).
/// A kind with `[adapter]` is driven through it instead of a terminal.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterSpec {
    /// Program and arguments; the first element is the program.
    pub command: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

/// The agent speaks the Agent Client Protocol over stdio (L2, `[structured] acp = { argv = [...] }`). The node drives it through
/// the ACP bridge (`agora acp-bridge`, `agora-adapters::acp`), which looks like an L3 adapter to everything else.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcpSpec {
    /// Program and arguments of the agent in ACP mode; the first element is the program.
    pub argv: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

/// ADR-0059: one `[[launch.option]]`, a named choice made when an agent starts. Its `args` are added to the launch command (a terminal
/// kind) or to the ACP server command (`[structured] acp`); `label` is a Fluent message id the clients show.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchOption {
    pub id: String,
    pub label: String,
    pub args: Vec<String>,
    /// The option makes the agent act without asking (a permission-skipping flag). Never preselected, confirmed in the window,
    /// signed, audited and badged; the node's `skip_permissions` policy decides whether it may be used at all.
    #[serde(default)]
    pub skips_permissions: bool,
}

/// ADR-0066: one `[[launch.setting]]`, a value chosen when an agent starts (`model`, `effort`, `mode`, ...). Each `{value}` in `args`
/// is replaced by the chosen value (a [`valid_setting_value`], so it is one argument and never another flag); the arguments are
/// added to the launch command (a terminal kind) or to the ACP server command (`[structured] acp`). `label` is a Fluent message id.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchSetting {
    pub id: String,
    pub label: String,
    pub args: Vec<String>,
    /// The values a start may choose, in the order a client lists them. Empty: any [`valid_setting_value`] (a model name, which the
    /// tool itself checks).
    #[serde(default)]
    pub values: Vec<String>,
    /// What the tool uses when nothing is chosen, for a client to show; never added to the command by the node.
    #[serde(default)]
    pub default: Option<String>,
}

impl LaunchSetting {
    /// The arguments for `value`, or `None` when the value is not one this setting takes.
    pub fn args_for(&self, value: &str) -> Option<Vec<String>> {
        let allowed = valid_setting_value(value)
            && (self.values.is_empty() || self.values.iter().any(|v| v == value));
        allowed.then(|| {
            self.args
                .iter()
                .map(|a| a.replace("{value}", value))
                .collect()
        })
    }
}

/// ADR-0066: a value a start may give a setting. It ends up inside one argument of the agent's command line, so it is a plain token
/// (letters, digits, `.`, `_`, `:`, `/`, `@`, `+`, `-`; at most 128), and it never starts with `-` (it cannot become a flag) or with a
/// character a quoted template could break out of (no quote, no space, no `=`).
pub fn valid_setting_value(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && s.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '/' | '@' | '+' | '-')
        })
}

/// `[launch]`: what may be chosen when an agent starts (ADR-0059 options, ADR-0066 settings).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Launch {
    #[serde(default)]
    pub option: Vec<LaunchOption>,
    #[serde(default)]
    pub setting: Vec<LaunchSetting>,
}

/// `[structured]`: the ways an agent can be driven without a terminal. Keys this engine does not know yet (`adapter` as a registry
/// name) are accepted and ignored.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Structured {
    pub acp: Option<AcpSpec>,
    /// ADR-0101: `false` for a kind found to keep an "always" answer beyond its session: the node then never offers the person the
    /// always answer for it. Needs `min_engine = 6`, so an older engine (which would ignore the key and offer it) refuses the manifest.
    pub allow_always: Option<bool>,
}

impl Structured {
    /// ADR-0101: whether the node may offer "always" for this kind (the default: yes).
    pub fn offers_always(&self) -> bool {
        self.allow_always != Some(false)
    }
}

/// How the node hands an agent its own mesh MCP server (docs/specs/agent-manifest.md `[mcp]`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpSpec {
    pub method: McpMethod,
    /// `config_file` (required) and `cli_flag` (optional, for agents that take the server as flags and need no file): where to
    /// write the config, inside the agent's worktree.
    pub path: Option<String>,
    /// Only `json` for now.
    pub format: Option<String>,
    /// `cli_flag`: arguments appended to the launch command.
    #[serde(default)]
    pub flag: Vec<String>,
    /// `env`: variables set for the agent.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// ADR-0122 (engine 7): arguments that give the agent the owner's shared MCP servers too, appended only when it gets any. The
    /// node writes them to a file of its own, outside the worktree and readable by its user only (they may hold secrets), named by
    /// `{shared_config_file}`. A kind without it is given no shared MCP server.
    #[serde(default)]
    pub shared_flag: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpMethod {
    ConfigFile,
    CliFlag,
    Env,
}

/// How the numbers in a usage file are to be read (ADR-0042): each line is a JSON object, the paths are dotted object keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageMode {
    /// Every selected line is one more use: add it.
    Delta,
    /// Every selected line is the running total of the session: report the growth since the last one.
    Total,
}

/// A line must have `path` equal to `equals` to count as a usage line.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageSelect {
    pub path: String,
    pub equals: String,
}

/// `[usage.file]`: the file an agent writes its own usage to (ADR-0042).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageFile {
    /// Starts with `{home}/`; `*` matches within one path component.
    pub glob: String,
    /// Only `jsonl`.
    pub format: Option<String>,
    pub select: Option<UsageSelect>,
    /// The agent's working directory as the file records it; a file whose directory is not the agent's worktree is not the agent's.
    pub cwd: String,
    /// The latest value seen on any line is used for later usage lines (a line that has it uses its own).
    pub model: String,
    pub input: String,
    pub output: String,
    /// Prompt-side tokens counted in `input` as well (Claude Code reports them apart).
    pub cache_read: Option<String>,
    pub cache_write: Option<String>,
    /// A usage line whose value at this path was seen before is skipped (Claude Code repeats a message on several lines).
    pub dedupe: Option<String>,
    pub mode: UsageMode,
}

/// `[usage]`: other keys (`quota_profiles`) are accepted and not read yet.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct UsageSpec {
    pub source: Option<String>,
    pub file: Option<UsageFile>,
    /// `[usage.model_map]` (ADR-0021): the model id the agent reports → the price catalog's `provider/model` id. The node reports the
    /// mapped id with the agent's usage, so the hub prices it; an id not listed is reported as the agent gave it. At most
    /// [`MAX_MODEL_MAP`] entries of at most [`MAX_MODEL_ID`] bytes each.
    #[serde(default)]
    pub model_map: BTreeMap<String, String>,
}

/// Most entries a `[usage.model_map]` may have, and the longest model id on either side of one.
pub const MAX_MODEL_MAP: usize = 256;
pub const MAX_MODEL_ID: usize = 128;

impl UsageSpec {
    /// The catalog's id for a model the agent reported: its `[usage.model_map]` entry, or the id unchanged.
    pub fn catalog_model<'a>(&'a self, reported: &'a str) -> &'a str {
        self.model_map
            .get(reported)
            .map_or(reported, String::as_str)
    }
}

/// Whether `[usage.model_map]` is within its bounds: a reported id is printable text without spaces, and a catalog id is exactly
/// `provider/model`, both parts non-empty.
fn model_map_ok(map: &BTreeMap<String, String>) -> bool {
    let id_ok = |s: &str| {
        !s.is_empty()
            && s.len() <= MAX_MODEL_ID
            && s.chars().all(|c| !c.is_control() && !c.is_whitespace())
    };
    map.len() <= MAX_MODEL_MAP
        && map.iter().all(|(from, to)| {
            id_ok(from)
                && id_ok(to)
                && to
                    .split_once('/')
                    .is_some_and(|(p, m)| !p.is_empty() && !m.is_empty() && !m.contains('/'))
        })
}

/// Most `[[quota.window]]` entries a manifest may list, and the longest JSON path in one.
pub const MAX_QUOTA_WINDOWS: usize = 4;
pub const MAX_QUOTA_PATH: usize = 128;

/// ADR-0091: where one plan window sits in a selected line of the usage file. The three values are read, bounded and labelled by the
/// node (`usage::quota`); the window's name comes from its length in minutes, never from the keys of the file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuotaWindow {
    /// The share of the window used, 0 to 100 (a number; a fraction is rounded). Clamped by the node.
    pub percent: String,
    /// The window's length in minutes (a whole number, 1 or more).
    pub minutes: String,
    /// When the window resets, as a Unix time: seconds, or milliseconds when the value is 100 000 000 000 or more (both are accepted).
    pub resets: String,
}

/// `[quota]` (ADR-0091): which values of a selected `[usage.file]` line say how much of a plan window is used. Only for a kind that has
/// `[usage.file]`; the line's own `select` decides which lines count. Nothing else of the line is read for this.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuotaSpec {
    /// Optional path of the line's own time (an RFC 3339 string, as Codex's `timestamp`): when a file is read from its start, the
    /// observation is stamped with it instead of with the time the node read it. Read defensively; never later than now.
    pub at: Option<String>,
    #[serde(default)]
    pub window: Vec<QuotaWindow>,
}

/// ADR-0084: most names `[accounts] credential_env` may list, and the longest one.
pub const MAX_CREDENTIAL_ENV: usize = 8;
pub const MAX_CREDENTIAL_NAME: usize = 64;

/// `[accounts]`: other keys (`config_dir_env`, `login`, `status`) are accepted and not read yet.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct Accounts {
    /// ADR-0084: the environment variables the agent reads for its login, the one `agora doctor` suggests first. Only names checked on a
    /// real version of the agent belong here.
    #[serde(default)]
    pub credential_env: Vec<String>,
}

/// Names a manifest may not claim as a credential: scrubbing or filling one of them would break every agent (ADR-0084).
const RESERVED_ENV: &[&str] = &[
    "PATH",
    "PATHEXT",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TERM",
    "LANG",
    "PWD",
    "TMPDIR",
    "TEMP",
    "TMP",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "SYSTEMROOT",
    "COMSPEC",
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "DYLD_INSERT_LIBRARIES",
    "DYLD_LIBRARY_PATH",
];

/// ADR-0084: a plain environment variable name that may carry a credential: an ASCII letter or `_`, then letters, digits or `_`,
/// at most [`MAX_CREDENTIAL_NAME`] long, not one the system needs (`PATH`, `HOME`, ...) and not Agora's own (`AGORA_*`).
pub fn credential_name_ok(name: &str) -> bool {
    let mut chars = name.chars();
    let first_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    first_ok
        && name.len() <= MAX_CREDENTIAL_NAME
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !name.to_ascii_uppercase().starts_with("AGORA_")
        && !RESERVED_ENV.contains(&name.to_ascii_uppercase().as_str())
        && !windows_device_name(name)
}

/// A name Windows cannot use as a file name, with or without an extension (`CON`, `NUL`, `COM1`, ...): refused on every system, so a
/// store (a folder of files named by the secrets) made on one system works on all.
fn windows_device_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|p| {
        stem.strip_prefix(p)
            .is_some_and(|n| n.len() == 1 && n.as_bytes()[0].is_ascii_digit() && n != "0")
    })
}

/// An argv the operating system can spawn: a real program followed by arguments without NUL (an empty argument is valid).
fn spawnable_argv(argv: &[String]) -> bool {
    argv.first()
        .is_some_and(|program| !program.trim().is_empty())
        && argv.iter().all(|arg| !arg.contains('\0'))
}

/// Environment entries accepted by `std::process::Command` on every supported platform.
fn spawnable_env(env: &BTreeMap<String, String>) -> bool {
    env.iter().all(|(name, value)| {
        !name.is_empty() && !name.contains(['=', '\0']) && !value.contains('\0')
    })
}

/// A parsed, validated manifest. Sections the engine does not interpret yet (accounts, hooks,
/// structured) are accepted and ignored so newer manifests still load.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Manifest {
    pub kind: String,
    pub display_name: String,
    /// ADR-0123: who serves the kind's models (`anthropic`, `openai`, ...: `[a-z0-9-]`, at most 32), shown beside the agent; empty when
    /// the kind can use many providers or the manifest does not say. Older engines ignore it.
    #[serde(default)]
    pub provider: String,
    pub version: String,
    pub min_engine: u32,
    #[serde(default)]
    pub process: Process,
    pub resume: Option<Resume>,
    /// ADR-0060: how the node learns the agent's own session id (older engines ignore it).
    pub session: Option<SessionSpec>,
    #[serde(default)]
    pub rules: Vec<Rule>,
    pub mcp: Option<McpSpec>,
    pub adapter: Option<AdapterSpec>,
    pub structured: Option<Structured>,
    pub usage: Option<UsageSpec>,
    /// What the agent needs beyond its worktree when the node confines agents (ADR-0047). Requests only: the node honours them for
    /// kinds its owner trusts.
    pub sandbox: Option<SandboxNeeds>,
    /// ADR-0057: `[[ready]]` rules that say the input box is empty and waiting, stricter than an `idle` rule (which may also match a
    /// menu's selected row). The node types a helper's first prompt only into a screen these match; a kind without them never has its
    /// first prompt typed for it. Same shape as `[[rules]]`, with `state = "idle"`.
    #[serde(default)]
    pub ready: Vec<Rule>,
    /// ADR-0057: `[[typed]]` rules that say the input box holds typed text and nothing else is on screen (no dialog, no menu); checked
    /// with [`Manifest::input_box`] before the node presses Enter on a helper's first prompt. Same shape, `state = "idle"`, and each
    /// needs a regex to match (its own or one in `all`, #1150).
    #[serde(default)]
    pub typed: Vec<Rule>,
    /// ADR-0057: a regex over `bottom_non_empty_lines(12)` (multi-line) whose first capture group is the input box's text. Enter is pressed
    /// only when it holds exactly the typed prompt (whitespace ignored, so a wrapped line still counts).
    pub input_box: Option<String>,
    /// ADR-0059: `[[launch.option]]`s, in the order the start window lists them.
    #[serde(default)]
    pub launch: Launch,
    /// ADR-0084: what the agent reads for its login (older engines ignore it).
    pub accounts: Option<Accounts>,
    /// ADR-0091: the plan limits the tool reports in its usage file (`min_engine` 5).
    pub quota: Option<QuotaSpec>,
}

/// Folders an agent asks to read or write when confined; `~/` stands for the home folder.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct SandboxNeeds {
    #[serde(default)]
    pub read: Vec<String>,
    #[serde(default)]
    pub write: Vec<String>,
}

fn invalid(field: impl Into<String>) -> ManifestError {
    ManifestError::Invalid {
        field: field.into(),
    }
}

/// A Fluent message id: an ASCII letter, then letters, digits, `-` or `_`.
fn fluent_id(s: &str) -> bool {
    let mut c = s.chars();
    c.next().is_some_and(|c| c.is_ascii_alphabetic())
        && c.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && s.len() <= 64
}

/// ADR-0059: argument spellings known to make an agent act without asking (`--dangerously-skip-permissions`,
/// `--dangerously-bypass-approvals-and-sandbox`, any `--dangerously-*`, `--yolo`, `--yolo-plus`, `--yolo-*`).
pub fn skips_permissions_flag(arg: &str) -> bool {
    let a = arg.trim().to_ascii_lowercase();
    a.starts_with("--dangerously-")
        || a == "--yolo"
        || a.starts_with("--yolo-")
        || a.starts_with("--yolo=")
}

/// A launch option id: what a start names and a device signs (comma-joined), so lowercase ASCII, digits and `-` only.
pub fn valid_option_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 32
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

impl Resume {
    /// ADR-0060 Amendment A: `continue_argv` and `continue` go together; the sentence is one plain line that cannot be read as an option
    /// or as a placeholder; `continue_argv` carries both placeholders, `{continue}` as an argument of its own.
    fn validate(&self) -> Result<(), ManifestError> {
        if !spawnable_argv(&self.argv) || !self.argv.iter().any(|a| a.contains("{session_id}")) {
            return Err(invalid("resume.argv"));
        }
        if !self.fresh_argv.is_empty()
            && (self
                .fresh_argv
                .first()
                .is_none_or(|p| p.trim().is_empty() || p.contains(['{', '}']))
                || !self.fresh_argv.iter().any(|a| a.contains("{session_id}"))
                || self.fresh_argv.iter().any(|a| a.contains("{continue}"))
                || self.fresh_argv.iter().any(|a| a.contains('\0')))
        {
            return Err(invalid("resume.fresh_argv"));
        }
        let text = self.continue_text.as_deref();
        match (self.continue_argv.is_empty(), text) {
            (true, None) => return Ok(()),
            (false, Some(_)) => {}
            _ => return Err(invalid("resume.continue")),
        }
        let Some(text) = text else {
            return Err(invalid("resume.continue"));
        };
        let sentence_ok = !text.trim().is_empty()
            && text.len() <= MAX_CONTINUE_BYTES
            && !text.starts_with('-')
            && !text.contains(['{', '}'])
            && !text.chars().any(char::is_control);
        if !sentence_ok {
            return Err(invalid("resume.continue"));
        }
        let has = |needle: &str| {
            self.continue_argv
                .iter()
                .filter(|a| a.contains(needle))
                .count()
        };
        let own_argument = self.continue_argv.iter().any(|a| a == "{continue}");
        if !spawnable_argv(&self.continue_argv)
            || has("{session_id}") == 0
            || has("{continue}") != 1
            || !own_argument
        {
            return Err(invalid("resume.continue_argv"));
        }
        Ok(())
    }
}

impl Manifest {
    /// ADR-0094: the program names a plain terminal's foreground program is matched against for this kind: its `[process] names`, or,
    /// when it lists none, the program of its ACP launch (`[structured] acp argv[0]`) when that is a plain file name and not a shell,
    /// interpreter or package runner (a kind converted from the ACP registry runs its own binary there, `goose`, `kimi`; one launched
    /// with `npx` or `uvx` runs as the runtime and has no name of its own). The registry's generated manifests then need no change.
    pub fn program_names(&self) -> Vec<String> {
        if !self.process.names.is_empty() {
            return self.process.names.clone();
        }
        self.structured
            .as_ref()
            .and_then(|s| s.acp.as_ref())
            .and_then(|a| a.argv.first())
            .filter(|p| {
                foreground_names(std::slice::from_ref(*p))
                    && !crate::registry_convert::forbidden_executable(p)
            })
            .map(|p| vec![p.clone()])
            .unwrap_or_default()
    }

    /// ADR-0059: the declared option with this id.
    pub fn launch_option(&self, id: &str) -> Option<&LaunchOption> {
        self.launch.option.iter().find(|o| o.id == id)
    }

    /// ADR-0066: the declared setting with this id.
    pub fn launch_setting(&self, id: &str) -> Option<&LaunchSetting> {
        self.launch.setting.iter().find(|s| s.id == id)
    }

    /// What drives this kind without a terminal, as an adapter command: its own `[adapter]`, or the ACP bridge (`bridge` is the
    /// program that runs `acp-bridge`, the `agora` binary) around `[structured] acp`. `None` for a kind that runs in a terminal.
    pub fn driver(&self, bridge: &std::path::Path) -> Option<AdapterSpec> {
        if let Some(a) = &self.adapter {
            return Some(a.clone());
        }
        let acp = self.structured.as_ref()?.acp.as_ref()?;
        let mut command = vec![
            bridge.to_string_lossy().into_owned(),
            "acp-bridge".to_string(),
            "--".to_string(),
        ];
        command.extend(acp.argv.iter().cloned());
        Some(AdapterSpec {
            command,
            env: acp.env.clone(),
        })
    }

    pub fn parse(src: &str) -> Result<Self, ManifestError> {
        let m: Self = toml::from_str(src).map_err(|e| ManifestError::Parse(e.to_string()))?;
        m.validate()?;
        Ok(m)
    }

    fn validate(&self) -> Result<(), ManifestError> {
        if self.min_engine > ENGINE_VERSION {
            return Err(ManifestError::EngineTooOld {
                required: self.min_engine,
                supported: ENGINE_VERSION,
            });
        }
        let mut kind_chars = self.kind.chars();
        let kind_ok = kind_chars
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            && self.kind.len() <= MAX_KIND_BYTES
            && kind_chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !kind_ok {
            return Err(invalid("kind"));
        }
        if self.min_engine == 0 {
            return Err(invalid("min_engine"));
        }
        if self.provider.len() > 32
            || !self
                .provider
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(invalid("provider"));
        }
        if self.display_name.trim().is_empty()
            || self
                .display_name
                .chars()
                .any(|c| c.is_control() || agora_core::text::is_hidden(c))
        {
            return Err(invalid("display_name"));
        }
        let structured =
            self.adapter.is_some() || self.structured.as_ref().is_some_and(|s| s.acp.is_some());
        if self.process.launch.is_empty() && !structured {
            return Err(invalid("process.launch"));
        }
        if !self.process.launch.is_empty() && !spawnable_argv(&self.process.launch) {
            return Err(invalid("process.launch"));
        }
        if !spawnable_env(&self.process.env) {
            return Err(invalid("process.env"));
        }
        if !foreground_names(&self.process.names) {
            return Err(invalid("process.names"));
        }
        if let Some(r) = &self.resume {
            r.validate()?;
        }
        if let Some(a) = &self.accounts {
            let mut seen = HashSet::new();
            let ok = (a.credential_env.is_empty() || self.min_engine >= 4)
                && a.credential_env.len() <= MAX_CREDENTIAL_ENV
                && a.credential_env
                    .iter()
                    .all(|n| credential_name_ok(n) && seen.insert(n.to_ascii_uppercase()));
            if !ok {
                return Err(invalid("accounts.credential_env"));
            }
        }
        if self
            .adapter
            .as_ref()
            .is_some_and(|a| !spawnable_argv(&a.command))
        {
            return Err(invalid("adapter.command"));
        }
        if self
            .adapter
            .as_ref()
            .is_some_and(|a| !spawnable_env(&a.env))
        {
            return Err(invalid("adapter.env"));
        }
        if self
            .structured
            .as_ref()
            .and_then(|s| s.acp.as_ref())
            .is_some_and(|a| !spawnable_argv(&a.argv))
        {
            return Err(invalid("structured.acp.argv"));
        }
        if self
            .structured
            .as_ref()
            .and_then(|s| s.acp.as_ref())
            .is_some_and(|a| !spawnable_env(&a.env))
        {
            return Err(invalid("structured.acp.env"));
        }
        if let Some(u) = self.usage.as_ref().and_then(|u| u.file.as_ref()) {
            let paths_ok = [&u.cwd, &u.model, &u.input, &u.output]
                .into_iter()
                .chain(u.cache_read.iter())
                .chain(u.cache_write.iter())
                .chain(u.dedupe.iter())
                .chain(u.select.iter().map(|s| &s.path))
                .all(|p| !p.is_empty());
            let glob_ok = u.glob.starts_with("{home}/")
                && !u.glob.split('/').any(|c| c == "..")
                && !u.glob.contains('\0');
            if !paths_ok || !glob_ok || u.format.as_deref().is_some_and(|f| f != "jsonl") {
                return Err(invalid("usage.file"));
            }
        }
        if self
            .usage
            .as_ref()
            .is_some_and(|u| !model_map_ok(&u.model_map))
        {
            return Err(invalid("usage.model_map"));
        }
        if self
            .structured
            .as_ref()
            .is_some_and(|s| s.allow_always.is_some())
            && self.min_engine < 6
        {
            return Err(invalid("structured.allow_always"));
        }
        if let Some(q) = &self.quota {
            let path_ok = |p: &String| {
                !p.is_empty()
                    && p.len() <= MAX_QUOTA_PATH
                    && !p.contains('\0')
                    && p.split('.').all(|k| !k.is_empty())
            };
            let ok = self.min_engine >= 5
                && self.usage.as_ref().is_some_and(|u| u.file.is_some())
                && !q.window.is_empty()
                && q.window.len() <= MAX_QUOTA_WINDOWS
                && q.at.as_ref().is_none_or(path_ok)
                && q.window
                    .iter()
                    .all(|w| [&w.percent, &w.minutes, &w.resets].into_iter().all(path_ok));
            if !ok {
                return Err(invalid("quota"));
            }
        }
        if let Some(m) = &self.mcp {
            let ok = match m.method {
                McpMethod::ConfigFile => m
                    .path
                    .as_deref()
                    .is_some_and(|p| !p.is_empty() && !p.contains('\0')),
                McpMethod::CliFlag => {
                    m.path
                        .as_deref()
                        .is_none_or(|p| !p.is_empty() && !p.contains('\0'))
                        && !m.flag.is_empty()
                        && m.flag.iter().all(|arg| !arg.contains('\0'))
                }
                McpMethod::Env => !m.env.is_empty(),
            };
            if !ok || !spawnable_env(&m.env) || m.format.as_deref().is_some_and(|f| f != "json") {
                return Err(invalid("mcp"));
            }
        }
        if let Some(sp) = &self.session {
            let args_ok = sp.id_args.is_empty()
                || (sp.id_args.iter().any(|a| a.contains("{session_id}"))
                    && sp.id_args.iter().all(|a| !a.contains('\0')));
            let file_ok = sp.file.as_ref().is_none_or(|f| {
                f.glob.starts_with("{home}/")
                    && !f.glob.split('/').any(|c| c == "..")
                    && !f.glob.contains('\0')
                    && !f.id.is_empty()
                    && !f.cwd.is_empty()
                    && f.select.as_ref().is_none_or(|s| !s.path.is_empty())
            });
            if !args_ok || !file_ok || (!sp.id_args.is_empty() && sp.file.is_some()) {
                return Err(invalid("session"));
            }
        }
        for (name, list) in [("ready", &self.ready), ("typed", &self.typed)] {
            for (i, r) in list.iter().enumerate() {
                if r.state != RuleState::Idle {
                    return Err(invalid(format!("{name}[{i}].state")));
                }
            }
        }
        if let Some(re) = &self.input_box {
            let ok = crate::rules::compile(re).is_ok_and(|r| r.captures_len() >= 2);
            if !ok {
                return Err(invalid("input_box"));
            }
        }
        self.validate_launch()?;
        for (name, list) in [
            ("rules", &self.rules),
            ("ready", &self.ready),
            ("typed", &self.typed),
        ] {
            Self::validate_rules(name, list)?;
        }
        // #1150: phrases are found with runs of whitespace as one space, so a prompt holding a footer word the input box breaks in its
        // middle would read as the footer once typed. Only a regex sees the rows as they are: every `[[typed]]` rule must need one to
        // match, its own or one in `all` (one in `any` need not match, one in `not` only excludes), and not one that matches empty
        // text (`''`, `^`, `.*`, `\s*`), which every screen passes.
        for (i, r) in self.typed.iter().enumerate() {
            let needs_regex = r
                .regex
                .iter()
                .chain(r.all.iter().filter_map(|c| c.regex.as_ref()))
                .any(|re| crate::rules::compile(re).is_ok_and(|r| !r.is_match("")));
            if !needs_regex {
                return Err(invalid(format!("typed[{i}].regex")));
            }
        }
        Ok(())
    }

    fn validate_launch(&self) -> Result<(), ManifestError> {
        let options = &self.launch.option;
        if (!options.is_empty() && self.min_engine < 2) || options.len() > MAX_LAUNCH_OPTIONS {
            return Err(invalid("launch.option"));
        }
        let mut ids = HashSet::new();
        for (i, o) in options.iter().enumerate() {
            let at = |f: &str| invalid(format!("launch.option[{i}].{f}"));
            if !valid_option_id(&o.id) || !ids.insert(o.id.as_str()) {
                return Err(at("id"));
            }
            if !fluent_id(&o.label) {
                return Err(at("label"));
            }
            let args_ok = !o.args.is_empty()
                && o.args.len() <= MAX_OPTION_ARGS
                && o.args
                    .iter()
                    .all(|a| !a.trim().is_empty() && a.len() <= 256 && !a.contains('\0'));
            if !args_ok {
                return Err(at("args"));
            }
            // A flag known to make an agent act without asking must be declared as such, so a mislabelled option cannot dodge the
            // node's `skip_permissions` policy, the confirmation or the badge.
            if !o.skips_permissions && o.args.iter().any(|a| skips_permissions_flag(a)) {
                return Err(at("skips_permissions"));
            }
        }
        // ADR-0066: settings. Same id rules as options (the two share no id: a start names both by id); `args` must use `{value}`;
        // listed values and the default are values a start could give; a setting can never add a flag that skips permissions.
        let settings = &self.launch.setting;
        if (!settings.is_empty() && self.min_engine < 3) || settings.len() > MAX_LAUNCH_SETTINGS {
            return Err(invalid("launch.setting"));
        }
        let mut setting_ids = HashSet::new();
        for (i, st) in settings.iter().enumerate() {
            let at = |f: &str| invalid(format!("launch.setting[{i}].{f}"));
            if !valid_option_id(&st.id)
                || ids.contains(st.id.as_str())
                || !setting_ids.insert(st.id.as_str())
            {
                return Err(at("id"));
            }
            if !fluent_id(&st.label) {
                return Err(at("label"));
            }
            let args_ok = !st.args.is_empty()
                && st.args.len() <= MAX_OPTION_ARGS
                && st.args.iter().any(|a| a.contains("{value}"))
                && st
                    .args
                    .iter()
                    .all(|a| !a.trim().is_empty() && a.len() <= 256 && !a.contains('\0'))
                && !st.args.iter().any(|a| skips_permissions_flag(a));
            if !args_ok {
                return Err(at("args"));
            }
            let mut seen = HashSet::new();
            if st.values.len() > MAX_SETTING_VALUES
                || !st
                    .values
                    .iter()
                    .all(|v| valid_setting_value(v) && seen.insert(v.as_str()))
            {
                return Err(at("values"));
            }
            if let Some(d) = &st.default {
                if st.args_for(d).is_none() {
                    return Err(at("default"));
                }
            }
        }
        Ok(())
    }

    fn validate_rules(name: &str, rules: &[Rule]) -> Result<(), ManifestError> {
        let mut ids = HashSet::new();
        for (i, r) in rules.iter().enumerate() {
            let at = |f: &str| invalid(format!("{name}[{i}].{f}"));
            if r.id.is_empty() || !ids.insert(r.id.as_str()) {
                return Err(at("id"));
            }
            crate::rules::Region::parse(&r.region).ok_or_else(|| at("region"))?;
            let own = r.contains.is_some() || r.regex.is_some();
            if r.contains.is_some() && r.regex.is_some() {
                // Exactly one of `contains`/`regex`, like every condition in `all`/`any`/`not`: the
                // detector would silently drop a rule that names both.
                return Err(at("conditions"));
            }
            if !own && r.all.is_empty() && r.any.is_empty() {
                // A rule with only `not` would match every screen without that text: too easy to get wrong.
                return Err(at("conditions"));
            }
            for (name, c) in r.contains.iter().map(|c| ("contains", c)) {
                if c.is_empty() {
                    return Err(at(name));
                }
            }
            if let Some(re) = &r.regex {
                crate::rules::compile(re).map_err(|_| at("regex"))?;
            }
            for (group, list) in [("all", &r.all), ("any", &r.any), ("not", &r.not)] {
                for (j, c) in list.iter().enumerate() {
                    let ok = match (&c.contains, &c.regex) {
                        (Some(s), None) => !s.is_empty(),
                        (None, Some(re)) => crate::rules::compile(re).is_ok(),
                        _ => false, // exactly one of contains/regex
                    };
                    if !ok {
                        return Err(at(&format!("{group}[{j}]")));
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod launch_tests {
    use super::*;

    fn with(launch: &str) -> String {
        format!("kind = \"k\"\ndisplay_name = \"K\"\nversion = \"1\"\nmin_engine = 2\n[process]\nlaunch = [\"k\"]\n{launch}")
    }

    const SKIP: &str = "[[launch.option]]\nid = \"skip-permissions\"\nlabel = \"option-skip-permissions\"\nargs = [\"--yes\"]\nskips_permissions = true\n";

    fn plain(tail: &str) -> String {
        format!("kind = \"k\"\ndisplay_name = \"K\"\nversion = \"1\"\nmin_engine = 1\n{tail}")
    }

    /// ADR-0123: a manifest may say who serves its models; a provider in the wrong form refuses the manifest.
    #[test]
    fn a_provider_is_a_short_plain_id() {
        let p = |line: &str| Manifest::parse(&format!("{line}\n{}", with("")));
        assert_eq!(p("").unwrap().provider, "");
        assert_eq!(p("provider = \"anthropic\"").unwrap().provider, "anthropic");
        let long = "a".repeat(33);
        for bad in ["Anthropic", "open ai", "x/y", "\\u001b[2J", long.as_str()] {
            assert!(p(&format!("provider = \"{bad}\"")).is_err(), "{bad}");
        }
    }

    #[test]
    fn launch_options_are_read_in_order_and_harmless_by_default() {
        let src = format!("{SKIP}[[launch.option]]\nid = \"resume-last\"\nlabel = \"option-resume-last\"\nargs = [\"--continue\"]\n");
        let m = Manifest::parse(&with(&src)).unwrap();
        let ids: Vec<_> = m.launch.option.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids, ["skip-permissions", "resume-last"]);
        assert!(
            m.launch_option("skip-permissions")
                .unwrap()
                .skips_permissions
        );
        assert!(!m.launch_option("resume-last").unwrap().skips_permissions);
        assert!(m.launch_option("other").is_none());
        assert!(Manifest::parse(&with("")).unwrap().launch.option.is_empty());
        // Declared as skipping permissions, the same flags load.
        for flag in ["--dangerously-skip-permissions", "--yolo", "--yolo-plus"] {
            assert!(
                Manifest::parse(&with(&SKIP.replace("--yes", flag))).is_ok(),
                "{flag}"
            );
        }
        assert!(!skips_permissions_flag("--yolonger") && !skips_permissions_flag("--continue"));
    }

    #[test]
    fn a_bad_launch_option_is_refused() {
        for bad in [
            format!("{SKIP}{SKIP}"),
            SKIP.replace("skip-permissions\"", "Skip_Permissions\""),
            SKIP.replace("id = \"skip-permissions\"", "id = \"\""),
            SKIP.replace("[\"--yes\"]", "[]"),
            SKIP.replace("[\"--yes\"]", "[\" \"]"),
            SKIP.replace("option-skip-permissions", "Don't ask"),
            SKIP.replace("option-skip-permissions", ""),
            // A known "do not ask" flag declared as harmless.
            SKIP.replace("skips_permissions = true", "skips_permissions = false")
                .replace("--yes", "--dangerously-skip-permissions"),
            SKIP.replace("skips_permissions = true\n", "")
                .replace("--yes", "--yolo"),
            SKIP.replace("skips_permissions = true", "skips_permissions = false")
                .replace("--yes", "--yolo-plus"),
            SKIP.replace("skips_permissions = true", "skips_permissions = false")
                .replace("--yes", "--Dangerously-Bypass-Approvals-And-Sandbox"),
            format!("{SKIP}extra = 1\n"),
            (0..=MAX_LAUNCH_OPTIONS)
                .map(|i| SKIP.replace("\"skip-permissions\"", &format!("\"o{i}\"")))
                .collect::<String>(),
        ] {
            assert!(Manifest::parse(&with(&bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn credential_env_is_validated_and_read() {
        let with_env = |names: &str| {
            Manifest::parse(&format!(
                "{}\n[accounts]\nlogin = [\"k\", \"login\"]\ncredential_env = {names}\n",
                with("").replace("min_engine = 2", "min_engine = 4")
            ))
        };
        let ok = with_env("[\"K_TOKEN\", \"_K2\"]").unwrap();
        assert_eq!(ok.accounts.unwrap().credential_env, ["K_TOKEN", "_K2"]);
        assert!(Manifest::parse(&with("")).unwrap().accounts.is_none());
        let long = format!("[\"{}\"]", "A".repeat(MAX_CREDENTIAL_NAME + 1));
        let many = format!(
            "[{}]",
            (0..=MAX_CREDENTIAL_ENV)
                .map(|i| format!("\"N{i}\""))
                .collect::<Vec<_>>()
                .join(",")
        );
        for bad in [
            "[\"\"]",
            "[\"1A\"]",
            "[\"A=B\"]",
            "[\"A B\"]",
            "[\"A*\"]",
            "[\"$A\"]",
            "[\"PATH\"]",
            "[\"path\"]",
            "[\"AGORA_TOKEN\"]",
            "[\"CON\"]",
            "[\"nul\"]",
            "[\"Com1\"]",
            "[\"LPT9\"]",
            "[\"AUX\"]",
            "[\"A\", \"a\"]",
            "[\"\u{00e9}\"]",
            &long,
            &many,
        ] {
            assert!(with_env(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_manifest_newer_than_this_engine_is_refused() {
        let src = with(SKIP).replace("min_engine = 2", "min_engine = 8");
        assert!(matches!(
            Manifest::parse(&src),
            Err(ManifestError::EngineTooOld {
                required: 8,
                supported: ENGINE_VERSION
            })
        ));
    }

    /// ADR-0094: a kind is matched by its `[process] names`; one that lists none (a kind converted from the ACP registry) by its ACP
    /// program when that is a plain file name of its own, never a shell, an interpreter, a package runner or a path.
    #[test]
    fn a_kinds_program_names_come_from_its_names_else_its_own_acp_program() {
        let acp = |argv: &str, names: &str| {
            Manifest::parse(&format!(
                "kind = \"k\"\ndisplay_name = \"K\"\nversion = \"1\"\nmin_engine = 1\n{names}\n[structured]\nacp = {{ argv = [{argv}] }}\n"
            ))
            .unwrap()
            .program_names()
        };
        assert_eq!(acp(r#""goose", "acp""#, ""), ["goose"]);
        assert_eq!(acp(r#""kiro-cli", "acp""#, ""), ["kiro-cli"]);
        assert_eq!(
            acp(r#""goose", "acp""#, "[process]\nnames = [\"goose-cli\"]"),
            ["goose-cli"]
        );
        for none in [
            r#""npx", "--yes", "x@1""#,
            r#""uvx", "x==1""#,
            r#""node", "x.js""#,
            r#""bash", "-c", "x""#,
            r#""./bin/agent""#,
            r#""C:\\x\\agent.exe""#,
        ] {
            assert!(acp(none, "").is_empty(), "{none}");
        }
    }

    #[test]
    fn proof_f450_display_names_cannot_control_the_users_terminal() {
        let control = Manifest::parse(&plain("[process]\nlaunch = [\"tool\"]\n"));
        assert!(control.is_ok());
        println!("CONTROL OK");
        let actual = Manifest::parse(
            &plain("[process]\nlaunch = [\"tool\"]\n")
                .replace("display_name = \"K\"", "display_name = \"clear\\u001B[2J\""),
        );
        println!("EXPECTED: a terminal-active display_name is rejected");
        println!("ACTUAL: accepted = {}", actual.is_ok());
        if actual.is_ok() {
            println!("PROBLEM CONFIRMED");
        } else {
            println!("PROBLEM NOT REPRODUCED");
        }
        assert!(actual.is_err());
    }

    #[test]
    fn proof_f451_terminal_launch_requires_a_real_program() {
        let control = Manifest::parse(&plain("[process]\nlaunch = [\"tool\", \"--ok\"]\n"));
        assert!(control.is_ok());
        println!("CONTROL OK");
        let actual = Manifest::parse(&plain("[process]\nlaunch = [\"\"]\n"));
        println!("EXPECTED: an empty L1 launch program is rejected");
        println!("ACTUAL: accepted = {}", actual.is_ok());
        if actual.is_ok() {
            println!("PROBLEM CONFIRMED");
        } else {
            println!("PROBLEM NOT REPRODUCED");
        }
        assert!(actual.is_err());
    }

    #[test]
    fn proof_f452_adapter_commands_reject_unspawnable_arguments() {
        let control = Manifest::parse(&plain("[adapter]\ncommand = [\"tool\", \"--ok\"]\n"));
        assert!(control.is_ok());
        println!("CONTROL OK");
        let actual = Manifest::parse(&plain(
            "[adapter]\ncommand = [\"tool\", \"bad\\u0000arg\"]\n",
        ));
        println!("EXPECTED: an L3 adapter argv containing NUL is rejected");
        println!("ACTUAL: accepted = {}", actual.is_ok());
        if actual.is_ok() {
            println!("PROBLEM CONFIRMED");
        } else {
            println!("PROBLEM NOT REPRODUCED");
        }
        assert!(actual.is_err());
    }

    #[test]
    fn proof_f453_acp_commands_reject_unspawnable_arguments() {
        let control = Manifest::parse(&plain(
            "[structured]\nacp = { argv = [\"tool\", \"--ok\"] }\n",
        ));
        assert!(control.is_ok());
        println!("CONTROL OK");
        let actual = Manifest::parse(&plain(
            "[structured]\nacp = { argv = [\"tool\\u0000name\"] }\n",
        ));
        println!("EXPECTED: an L2 ACP argv containing NUL is rejected");
        println!("ACTUAL: accepted = {}", actual.is_ok());
        if actual.is_ok() {
            println!("PROBLEM CONFIRMED");
        } else {
            println!("PROBLEM NOT REPRODUCED");
        }
        assert!(actual.is_err());
    }

    #[test]
    fn proof_f454_resume_commands_reject_unspawnable_arguments() {
        let control = Manifest::parse(&plain(
            "[process]\nlaunch = [\"tool\"]\n[resume]\nargv = [\"tool\", \"--resume\", \"{session_id}\"]\n",
        ));
        assert!(control.is_ok());
        println!("CONTROL OK");
        let actual = Manifest::parse(&plain(
            "[process]\nlaunch = [\"tool\"]\n[resume]\nargv = [\"tool\", \"--resume\", \"{session_id}\\u0000\"]\n",
        ));
        println!("EXPECTED: a resume argv containing NUL is rejected");
        println!("ACTUAL: accepted = {}", actual.is_ok());
        if actual.is_ok() {
            println!("PROBLEM CONFIRMED");
        } else {
            println!("PROBLEM NOT REPRODUCED");
        }
        assert!(actual.is_err());
    }

    #[test]
    fn verify_f450_display_names_reject_terminal_layout_and_hidden_characters() {
        for escaped in ["\\u001B", "\\n", "\\t", "\\u202E", "\\u200B"] {
            let src = plain("[process]\nlaunch = [\"tool\"]\n").replace(
                "display_name = \"K\"",
                &format!("display_name = \"safe{escaped}name\""),
            );
            assert!(matches!(
                Manifest::parse(&src),
                Err(ManifestError::Invalid { field }) if field == "display_name"
            ));
        }
        let unicode = plain("[process]\nlaunch = [\"tool\"]\n")
            .replace("display_name = \"K\"", "display_name = \"Çözüm 工具\"");
        assert!(Manifest::parse(&unicode).is_ok());
        println!("FIX VERIFIED: F450 display names are visible inert text");
    }

    #[test]
    fn process_names_are_plain_file_names() {
        let with = |names: &str| {
            Manifest::parse(&format!(
                "kind = \"k\"\ndisplay_name = \"K\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"k\"]\nnames = {names}\n"
            ))
        };
        for ok in [
            "[]",
            "[\"claude\"]",
            "[\"claude\", \"claude.exe\", \"wstack\"]",
        ] {
            assert!(with(ok).is_ok(), "{ok}");
        }
        let long = format!("[\"{}\"]", "a".repeat(MAX_PROCESS_NAME_BYTES + 1));
        let many = format!(
            "[{}]",
            (0..=MAX_PROCESS_NAMES)
                .map(|i| format!("\"n{i}\""))
                .collect::<Vec<_>>()
                .join(", ")
        );
        for bad in [
            "[\"\"]",
            "[\"bin/claude\"]",
            "[\"a\\\\b\"]",
            "[\"two words\"]",
            "[\"cl\\u001baude\"]",
            "[\"cl\\u202eaude\"]",
            "[\"..\"]",
            long.as_str(),
            many.as_str(),
        ] {
            assert!(
                matches!(with(bad), Err(ManifestError::Invalid { field }) if field == "process.names"),
                "{bad}"
            );
        }
    }

    #[test]
    fn verify_f451_terminal_argv_rejects_bad_programs_and_nul_but_keeps_empty_args() {
        for launch in [
            "[\"\"]",
            "[\"tool\\u0000\"]",
            "[\"tool\", \"bad\\u0000arg\"]",
        ] {
            let src = plain(&format!("[process]\nlaunch = {launch}\n"));
            assert!(matches!(
                Manifest::parse(&src),
                Err(ManifestError::Invalid { field }) if field == "process.launch"
            ));
        }
        assert!(Manifest::parse(&plain("[process]\nlaunch = [\"tool\", \"\"]\n")).is_ok());
        assert!(Manifest::parse(&plain("[structured]\nacp = { argv = [\"tool\"] }\n")).is_ok());
        println!(
            "FIX VERIFIED: F451 L1 launch argv is spawnable without rejecting empty arguments"
        );
    }

    #[test]
    fn verify_f452_adapter_argv_rejects_empty_program_and_nul_at_every_position() {
        for command in [
            "[]",
            "[\"\"]",
            "[\"tool\\u0000\"]",
            "[\"tool\", \"bad\\u0000arg\"]",
        ] {
            let src = plain(&format!("[adapter]\ncommand = {command}\n"));
            assert!(matches!(
                Manifest::parse(&src),
                Err(ManifestError::Invalid { field }) if field == "adapter.command"
            ));
        }
        assert!(Manifest::parse(&plain("[adapter]\ncommand = [\"tool\", \"\"]\n")).is_ok());
        println!("FIX VERIFIED: F452 L3 adapter argv is spawnable");
    }

    #[test]
    fn verify_f453_acp_argv_rejects_empty_program_and_nul_at_every_position() {
        for argv in [
            "[]",
            "[\"\"]",
            "[\"tool\\u0000\"]",
            "[\"tool\", \"bad\\u0000arg\"]",
        ] {
            let src = plain(&format!("[structured]\nacp = {{ argv = {argv} }}\n"));
            assert!(matches!(
                Manifest::parse(&src),
                Err(ManifestError::Invalid { field }) if field == "structured.acp.argv"
            ));
        }
        assert!(
            Manifest::parse(&plain("[structured]\nacp = { argv = [\"tool\", \"\"] }\n")).is_ok()
        );
        println!("FIX VERIFIED: F453 L2 ACP argv is spawnable");
    }

    #[test]
    fn verify_f454_every_resume_argv_rejects_nul_and_valid_empty_args_remain() {
        let main = plain(
            "[process]\nlaunch = [\"tool\"]\n[resume]\nargv = [\"tool\", \"\", \"{session_id}\"]\n",
        );
        assert!(Manifest::parse(&main).is_ok());
        let bad_main = main.replace("{session_id}", "{session_id}\\u0000");
        assert!(matches!(
            Manifest::parse(&bad_main),
            Err(ManifestError::Invalid { field }) if field == "resume.argv"
        ));

        let bad_fresh = plain(
            "[process]\nlaunch = [\"tool\"]\n[resume]\nargv = [\"tool\", \"{session_id}\"]\nfresh_argv = [\"tool\", \"{session_id}\\u0000\"]\n",
        );
        assert!(matches!(
            Manifest::parse(&bad_fresh),
            Err(ManifestError::Invalid { field }) if field == "resume.fresh_argv"
        ));

        let bad_continue = plain(
            "[process]\nlaunch = [\"tool\"]\n[resume]\nargv = [\"tool\", \"{session_id}\"]\ncontinue_argv = [\"tool\", \"bad\\u0000arg\", \"{session_id}\", \"{continue}\"]\ncontinue = \"carry on\"\n",
        );
        assert!(matches!(
            Manifest::parse(&bad_continue),
            Err(ManifestError::Invalid { field }) if field == "resume.continue_argv"
        ));
        println!("FIX VERIFIED: F454 resume, fresh, and continue argv are spawnable");
    }

    #[test]
    fn proof_f455_session_id_arguments_reject_nul() {
        let control = Manifest::parse(&plain(
            "[process]\nlaunch = [\"tool\"]\n[session]\nid_args = [\"--session-id\", \"{session_id}\"]\n",
        ));
        assert!(control.is_ok());
        println!("CONTROL OK");
        let actual = Manifest::parse(&plain(
            "[process]\nlaunch = [\"tool\"]\n[session]\nid_args = [\"--session-id\", \"{session_id}\\u0000\"]\n",
        ));
        println!("EXPECTED: session id argv containing NUL is rejected");
        println!("ACTUAL: accepted = {}", actual.is_ok());
        if actual.is_ok() {
            println!("PROBLEM CONFIRMED");
        } else {
            println!("PROBLEM NOT REPRODUCED");
        }
        assert!(actual.is_err());
    }

    #[test]
    fn proof_f456_terminal_environment_rejects_unspawnable_values() {
        let control = Manifest::parse(&plain(
            "[process]\nlaunch = [\"tool\"]\nenv = { OK = \"value\" }\n",
        ));
        assert!(control.is_ok());
        println!("CONTROL OK");
        let actual = Manifest::parse(&plain(
            "[process]\nlaunch = [\"tool\"]\nenv = { OK = \"bad\\u0000value\" }\n",
        ));
        println!("EXPECTED: L1 environment values containing NUL are rejected");
        println!("ACTUAL: accepted = {}", actual.is_ok());
        if actual.is_ok() {
            println!("PROBLEM CONFIRMED");
        } else {
            println!("PROBLEM NOT REPRODUCED");
        }
        assert!(actual.is_err());
    }

    #[test]
    fn proof_f457_adapter_environment_rejects_unspawnable_names() {
        let control = Manifest::parse(&plain(
            "[adapter]\ncommand = [\"tool\"]\nenv = { OK = \"value\" }\n",
        ));
        assert!(control.is_ok());
        println!("CONTROL OK");
        let actual = Manifest::parse(&plain(
            "[adapter]\ncommand = [\"tool\"]\nenv = { \"BAD=NAME\" = \"value\" }\n",
        ));
        println!("EXPECTED: L3 environment names containing '=' are rejected");
        println!("ACTUAL: accepted = {}", actual.is_ok());
        if actual.is_ok() {
            println!("PROBLEM CONFIRMED");
        } else {
            println!("PROBLEM NOT REPRODUCED");
        }
        assert!(actual.is_err());
    }

    #[test]
    fn proof_f458_acp_environment_rejects_unspawnable_values() {
        let control = Manifest::parse(&plain(
            "[structured.acp]\nargv = [\"tool\"]\nenv = { OK = \"value\" }\n",
        ));
        assert!(control.is_ok());
        println!("CONTROL OK");
        let actual = Manifest::parse(&plain(
            "[structured.acp]\nargv = [\"tool\"]\nenv = { OK = \"bad\\u0000value\" }\n",
        ));
        println!("EXPECTED: L2 environment values containing NUL are rejected");
        println!("ACTUAL: accepted = {}", actual.is_ok());
        if actual.is_ok() {
            println!("PROBLEM CONFIRMED");
        } else {
            println!("PROBLEM NOT REPRODUCED");
        }
        assert!(actual.is_err());
    }

    #[test]
    fn proof_f459_mcp_cli_flags_reject_unspawnable_arguments() {
        let control = Manifest::parse(&plain(
            "[process]\nlaunch = [\"tool\"]\n[mcp]\nmethod = \"cli_flag\"\nflag = [\"--mcp\", \"{socket}\"]\n",
        ));
        assert!(control.is_ok());
        println!("CONTROL OK");
        let actual = Manifest::parse(&plain(
            "[process]\nlaunch = [\"tool\"]\n[mcp]\nmethod = \"cli_flag\"\nflag = [\"--mcp\\u0000\"]\n",
        ));
        println!("EXPECTED: MCP argv additions containing NUL are rejected");
        println!("ACTUAL: accepted = {}", actual.is_ok());
        if actual.is_ok() {
            println!("PROBLEM CONFIRMED");
        } else {
            println!("PROBLEM NOT REPRODUCED");
        }
        assert!(actual.is_err());
    }

    #[test]
    fn verify_f455_session_id_argv_rejects_nul_but_keeps_empty_arguments() {
        for args in [
            "[\"bad\\u0000\", \"{session_id}\"]",
            "[\"--id\", \"{session_id}\\u0000\"]",
        ] {
            let src = plain(&format!(
                "[process]\nlaunch = [\"tool\"]\n[session]\nid_args = {args}\n"
            ));
            assert!(
                matches!(Manifest::parse(&src), Err(ManifestError::Invalid { field }) if field == "session")
            );
        }
        assert!(Manifest::parse(&plain(
            "[process]\nlaunch = [\"tool\"]\n[session]\nid_args = [\"--id\", \"\", \"{session_id}\"]\n"
        ))
        .is_ok());
        println!("FIX VERIFIED: F455 session id argv cannot fail at OS spawn");
    }

    #[test]
    fn verify_f456_terminal_environment_rejects_bad_names_and_values() {
        for env in [
            "{ \"\" = \"v\" }",
            "{ \"BAD=NAME\" = \"v\" }",
            "{ \"BAD\\u0000NAME\" = \"v\" }",
            "{ OK = \"bad\\u0000value\" }",
        ] {
            let src = plain(&format!("[process]\nlaunch = [\"tool\"]\nenv = {env}\n"));
            assert!(
                matches!(Manifest::parse(&src), Err(ManifestError::Invalid { field }) if field == "process.env")
            );
        }
        assert!(Manifest::parse(&plain(
            "[process]\nlaunch = [\"tool\"]\nenv = { OK = \"\" }\n"
        ))
        .is_ok());
        println!("FIX VERIFIED: F456 L1 environment is spawnable");
    }

    #[test]
    fn verify_f457_adapter_environment_rejects_bad_names_and_values() {
        for env in ["{ \"BAD=NAME\" = \"v\" }", "{ OK = \"bad\\u0000value\" }"] {
            let src = plain(&format!("[adapter]\ncommand = [\"tool\"]\nenv = {env}\n"));
            assert!(
                matches!(Manifest::parse(&src), Err(ManifestError::Invalid { field }) if field == "adapter.env")
            );
        }
        assert!(Manifest::parse(&plain(
            "[adapter]\ncommand = [\"tool\"]\nenv = { OK = \"v\" }\n"
        ))
        .is_ok());
        println!("FIX VERIFIED: F457 L3 environment is spawnable");
    }

    #[test]
    fn verify_f458_acp_environment_rejects_bad_names_and_values() {
        for env in ["{ \"BAD=NAME\" = \"v\" }", "{ OK = \"bad\\u0000value\" }"] {
            let src = plain(&format!(
                "[structured.acp]\nargv = [\"tool\"]\nenv = {env}\n"
            ));
            assert!(
                matches!(Manifest::parse(&src), Err(ManifestError::Invalid { field }) if field == "structured.acp.env")
            );
        }
        assert!(Manifest::parse(&plain(
            "[structured.acp]\nargv = [\"tool\"]\nenv = { OK = \"v\" }\n"
        ))
        .is_ok());
        println!("FIX VERIFIED: F458 L2 environment is spawnable");
    }

    #[test]
    fn verify_f459_mcp_cli_flags_reject_nul_but_keep_empty_arguments() {
        for flags in ["[\"bad\\u0000\"]", "[\"--mcp\", \"bad\\u0000arg\"]"] {
            let src = plain(&format!(
                "[process]\nlaunch = [\"tool\"]\n[mcp]\nmethod = \"cli_flag\"\nflag = {flags}\n"
            ));
            assert!(
                matches!(Manifest::parse(&src), Err(ManifestError::Invalid { field }) if field == "mcp")
            );
        }
        assert!(Manifest::parse(&plain("[process]\nlaunch = [\"tool\"]\n[mcp]\nmethod = \"cli_flag\"\nflag = [\"--mcp\", \"\"]\n")).is_ok());
        println!("FIX VERIFIED: F459 MCP argv additions are spawnable");
    }

    #[test]
    fn proof_f460_mcp_environment_rejects_unspawnable_entries() {
        let control = Manifest::parse(&plain(
            "[process]\nlaunch = [\"tool\"]\n[mcp]\nmethod = \"env\"\n[mcp.env]\nOK = \"value\"\n",
        ));
        assert!(control.is_ok());
        println!("CONTROL OK");
        let actual = Manifest::parse(&plain(
            "[process]\nlaunch = [\"tool\"]\n[mcp]\nmethod = \"env\"\n[mcp.env]\n\"BAD=NAME\" = \"value\"\n",
        ));
        println!("EXPECTED: MCP environment names containing '=' are rejected");
        println!("ACTUAL: accepted = {}", actual.is_ok());
        if actual.is_ok() {
            println!("PROBLEM CONFIRMED");
        } else {
            println!("PROBLEM NOT REPRODUCED");
        }
        assert!(actual.is_err());
    }

    #[test]
    fn proof_f461_mcp_paths_reject_nul() {
        let control = Manifest::parse(&plain(
            "[process]\nlaunch = [\"tool\"]\n[mcp]\nmethod = \"config_file\"\npath = \".mcp.json\"\n",
        ));
        assert!(control.is_ok());
        println!("CONTROL OK");
        let actual = Manifest::parse(&plain(
            "[process]\nlaunch = [\"tool\"]\n[mcp]\nmethod = \"config_file\"\npath = \"bad\\u0000.json\"\n",
        ));
        println!("EXPECTED: an MCP config path containing NUL is rejected");
        println!("ACTUAL: accepted = {}", actual.is_ok());
        if actual.is_ok() {
            println!("PROBLEM CONFIRMED");
        } else {
            println!("PROBLEM NOT REPRODUCED");
        }
        assert!(actual.is_err());
    }

    #[test]
    fn proof_f462_manifest_kinds_fit_the_visible_approval_identity() {
        let control = Manifest::parse(&plain("[process]\nlaunch = [\"tool\"]\n"));
        assert!(control.is_ok());
        println!("CONTROL OK");
        let actual = Manifest::parse(
            &plain("[process]\nlaunch = [\"tool\"]\n")
                .replace("kind = \"k\"", &format!("kind = \"{}\"", "k".repeat(65))),
        );
        println!("EXPECTED: a kind longer than the 64-character approval identity is rejected");
        println!("ACTUAL: accepted = {}", actual.is_ok());
        if actual.is_ok() {
            println!("PROBLEM CONFIRMED");
        } else {
            println!("PROBLEM NOT REPRODUCED");
        }
        assert!(actual.is_err());
    }

    #[test]
    fn proof_f463_engine_versions_start_at_one() {
        let control = Manifest::parse(&plain("[process]\nlaunch = [\"tool\"]\n"));
        assert!(control.is_ok());
        println!("CONTROL OK");
        let actual = Manifest::parse(
            &plain("[process]\nlaunch = [\"tool\"]\n").replace("min_engine = 1", "min_engine = 0"),
        );
        println!("EXPECTED: engine version zero is rejected");
        println!("ACTUAL: accepted = {}", actual.is_ok());
        if actual.is_ok() {
            println!("PROBLEM CONFIRMED");
        } else {
            println!("PROBLEM NOT REPRODUCED");
        }
        assert!(actual.is_err());
    }

    #[test]
    fn proof_f464_terminal_manifests_need_a_launch_command() {
        let control = Manifest::parse(&plain("[process]\nlaunch = [\"tool\"]\n"));
        assert!(control.is_ok());
        println!("CONTROL OK");
        let actual = Manifest::parse(
            "kind = \"k\"\ndisplay_name = \"K\"\nversion = \"1\"\nmin_engine = 1\n",
        );
        println!("EXPECTED: a terminal manifest without process.launch is rejected");
        println!("ACTUAL: accepted = {}", actual.is_ok());
        if actual.is_ok() {
            println!("PROBLEM CONFIRMED");
        } else {
            println!("PROBLEM NOT REPRODUCED");
        }
        assert!(actual.is_err());
    }

    #[test]
    fn verify_f460_mcp_environment_is_spawnable() {
        let src = |entry: &str| {
            plain(&format!(
                "[process]\nlaunch = [\"tool\"]\n[mcp]\nmethod = \"env\"\n[mcp.env]\n{entry}\n"
            ))
        };
        for entry in [
            "\"\" = \"v\"",
            "\"BAD=NAME\" = \"v\"",
            "\"BAD\\u0000NAME\" = \"v\"",
            "OK = \"bad\\u0000value\"",
        ] {
            assert!(
                matches!(Manifest::parse(&src(entry)), Err(ManifestError::Invalid { field }) if field == "mcp")
            );
        }
        assert!(Manifest::parse(&src("OK = \"\"")).is_ok());
        println!("FIX VERIFIED: F460 MCP environment entries are spawnable");
    }

    #[test]
    fn verify_f461_mcp_file_paths_reject_nul_and_empty_values() {
        let manifest =
            |tail: &str| plain(&format!("[process]\nlaunch = [\"tool\"]\n[mcp]\n{tail}"));
        for tail in [
            "method = \"config_file\"\npath = \"bad\\u0000.json\"\n",
            "method = \"cli_flag\"\npath = \"bad\\u0000.json\"\nflag = [\"--mcp\"]\n",
            "method = \"cli_flag\"\npath = \"\"\nflag = [\"--mcp\"]\n",
        ] {
            assert!(
                matches!(Manifest::parse(&manifest(tail)), Err(ManifestError::Invalid { field }) if field == "mcp")
            );
        }
        assert!(Manifest::parse(&manifest(
            "method = \"config_file\"\npath = \".mcp.json\"\n"
        ))
        .is_ok());
        assert!(Manifest::parse(&manifest("method = \"cli_flag\"\nflag = [\"--mcp\"]\n")).is_ok());
        println!("FIX VERIFIED: F461 MCP file paths are usable OS paths");
    }

    #[test]
    fn verify_f462_kind_length_boundary() {
        let manifest = |len| {
            plain("[process]\nlaunch = [\"tool\"]\n")
                .replace("kind = \"k\"", &format!("kind = \"{}\"", "k".repeat(len)))
        };
        assert!(Manifest::parse(&manifest(MAX_KIND_BYTES)).is_ok());
        assert!(
            matches!(Manifest::parse(&manifest(MAX_KIND_BYTES + 1)), Err(ManifestError::Invalid { field }) if field == "kind")
        );
        println!("FIX VERIFIED: F462 kind ids fit their 64-character contract");
    }

    #[test]
    fn verify_f463_engine_version_lower_and_upper_boundaries() {
        let manifest = |engine| {
            plain("[process]\nlaunch = [\"tool\"]\n")
                .replace("min_engine = 1", &format!("min_engine = {engine}"))
        };
        assert!(
            matches!(Manifest::parse(&manifest(0)), Err(ManifestError::Invalid { field }) if field == "min_engine")
        );
        assert!(Manifest::parse(&manifest(1)).is_ok());
        assert!(Manifest::parse(&manifest(ENGINE_VERSION)).is_ok());
        assert!(matches!(
            Manifest::parse(&manifest(ENGINE_VERSION + 1)),
            Err(ManifestError::EngineTooOld { .. })
        ));
        println!("FIX VERIFIED: F463 engine versions begin at one");
    }

    #[test]
    fn verify_f464_only_structured_manifests_may_omit_terminal_launch() {
        assert!(
            matches!(Manifest::parse(&plain("")), Err(ManifestError::Invalid { field }) if field == "process.launch")
        );
        assert!(Manifest::parse(&plain("[adapter]\ncommand = [\"tool\"]\n")).is_ok());
        assert!(Manifest::parse(&plain("[structured]\nacp = { argv = [\"tool\"] }\n")).is_ok());
        assert!(Manifest::parse(&plain("[process]\nlaunch = [\"tool\"]\n")).is_ok());
        println!("FIX VERIFIED: F464 terminal manifests name a launch command");
    }

    #[test]
    fn proof_f465_kind_ids_start_with_an_alphanumeric_character() {
        let control = Manifest::parse(&plain("[process]\nlaunch = [\"tool\"]\n"));
        assert!(control.is_ok());
        println!("CONTROL OK");
        let actual = Manifest::parse(
            &plain("[process]\nlaunch = [\"tool\"]\n").replace("kind = \"k\"", "kind = \"-kind\""),
        );
        println!("EXPECTED: a kind beginning with punctuation is rejected");
        println!("ACTUAL: accepted = {}", actual.is_ok());
        if actual.is_ok() {
            println!("PROBLEM CONFIRMED");
        } else {
            println!("PROBLEM NOT REPRODUCED");
        }
        assert!(actual.is_err());
    }

    #[test]
    fn proof_f466_terminal_programs_are_not_whitespace() {
        let control = Manifest::parse(&plain("[process]\nlaunch = [\"tool\"]\n"));
        assert!(control.is_ok());
        println!("CONTROL OK");
        let actual = Manifest::parse(&plain("[process]\nlaunch = [\"   \"]\n"));
        let spawnable = std::process::Command::new("   ").status().is_ok();
        println!("EXPECTED: a whitespace-only L1 program is rejected");
        println!(
            "ACTUAL: accepted = {}, OS spawned = {spawnable}",
            actual.is_ok()
        );
        if actual.is_ok() && !spawnable {
            println!("PROBLEM CONFIRMED");
        } else {
            println!("PROBLEM NOT REPRODUCED");
        }
        assert!(actual.is_err());
    }

    #[test]
    fn proof_f467_adapter_programs_are_not_whitespace() {
        let control = Manifest::parse(&plain("[adapter]\ncommand = [\"tool\"]\n"));
        assert!(control.is_ok());
        println!("CONTROL OK");
        let actual = Manifest::parse(&plain("[adapter]\ncommand = [\" \"]\n"));
        let spawnable = std::process::Command::new(" ").status().is_ok();
        println!("EXPECTED: a whitespace-only L3 program is rejected");
        println!(
            "ACTUAL: accepted = {}, OS spawned = {spawnable}",
            actual.is_ok()
        );
        if actual.is_ok() && !spawnable {
            println!("PROBLEM CONFIRMED");
        } else {
            println!("PROBLEM NOT REPRODUCED");
        }
        assert!(actual.is_err());
    }

    #[test]
    fn proof_f468_acp_programs_are_not_whitespace() {
        let control = Manifest::parse(&plain("[structured]\nacp = { argv = [\"tool\"] }\n"));
        assert!(control.is_ok());
        println!("CONTROL OK");
        let actual = Manifest::parse(&plain("[structured]\nacp = { argv = [\"\\t\"] }\n"));
        let spawnable = std::process::Command::new("\t").status().is_ok();
        println!("EXPECTED: a whitespace-only L2 program is rejected");
        println!(
            "ACTUAL: accepted = {}, OS spawned = {spawnable}",
            actual.is_ok()
        );
        if actual.is_ok() && !spawnable {
            println!("PROBLEM CONFIRMED");
        } else {
            println!("PROBLEM NOT REPRODUCED");
        }
        assert!(actual.is_err());
    }

    #[test]
    fn proof_f469_resume_programs_are_not_whitespace() {
        let control = Manifest::parse(&plain(
            "[process]\nlaunch = [\"tool\"]\n[resume]\nargv = [\"tool\", \"{session_id}\"]\n",
        ));
        assert!(control.is_ok());
        println!("CONTROL OK");
        let actual = Manifest::parse(&plain(
            "[process]\nlaunch = [\"tool\"]\n[resume]\nargv = [\"  \", \"{session_id}\"]\n",
        ));
        let spawnable = std::process::Command::new("  ").status().is_ok();
        println!("EXPECTED: a whitespace-only resume program is rejected");
        println!(
            "ACTUAL: accepted = {}, OS spawned = {spawnable}",
            actual.is_ok()
        );
        if actual.is_ok() && !spawnable {
            println!("PROBLEM CONFIRMED");
        } else {
            println!("PROBLEM NOT REPRODUCED");
        }
        assert!(actual.is_err());
    }

    #[test]
    fn verify_f465_kind_id_start_boundary() {
        let manifest = |kind: &str| {
            plain("[process]\nlaunch = [\"tool\"]\n")
                .replace("kind = \"k\"", &format!("kind = \"{kind}\""))
        };
        for valid in ["k", "0", "0-kind", "kind-"] {
            assert!(Manifest::parse(&manifest(valid)).is_ok(), "{valid}");
        }
        for invalid_kind in ["-kind", "--", "_kind"] {
            assert!(
                matches!(Manifest::parse(&manifest(invalid_kind)), Err(ManifestError::Invalid { field }) if field == "kind")
            );
        }
        println!("FIX VERIFIED: F465 kind ids start with a lowercase letter or digit");
    }

    #[test]
    fn verify_f466_terminal_program_boundary() {
        for program in [" ", "\\t", "  \\t  "] {
            let src = plain(&format!("[process]\nlaunch = [\"{program}\"]\n"));
            assert!(
                matches!(Manifest::parse(&src), Err(ManifestError::Invalid { field }) if field == "process.launch")
            );
        }
        assert!(Manifest::parse(&plain("[process]\nlaunch = [\"tool\", \"   \"]\n")).is_ok());
        println!("FIX VERIFIED: F466 L1 command names a real program");
    }

    #[test]
    fn verify_f467_adapter_program_boundary() {
        for program in [" ", "\\t", "  \\t  "] {
            let src = plain(&format!("[adapter]\ncommand = [\"{program}\"]\n"));
            assert!(
                matches!(Manifest::parse(&src), Err(ManifestError::Invalid { field }) if field == "adapter.command")
            );
        }
        assert!(Manifest::parse(&plain("[adapter]\ncommand = [\"tool\", \"   \"]\n")).is_ok());
        println!("FIX VERIFIED: F467 L3 command names a real program");
    }

    #[test]
    fn verify_f468_acp_program_boundary() {
        for program in [" ", "\\t", "  \\t  "] {
            let src = plain(&format!(
                "[structured]\nacp = {{ argv = [\"{program}\"] }}\n"
            ));
            assert!(
                matches!(Manifest::parse(&src), Err(ManifestError::Invalid { field }) if field == "structured.acp.argv")
            );
        }
        assert!(Manifest::parse(&plain(
            "[structured]\nacp = { argv = [\"tool\", \"   \"] }\n"
        ))
        .is_ok());
        println!("FIX VERIFIED: F468 L2 command names a real program");
    }

    #[test]
    fn verify_f469_every_resume_program_boundary() {
        let main =
            plain("[process]\nlaunch = [\"tool\"]\n[resume]\nargv = [\" \", \"{session_id}\"]\n");
        assert!(
            matches!(Manifest::parse(&main), Err(ManifestError::Invalid { field }) if field == "resume.argv")
        );
        let fresh = plain(
            "[process]\nlaunch = [\"tool\"]\n[resume]\nargv = [\"tool\", \"{session_id}\"]\nfresh_argv = [\"\\t\", \"{session_id}\"]\n",
        );
        assert!(
            matches!(Manifest::parse(&fresh), Err(ManifestError::Invalid { field }) if field == "resume.fresh_argv")
        );
        let carry = plain(
            "[process]\nlaunch = [\"tool\"]\n[resume]\nargv = [\"tool\", \"{session_id}\"]\ncontinue_argv = [\"  \", \"{session_id}\", \"{continue}\"]\ncontinue = \"carry on\"\n",
        );
        assert!(
            matches!(Manifest::parse(&carry), Err(ManifestError::Invalid { field }) if field == "resume.continue_argv")
        );
        let valid = plain(
            "[process]\nlaunch = [\"tool\"]\n[resume]\nargv = [\"tool\", \"\", \"{session_id}\"]\n",
        );
        assert!(Manifest::parse(&valid).is_ok());
        println!("FIX VERIFIED: F469 every resume command names a real program");
    }

    #[test]
    fn proof_f447_launch_options_require_the_engine_that_understands_them() {
        let control = Manifest::parse(&with(SKIP));
        assert!(control.is_ok());
        println!("CONTROL OK");
        let actual = Manifest::parse(&with(SKIP).replace("min_engine = 2", "min_engine = 1"));
        println!("EXPECTED: min_engine 1 refuses launch.option");
        println!("ACTUAL: accepted = {}", actual.is_ok());
        if actual.is_ok() {
            println!("PROBLEM CONFIRMED");
        } else {
            println!("PROBLEM NOT REPRODUCED");
        }
        assert!(actual.is_err());
    }

    #[test]
    fn proof_f448_launch_settings_require_the_engine_that_understands_them() {
        let control = Manifest::parse(&with(MODEL).replace("min_engine = 2", "min_engine = 3"));
        assert!(control.is_ok());
        println!("CONTROL OK");
        let actual = Manifest::parse(&with(MODEL).replace("min_engine = 2", "min_engine = 1"));
        println!("EXPECTED: min_engine 1 refuses launch.setting");
        println!("ACTUAL: accepted = {}", actual.is_ok());
        if actual.is_ok() {
            println!("PROBLEM CONFIRMED");
        } else {
            println!("PROBLEM NOT REPRODUCED");
        }
        assert!(actual.is_err());
    }

    #[test]
    fn proof_f449_credential_env_requires_the_engine_that_understands_it() {
        let manifest = |engine| {
            with("").replace("min_engine = 2", &format!("min_engine = {engine}"))
                + "\n[accounts]\nlogin = [\"k\", \"login\"]\ncredential_env = [\"K_TOKEN\"]\n"
        };
        let control = Manifest::parse(&manifest(4));
        assert!(control.is_ok());
        println!("CONTROL OK");
        let actual = Manifest::parse(&manifest(1));
        println!("EXPECTED: min_engine 1 refuses accounts.credential_env");
        println!("ACTUAL: accepted = {}", actual.is_ok());
        if actual.is_ok() {
            println!("PROBLEM CONFIRMED");
        } else {
            println!("PROBLEM NOT REPRODUCED");
        }
        assert!(actual.is_err());
    }

    #[test]
    fn verify_f447_launch_option_engine_floor_and_plain_engine_one_manifest() {
        assert!(Manifest::parse(&with("").replace("min_engine = 2", "min_engine = 1")).is_ok());
        let zero = with(SKIP).replace("min_engine = 2", "min_engine = 0");
        assert!(
            matches!(Manifest::parse(&zero), Err(ManifestError::Invalid { field }) if field == "min_engine")
        );
        let one = with(SKIP).replace("min_engine = 2", "min_engine = 1");
        assert!(
            matches!(Manifest::parse(&one), Err(ManifestError::Invalid { field }) if field == "launch.option")
        );
        assert!(Manifest::parse(&with(SKIP)).is_ok());
        println!("FIX VERIFIED: F447 launch options require engine 2 or newer");
    }

    #[test]
    fn verify_f448_launch_setting_engine_floor_boundaries() {
        for engine in [1, 2] {
            let src = with(MODEL).replace("min_engine = 2", &format!("min_engine = {engine}"));
            assert!(matches!(
                Manifest::parse(&src),
                Err(ManifestError::Invalid { field }) if field == "launch.setting"
            ));
        }
        let engine_three = with(MODEL).replace("min_engine = 2", "min_engine = 3");
        assert!(Manifest::parse(&engine_three).is_ok());
        println!("FIX VERIFIED: F448 launch settings require engine 3 or newer");
    }

    #[test]
    fn verify_f449_credential_engine_floor_and_empty_credential_list() {
        let manifest = |engine, names| {
            with("").replace("min_engine = 2", &format!("min_engine = {engine}"))
                + &format!("\n[accounts]\nlogin = [\"k\", \"login\"]\ncredential_env = {names}\n")
        };
        assert!(Manifest::parse(&manifest(1, "[]")).is_ok());
        for engine in [1, 3] {
            assert!(matches!(
                Manifest::parse(&manifest(engine, "[\"K_TOKEN\"]")),
                Err(ManifestError::Invalid { field }) if field == "accounts.credential_env"
            ));
        }
        assert!(Manifest::parse(&manifest(4, "[\"K_TOKEN\"]")).is_ok());
        println!("FIX VERIFIED: F449 credential declarations require engine 4 or newer");
    }

    /// ADR-0101: `[structured] allow_always = false` switches the always answer off for a kind, and needs engine 6.
    #[test]
    fn allow_always_switches_the_offer_off_and_needs_engine_six() {
        let acp = |engine: u32, extra: &str| {
            format!("kind = \"k-acp\"\ndisplay_name = \"K\"\nversion = \"1\"\nmin_engine = {engine}\n[structured]\nacp = {{ argv = [\"k\", \"acp\"] }}\n{extra}")
        };
        let on = Manifest::parse(&acp(1, "")).expect("plain acp kind");
        assert!(on
            .structured
            .as_ref()
            .is_some_and(Structured::offers_always));
        let off = Manifest::parse(&acp(6, "allow_always = false\n")).expect("switched off");
        assert!(!off
            .structured
            .as_ref()
            .is_some_and(Structured::offers_always));
        assert!(matches!(
            Manifest::parse(&acp(5, "allow_always = false\n")),
            Err(ManifestError::Invalid { .. })
        ));
    }

    const EFFORT: &str = "[[launch.setting]]\nid = \"effort\"\nlabel = \"setting-effort\"\nargs = [\"-c\", \"model_reasoning_effort=\\\"{value}\\\"\"]\nvalues = [\"low\", \"high\"]\ndefault = \"low\"\n";
    const MODEL: &str = "[[launch.setting]]\nid = \"model\"\nlabel = \"setting-model\"\nargs = [\"--model\", \"{value}\"]\n";

    #[test]
    fn launch_settings_put_a_checked_value_into_one_argument() {
        // ADR-0066.
        let src = with(&format!("{EFFORT}{MODEL}")).replace("min_engine = 2", "min_engine = 3");
        let m = Manifest::parse(&src).unwrap();
        let effort = m.launch_setting("effort").unwrap();
        assert_eq!(
            effort.args_for("high").unwrap(),
            ["-c", "model_reasoning_effort=\"high\""]
        );
        assert_eq!(effort.args_for("medium"), None, "not listed");
        let model = m.launch_setting("model").unwrap();
        assert_eq!(
            model.args_for("gpt-6.1-sol").unwrap(),
            ["--model", "gpt-6.1-sol"]
        );
        // A value can never become a flag, another argument, or break out of a quoted template.
        for bad in [
            "",
            "-x",
            "--dangerously-skip-permissions",
            "a b",
            "a\"b",
            "a=b",
            "a;b",
            "a\nb",
            &"x".repeat(129),
        ] {
            assert_eq!(model.args_for(bad), None, "{bad:?}");
        }
        assert!(m.launch_setting("mode").is_none());
    }

    #[test]
    fn a_launch_setting_that_could_be_misused_is_refused() {
        let field = |src: &str| match Manifest::parse(
            &with(src).replace("min_engine = 2", "min_engine = 3"),
        ) {
            Err(ManifestError::Invalid { field }) => field,
            other => panic!("{other:?}"),
        };
        // No `{value}` in its arguments, a flag that skips permissions, a listed value that is not a plain token, a default that is
        // not listed, a duplicate id, and an id an option already has.
        assert_eq!(
            field(&MODEL.replace("{value}", "x")),
            "launch.setting[0].args"
        );
        assert_eq!(
            field(&MODEL.replace("\"--model\", ", "\"--dangerously-skip-permissions\", ")),
            "launch.setting[0].args"
        );
        assert_eq!(
            field(&EFFORT.replace("\"high\"]", "\"-high\"]")),
            "launch.setting[0].values"
        );
        assert_eq!(
            field(&EFFORT.replace("default = \"low\"", "default = \"max\"")),
            "launch.setting[0].default"
        );
        assert_eq!(field(&format!("{MODEL}{MODEL}")), "launch.setting[1].id");
        let clash = format!(
            "{SKIP}{}",
            MODEL.replace("\"model\"", "\"skip-permissions\"")
        );
        assert_eq!(field(&clash), "launch.setting[0].id");
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod usage_tests {
    use super::*;

    fn with(file: &str) -> String {
        format!(
            "kind = \"k\"\ndisplay_name = \"K\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"k\"]\n[usage]\nsource = \"file\"\n[usage.file]\n{file}"
        )
    }

    const GOOD: &str = "glob = \"{home}/.k/*/*.jsonl\"\ncwd = \"cwd\"\nmodel = \"m\"\ninput = \"i\"\noutput = \"o\"\nmode = \"delta\"\n";

    const QUOTA: &str =
        "[[quota.window]]\npercent = \"a.used\"\nminutes = \"a.len\"\nresets = \"a.at\"\n";

    fn quota(min_engine: u32, usage: bool, tail: &str) -> String {
        let head = format!(
            "kind = \"k\"\ndisplay_name = \"K\"\nversion = \"1\"\nmin_engine = {min_engine}\n[process]\nlaunch = [\"k\"]\n"
        );
        let u = if usage {
            format!("[usage]\nsource = \"file\"\n[usage.file]\n{GOOD}")
        } else {
            String::new()
        };
        format!("{head}{u}[quota]\n{tail}")
    }

    /// ADR-0021: `[usage.model_map]` is read and bounded; an id it does not list is reported as given.
    #[test]
    fn a_model_map_is_read_and_bounded() {
        let m = Manifest::parse(&format!(
            "{}[usage.model_map]\n\"big-1\" = \"example/big-1-2026\"\n",
            with(GOOD)
        ))
        .unwrap();
        let u = m.usage.unwrap();
        assert_eq!(u.catalog_model("big-1"), "example/big-1-2026");
        assert_eq!(u.catalog_model("other"), "other");
        // Negative control: no map, nothing changes.
        assert!(Manifest::parse(&with(GOOD))
            .unwrap()
            .usage
            .unwrap()
            .model_map
            .is_empty());
        let long = "x".repeat(MAX_MODEL_ID + 1);
        for bad in [
            "\"a\" = \"no-provider\"".to_string(),
            "\"a\" = \"/m\"".to_string(),
            "\"a\" = \"p/\"".to_string(),
            "\"a\" = \"p/m/x\"".to_string(),
            "\"a b\" = \"p/m\"".to_string(),
            "\"\" = \"p/m\"".to_string(),
            "\"a\" = \"p/m\\n\"".to_string(),
            format!("\"{long}\" = \"p/m\""),
            format!("\"a\" = \"p/{long}\""),
        ] {
            let text = format!("{}[usage.model_map]\n{bad}\n", with(GOOD));
            assert!(Manifest::parse(&text).is_err(), "{bad}");
        }
        let many: String = (0..=MAX_MODEL_MAP)
            .map(|i| format!("\"m{i}\" = \"p/m{i}\"\n"))
            .collect();
        assert!(Manifest::parse(&format!("{}[usage.model_map]\n{many}", with(GOOD))).is_err());
        let enough: String = (0..MAX_MODEL_MAP)
            .map(|i| format!("\"m{i}\" = \"p/m{i}\"\n"))
            .collect();
        assert!(Manifest::parse(&format!("{}[usage.model_map]\n{enough}", with(GOOD))).is_ok());
    }

    /// ADR-0091: `[quota]` is for a kind with `[usage.file]`, needs `min_engine` 5, and its paths are checked.
    #[test]
    fn a_quota_mapping_is_validated() {
        let m = Manifest::parse(&quota(5, true, QUOTA)).unwrap();
        let w = &m.quota.unwrap().window;
        assert_eq!((w.len(), w[0].percent.as_str()), (1, "a.used"));
        for (bad, why) in [
            (quota(4, true, QUOTA), "min_engine below 5"),
            (quota(5, false, QUOTA), "no usage file"),
            (quota(5, true, ""), "no window"),
            (quota(5, true, &QUOTA.replace("a.used", "")), "empty path"),
            (
                quota(5, true, &QUOTA.replace("a.used", "a..b")),
                "empty key",
            ),
            (
                quota(5, true, &QUOTA.replace("a.used", &"k".repeat(129))),
                "long path",
            ),
            (
                quota(5, true, &format!("{QUOTA}extra = 1\n")),
                "unknown key",
            ),
            (
                quota(5, true, &QUOTA.replace("resets = \"a.at\"\n", "")),
                "missing resets",
            ),
            (quota(5, true, &QUOTA.repeat(5)), "too many windows"),
            (
                quota(5, true, &format!("at = \"\"\n{QUOTA}")),
                "empty at path",
            ),
        ] {
            assert!(Manifest::parse(&bad).is_err(), "{why}");
        }
        assert!(Manifest::parse(&quota(5, true, &QUOTA.repeat(4))).is_ok());
        let with_at =
            Manifest::parse(&quota(5, true, &format!("at = \"timestamp\"\n{QUOTA}"))).unwrap();
        assert_eq!(with_at.quota.unwrap().at.as_deref(), Some("timestamp"));
    }

    #[test]
    fn a_usage_file_needs_a_home_glob_real_paths_and_a_known_format() {
        let m = Manifest::parse(&with(GOOD)).unwrap();
        assert_eq!(m.usage.unwrap().file.unwrap().mode, UsageMode::Delta);
        for bad in [
            GOOD.replace("{home}/", "/etc/"),
            GOOD.replace("{home}/.k", "{home}/../x"),
            GOOD.replace("cwd = \"cwd\"", "cwd = \"\""),
            format!("{GOOD}format = \"csv\"\n"),
            format!("{GOOD}extra = 1\n"),
            GOOD.replace("mode = \"delta\"", "mode = \"sometimes\""),
        ] {
            assert!(Manifest::parse(&with(&bad)).is_err(), "{bad}");
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod session_tests {
    use super::*;

    const BASE: &str = "kind = \"k\"\ndisplay_name = \"K\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"k\"]\n";
    const FILE: &str =
        "[session.file]\nglob = \"{home}/.x/*.jsonl\"\nid = \"payload.id\"\ncwd = \"payload.cwd\"\n";

    #[test]
    fn a_session_spec_names_its_id_one_way_only() {
        let ok = Manifest::parse(&format!(
            "{BASE}[session]\nid_args = [\"--session-id\", \"{{session_id}}\"]\n"
        ))
        .unwrap();
        assert_eq!(ok.session.unwrap().id_args[1], "{session_id}");
        assert!(Manifest::parse(&format!("{BASE}{FILE}")).is_ok());
        for bad in [
            "[session]\nid_args = [\"--session-id\"]\n".to_string(),
            "[session.file]\nglob = \"/etc/*.jsonl\"\nid = \"a\"\ncwd = \"b\"\n".to_string(),
            "[session.file]\nglob = \"{home}/../*.jsonl\"\nid = \"a\"\ncwd = \"b\"\n".to_string(),
            format!("[session]\nid_args = [\"{{session_id}}\"]\n{FILE}"),
        ] {
            assert!(Manifest::parse(&format!("{BASE}{bad}")).is_err(), "{bad}");
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod mcp_tests {
    use super::*;

    fn load(mcp: &str) -> Result<Manifest, ManifestError> {
        Manifest::parse(&format!(
            "kind = \"x\"\ndisplay_name = \"X\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"x\"]\n[mcp]\n{mcp}"
        ))
    }

    #[test]
    fn a_config_file_needs_a_non_empty_path() {
        assert!(load("method = \"config_file\"\npath = \".mcp.json\"\n").is_ok());
        // A present-but-empty path is present but unusable: `inject` would resolve it to the worktree
        // itself and the start would fail late writing a directory. Refused at load, like every other
        // unusable field, so `agora doctor` can name it.
        let e = load("method = \"config_file\"\npath = \"\"\n").unwrap_err();
        assert_eq!(
            e,
            ManifestError::Invalid {
                field: "mcp".into()
            },
            "{e}"
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod continue_tests {
    use super::*;

    fn with(resume: &str) -> String {
        format!("kind = \"k\"\ndisplay_name = \"K\"\nversion = \"1\"\nmin_engine = 2\n[process]\nlaunch = [\"k\"]\n[resume]\n{resume}")
    }

    const ARGV: &str = "argv = [\"k\", \"--resume\", \"{session_id}\"]\n";

    fn good() -> String {
        with(&format!("{ARGV}continue_argv = [\"k\", \"--resume\", \"{{session_id}}\", \"{{continue}}\"]\ncontinue = \"Check the state of what you were doing, then carry on.\"\n"))
    }

    /// `[resume] fresh_argv` starts the session from nothing: it needs a program and `{session_id}`, and takes no message.
    #[test]
    fn a_fresh_start_command_is_read_and_checked() {
        let ok = format!("{ARGV}fresh_argv = [\"k\", \"--session-id\", \"{{session_id}}\"]\n");
        let r = Manifest::parse(&with(&ok)).unwrap().resume.unwrap();
        assert_eq!(r.fresh_argv, ["k", "--session-id", "{session_id}"]);
        assert!(Manifest::parse(&with(ARGV))
            .unwrap()
            .resume
            .unwrap()
            .fresh_argv
            .is_empty());
        for bad in [
            "fresh_argv = [\"k\", \"--session-id\"]",
            "fresh_argv = [\"\", \"{session_id}\"]",
            "fresh_argv = [\"{session_id}\"]",
            "fresh_argv = [\"{x}k\", \"{session_id}\"]",
            "fresh_argv = [\"k\", \"{session_id}\", \"{continue}\"]",
        ] {
            assert!(
                Manifest::parse(&with(&format!("{ARGV}{bad}\n"))).is_err(),
                "{bad}"
            );
        }
    }

    /// ADR-0060 Amendment A: the two fields go together and the sentence is data that cannot act as an option, a placeholder or a second
    /// line; a manifest without them is resumed as before.
    #[test]
    fn a_carry_on_sentence_is_read_and_checked() {
        let m = Manifest::parse(&good()).unwrap();
        let r = m.resume.unwrap();
        assert_eq!(r.continue_argv.last().unwrap(), "{continue}");
        assert!(r.continue_text.unwrap().starts_with("Check the state"));
        // None of it: fine.
        assert!(Manifest::parse(&with(ARGV))
            .unwrap()
            .resume
            .unwrap()
            .continue_argv
            .is_empty());
        let bad = |resume: &str| Manifest::parse(&with(resume)).is_err();
        let argv = "continue_argv = [\"k\", \"--resume\", \"{session_id}\", \"{continue}\"]\n";
        // One without the other.
        assert!(bad(&format!("{ARGV}{argv}")));
        assert!(bad(&format!("{ARGV}continue = \"Carry on.\"\n")));
        // The sentence: an option, a placeholder, a second line, empty, too long.
        for text in [
            "--yolo",
            "Carry {session_id}",
            "Carry on\\nand rm",
            "  ",
            &"x".repeat(MAX_CONTINUE_BYTES + 1),
        ] {
            assert!(
                bad(&format!("{ARGV}{argv}continue = \"{text}\"\n")),
                "{text:?}"
            );
        }
        // The command: no session id, no {continue}, {continue} twice, {continue} glued to another word.
        let t = "continue = \"Carry on.\"\n";
        assert!(bad(&format!(
            "{ARGV}continue_argv = [\"k\", \"{{continue}}\"]\n{t}"
        )));
        assert!(bad(&format!(
            "{ARGV}continue_argv = [\"k\", \"{{session_id}}\"]\n{t}"
        )));
        assert!(bad(&format!("{ARGV}continue_argv = [\"k\", \"{{session_id}}\", \"{{continue}}\", \"{{continue}}\"]\n{t}")));
        assert!(bad(&format!(
            "{ARGV}continue_argv = [\"k\", \"{{session_id}}\", \"say {{continue}}\"]\n{t}"
        )));
        assert!(bad(&format!(
            "{ARGV}continue_argv = [\"\", \"{{session_id}}\", \"{{continue}}\"]\n{t}"
        )));
    }
}
