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

/// How the node hands an agent its own mesh MCP server (docs/specs/agent-manifest.md `[mcp]`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpSpec {
    pub method: McpMethod,
    /// `config_file`/`cli_flag`: where to write the config, inside the agent's worktree.
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

/// A parsed, validated manifest. Sections the engine does not interpret yet (accounts, hooks, usage,
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
}

fn invalid(field: impl Into<String>) -> ManifestError {
    ManifestError::Invalid {
        field: field.into(),
    }
}

impl Manifest {
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
        if let Some(m) = &self.mcp {
            let ok = match m.method {
                McpMethod::ConfigFile => m.path.is_some(),
                McpMethod::CliFlag => m.path.is_some() && !m.flag.is_empty(),
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
