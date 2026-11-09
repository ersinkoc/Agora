//! Giving an agent its own mesh MCP server at launch (docs/specs/agent-manifest.md `[mcp]`, ADR-0008). Pure: turns the
//! manifest's `[mcp]` section plus what the node serves into a plan (files to write, arguments to append, variables to
//! set). The supervisor carries the plan out. Agents are data, not code: how a given agent wants its MCP server
//! described is in its manifest, not here.
//!
//! Placeholders usable in `path`, `flag` and `env` values: `{worktree}`, `{socket}` (this agent's mesh socket),
//! `{mcp_command}` (the program that serves it) and `{config_file}` (the file written for `config_file`/`cli_flag`). The
//! `_toml` forms, `{socket_toml}` and `{mcp_command_toml}`, are the same values as quoted TOML strings, for agents that take
//! config overrides as `key=value` flags (Codex's `-c mcp_servers.NAME.command=...`).
//!
//! Safety (threat T6, a third-party manifest): files are only ever written **inside the agent's worktree**. A path
//! that is absolute outside it, or that climbs out with `..`, is refused, so a manifest cannot make the node write
//! into the user's home or config directories.

use std::fmt;
use std::path::{Component, Path, PathBuf};

use serde_json::json;

use crate::manifest::{McpMethod, McpSpec};

/// What the node serves for this agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// The program an MCP client runs to talk to the endpoint (`agora`).
    pub command: PathBuf,
    /// The agent's own socket.
    pub socket: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Plan {
    /// `(path inside the worktree, contents)`.
    pub files: Vec<(PathBuf, String)>,
    pub argv_extra: Vec<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InjectError {
    /// A placeholder this module does not know.
    UnknownPlaceholder(String),
    /// A path that would leave the worktree.
    OutsideWorktree(String),
    /// The spec needs something it does not have (checked at manifest load; this is the second line of defence).
    Incomplete,
}

impl fmt::Display for InjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownPlaceholder(p) => write!(f, "unknown placeholder {{{p}}}"),
            Self::OutsideWorktree(p) => write!(f, "{p} is outside the worktree"),
            Self::Incomplete => write!(f, "incomplete [mcp] section"),
        }
    }
}

impl std::error::Error for InjectError {}

fn expand(template: &str, vars: &[(&str, String)]) -> Result<String, InjectError> {
    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            // A lone brace is literal text.
            out.push_str(&rest[open..]);
            return Ok(out);
        };
        let name = &after[..close];
        match vars.iter().find(|(k, _)| *k == name) {
            Some((_, v)) => out.push_str(v),
            None => return Err(InjectError::UnknownPlaceholder(name.to_string())),
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// `path` resolved against the worktree, refusing anything that ends up outside it.
fn inside(worktree: &Path, path: &str) -> Result<PathBuf, InjectError> {
    let p = Path::new(path);
    let full = if p.is_absolute() {
        p.to_path_buf()
    } else {
        worktree.join(p)
    };
    // Lexical check: no `..` anywhere (a path that merely contains one is refused even if it would come back).
    if full.components().any(|c| matches!(c, Component::ParentDir)) || !full.starts_with(worktree) {
        return Err(InjectError::OutsideWorktree(path.to_string()));
    }
    Ok(full)
}

/// The JSON most MCP-capable agents read: `{ "mcpServers": { name: { command, args } } }`.
fn json_config(ep: &Endpoint) -> String {
    let cfg = json!({
        "mcpServers": {
            "agora-mesh": {
                "command": ep.command.to_string_lossy(),
                "args": ["mesh", "stdio", "--socket", ep.socket.to_string_lossy()],
            }
        }
    });
    // Pretty: people read these files to see what the node did.
    serde_json::to_string_pretty(&cfg).unwrap_or_default()
}

/// `s` as a TOML basic string, quotes included.
fn toml_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn plan(spec: &McpSpec, worktree: &Path, ep: &Endpoint) -> Result<Plan, InjectError> {
    let mut vars: Vec<(&str, String)> = vec![
        ("worktree", worktree.to_string_lossy().into_owned()),
        ("socket", ep.socket.to_string_lossy().into_owned()),
        ("mcp_command", ep.command.to_string_lossy().into_owned()),
        ("socket_toml", toml_string(&ep.socket.to_string_lossy())),
        (
            "mcp_command_toml",
            toml_string(&ep.command.to_string_lossy()),
        ),
    ];
    let mut plan = Plan::default();
    // `config_file` always writes its file; `cli_flag` writes one only when the manifest names a path (an agent that takes the
    // server as flags has none, and `{config_file}` is then an unknown placeholder).
    let wants_file = match spec.method {
        McpMethod::ConfigFile => true,
        McpMethod::CliFlag => spec.path.is_some(),
        McpMethod::Env => false,
    };
    if wants_file {
        let path = spec.path.as_deref().ok_or(InjectError::Incomplete)?;
        let file = inside(worktree, &expand(path, &vars)?)?;
        vars.push(("config_file", file.to_string_lossy().into_owned()));
        plan.files.push((file, json_config(ep)));
    }
    match spec.method {
        McpMethod::ConfigFile => {}
        McpMethod::CliFlag => {
            if spec.flag.is_empty() {
                return Err(InjectError::Incomplete);
            }
            for a in &spec.flag {
                plan.argv_extra.push(expand(a, &vars)?);
            }
        }
        McpMethod::Env => {
            if spec.env.is_empty() {
                return Err(InjectError::Incomplete);
            }
            for (k, v) in &spec.env {
                plan.env.push((k.clone(), expand(v, &vars)?));
            }
        }
    }
    Ok(plan)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::manifest::Manifest;

    fn ep() -> Endpoint {
        Endpoint {
            command: "/usr/bin/agora".into(),
            socket: "/home/u/.agora/m/agent-1.sock".into(),
        }
    }

    fn spec(toml: &str) -> McpSpec {
        let src = format!(
            "kind = \"x\"\ndisplay_name = \"X\"\nversion = \"1\"\nmin_engine = 1\n[mcp]\n{toml}"
        );
        Manifest::parse(&src).unwrap().mcp.unwrap()
    }

    #[test]
    fn config_file_writes_the_standard_json_inside_the_worktree() {
        let s = spec("method = \"config_file\"\npath = \".mcp.json\"\nformat = \"json\"\n");
        let p = plan(&s, Path::new("/work/wt"), &ep()).unwrap();
        assert_eq!(p.files.len(), 1);
        assert_eq!(p.files[0].0, Path::new("/work/wt/.mcp.json"));
        let v: serde_json::Value = serde_json::from_str(&p.files[0].1).unwrap();
        assert_eq!(v["mcpServers"]["agora-mesh"]["command"], "/usr/bin/agora");
        assert_eq!(
            v["mcpServers"]["agora-mesh"]["args"],
            json!(["mesh", "stdio", "--socket", "/home/u/.agora/m/agent-1.sock"])
        );
        assert!(p.argv_extra.is_empty() && p.env.is_empty());
    }

    #[test]
    fn cli_flag_appends_arguments_pointing_at_the_written_file() {
        let s = spec("method = \"cli_flag\"\npath = \"{worktree}/.agora/mcp.json\"\nflag = [\"--mcp-config\", \"{config_file}\"]\n");
        let p = plan(&s, Path::new("/work/wt"), &ep()).unwrap();
        assert_eq!(p.argv_extra, ["--mcp-config", "/work/wt/.agora/mcp.json"]);
        assert_eq!(p.files[0].0, Path::new("/work/wt/.agora/mcp.json"));
    }

    #[test]
    fn cli_flag_without_a_path_writes_no_file_and_can_pass_the_server_as_toml_overrides() {
        let s = spec("method = \"cli_flag\"\nflag = [\"-c\", \"mcp_servers.agora-mesh.command={mcp_command_toml}\", \"-c\", \"mcp_servers.agora-mesh.args=[\\\"mesh\\\",\\\"stdio\\\",\\\"--socket\\\",{socket_toml}]\"]\n");
        let p = plan(&s, Path::new("/work/wt"), &ep()).unwrap();
        assert!(p.files.is_empty());
        assert_eq!(
            p.argv_extra,
            [
                "-c",
                "mcp_servers.agora-mesh.command=\"/usr/bin/agora\"",
                "-c",
                "mcp_servers.agora-mesh.args=[\"mesh\",\"stdio\",\"--socket\",\"/home/u/.agora/m/agent-1.sock\"]"
            ]
        );
        // `{config_file}` needs a file; without a path it is an unknown placeholder, not an empty string.
        let bad = spec("method = \"cli_flag\"\nflag = [\"{config_file}\"]\n");
        assert!(plan(&bad, Path::new("/w"), &ep()).is_err());
    }

    #[test]
    fn toml_strings_escape_quotes_backslashes_and_control_characters() {
        assert_eq!(toml_string("a\"b\\c\nd"), "\"a\\\"b\\\\c\\nd\"");
        assert_eq!(toml_string("C:\\x"), "\"C:\\\\x\"");
    }

    #[test]
    fn env_sets_variables_with_placeholders_and_writes_nothing() {
        let s = spec("method = \"env\"\n[mcp.env]\nAGORA_MCP_SOCKET = \"{socket}\"\nAGORA_MCP = \"{mcp_command} mesh stdio\"\n");
        let p = plan(&s, Path::new("/w"), &ep()).unwrap();
        assert!(p.files.is_empty());
        assert!(p.env.contains(&(
            "AGORA_MCP_SOCKET".into(),
            "/home/u/.agora/m/agent-1.sock".into()
        )));
        assert!(p
            .env
            .contains(&("AGORA_MCP".into(), "/usr/bin/agora mesh stdio".into())));
    }

    #[test]
    fn a_manifest_cannot_make_the_node_write_outside_the_worktree() {
        for path in [
            "../escape.json",
            "/etc/cron.d/x",
            "a/../../b",
            "{worktree}/../x",
            "/work/wt-evil/x",
        ] {
            let s = spec(&format!("method = \"config_file\"\npath = \"{path}\"\n"));
            assert!(
                matches!(
                    plan(&s, Path::new("/work/wt"), &ep()),
                    Err(InjectError::OutsideWorktree(_))
                ),
                "{path}"
            );
        }
        // An absolute path that is inside the worktree is fine.
        let s = spec("method = \"config_file\"\npath = \"/work/wt/.mcp.json\"\n");
        assert!(plan(&s, Path::new("/work/wt"), &ep()).is_ok());
    }

    #[test]
    fn unknown_placeholders_are_errors_and_lone_braces_are_text() {
        let s = spec("method = \"config_file\"\npath = \"{config_dir}/mcp.json\"\n");
        assert_eq!(
            plan(&s, Path::new("/w"), &ep()),
            Err(InjectError::UnknownPlaceholder("config_dir".into()))
        );
        let s = spec("method = \"env\"\n[mcp.env]\nX = \"a{b\"\n");
        assert_eq!(
            plan(&s, Path::new("/w"), &ep()).unwrap().env,
            [("X".into(), "a{b".into())]
        );
    }

    #[test]
    fn incomplete_sections_are_refused_when_the_manifest_loads() {
        let load = |mcp: &str| {
            Manifest::parse(&format!(
                "kind = \"x\"\ndisplay_name = \"X\"\nversion = \"1\"\nmin_engine = 1\n[mcp]\n{mcp}"
            ))
        };
        assert!(load("method = \"config_file\"\n").is_err(), "needs a path");
        assert!(
            load("method = \"cli_flag\"\npath = \"a\"\n").is_err(),
            "needs flag"
        );
        assert!(load("method = \"env\"\n").is_err(), "needs env");
        assert!(
            load("method = \"config_file\"\npath = \"a\"\nformat = \"toml\"\n").is_err(),
            "json only"
        );
        assert!(load("method = \"teleport\"\n").is_err());
        assert!(
            load("method = \"config_file\"\npath = \"a\"\nextra = 1\n").is_err(),
            "unknown field"
        );
    }
}
