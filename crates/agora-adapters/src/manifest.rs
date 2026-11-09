//! L1 agent manifests: parsing and validation (docs/specs/agent-manifest.md).

use std::collections::{BTreeMap, HashSet};

use serde::Deserialize;

/// Highest manifest `min_engine` this engine can run.
pub const ENGINE_VERSION: u32 = 1;

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

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct Process {
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

/// `[structured]`: the ways an agent can be driven without a terminal. Keys this engine does not know yet (`adapter` as a registry
/// name) are accepted and ignored.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Structured {
    pub acp: Option<AcpSpec>,
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

/// `[usage]`: other keys (`quota_profiles`, `model_map`) are accepted and not read yet.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct UsageSpec {
    pub source: Option<String>,
    pub file: Option<UsageFile>,
}

/// A parsed, validated manifest. Sections the engine does not interpret yet (accounts, hooks,
/// structured) are accepted and ignored so newer manifests still load.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Manifest {
    pub kind: String,
    pub display_name: String,
    pub version: String,
    pub min_engine: u32,
    #[serde(default)]
    pub process: Process,
    pub resume: Option<Resume>,
    #[serde(default)]
    pub rules: Vec<Rule>,
    pub mcp: Option<McpSpec>,
    pub adapter: Option<AdapterSpec>,
    pub structured: Option<Structured>,
    pub usage: Option<UsageSpec>,
}

fn invalid(field: impl Into<String>) -> ManifestError {
    ManifestError::Invalid {
        field: field.into(),
    }
}

impl Manifest {
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
        let kind_ok = !self.kind.is_empty()
            && self
                .kind
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !kind_ok {
            return Err(invalid("kind"));
        }
        if self.display_name.trim().is_empty() {
            return Err(invalid("display_name"));
        }
        if self
            .adapter
            .as_ref()
            .is_some_and(|a| a.command.first().is_none_or(String::is_empty))
        {
            return Err(invalid("adapter.command"));
        }
        if self
            .structured
            .as_ref()
            .and_then(|s| s.acp.as_ref())
            .is_some_and(|a| a.argv.first().is_none_or(String::is_empty))
        {
            return Err(invalid("structured.acp.argv"));
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
        if let Some(m) = &self.mcp {
            let ok = match m.method {
                McpMethod::ConfigFile => m.path.is_some(),
                McpMethod::CliFlag => !m.flag.is_empty(),
                McpMethod::Env => !m.env.is_empty(),
            };
            if !ok || m.format.as_deref().is_some_and(|f| f != "json") {
                return Err(invalid("mcp"));
            }
        }
        let mut ids = HashSet::new();
        for (i, r) in self.rules.iter().enumerate() {
            let at = |f: &str| invalid(format!("rules[{i}].{f}"));
            if r.id.is_empty() || !ids.insert(r.id.as_str()) {
                return Err(at("id"));
            }
            crate::rules::Region::parse(&r.region).ok_or_else(|| at("region"))?;
            let own = r.contains.is_some() || r.regex.is_some();
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
mod usage_tests {
    use super::*;

    fn with(file: &str) -> String {
        format!(
            "kind = \"k\"\ndisplay_name = \"K\"\nversion = \"1\"\nmin_engine = 1\n[usage]\nsource = \"file\"\n[usage.file]\n{file}"
        )
    }

    const GOOD: &str = "glob = \"{home}/.k/*/*.jsonl\"\ncwd = \"cwd\"\nmodel = \"m\"\ninput = \"i\"\noutput = \"o\"\nmode = \"delta\"\n";

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
