//! ADR-0092: one ACP registry `agent.json` becomes one L2 manifest and one ADR-0087 package entry, by a pure conversion.
//! Nothing here reads the network or the disk, and nodes never call it: it runs in the repository (see
//! `examples/registry_convert.rs`) and its output is reviewed before it ships. The registry is unsigned third-party input, so
//! every name and argument must be a plain token, `env` values are never copied, and a distribution this does not understand
//! is skipped with a reason code, never guessed.

use serde_json::{json, Map, Value};

use crate::manifest::skips_permissions_flag;
use crate::package::valid_name;

/// First line of every generated file.
pub const GENERATED: &str = "Generated from the ACP registry by agora-adapters registry_convert (ADR-0092). Do not edit; reviewed before it ships.";

/// Why an entry, or one of its distributions, was not converted. `code` is stable; the window words come from `locales/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    NotJson,
    BadId,
    BadField,
    NoSupportedDistribution,
    BadToken,
    UnpinnedVersion,
    SkipsPermissions,
    InsecureUrl,
    InconsistentCommand,
}

impl Reason {
    pub fn code(self) -> &'static str {
        match self {
            Reason::NotJson => "registry.not_json",
            Reason::BadId => "registry.bad_id",
            Reason::BadField => "registry.bad_field",
            Reason::NoSupportedDistribution => "registry.no_supported_distribution",
            Reason::BadToken => "registry.bad_token",
            Reason::UnpinnedVersion => "registry.unpinned_version",
            Reason::SkipsPermissions => "registry.skips_permissions",
            Reason::InsecureUrl => "registry.insecure_url",
            Reason::InconsistentCommand => "registry.inconsistent_command",
        }
    }
}

/// A distribution that was left out of an otherwise converted entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub distribution: &'static str,
    pub reason: Reason,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Converted {
    /// The Agora kind: `registry-<id>`, never a built-in's name, so a converted entry cannot shadow one.
    pub kind: String,
    /// The L2 manifest, TOML, starting with the generated header.
    pub manifest: String,
    /// The ADR-0087 package entry (JSON), starting with a `_generated` field.
    pub package: Value,
    pub notes: Vec<Note>,
}

/// A name, version, path or argument with no shell syntax, spaces or control characters.
fn plain_token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 200
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "@._/:=+~^,-".contains(c))
}

fn exact_version(v: &str) -> bool {
    let core = v.split(['-', '+']).next().unwrap_or("");
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
        && plain_token(v)
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// The `args` of a distribution: plain tokens, none that skips permissions.
fn args(d: &Value) -> Result<Vec<String>, Reason> {
    let mut out = Vec::new();
    for a in d
        .get("args")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let a = a.as_str().ok_or(Reason::BadToken)?;
        if !plain_token(a) {
            return Err(Reason::BadToken);
        }
        if skips_permissions_flag(a) {
            return Err(Reason::SkipsPermissions);
        }
        out.push(a.to_string());
    }
    Ok(out)
}

/// Names of the `env` variables a distribution sets. Values are never copied.
fn env_names(d: &Value) -> Vec<String> {
    let mut names: Vec<String> = d
        .get("env")
        .and_then(Value::as_object)
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    names.sort();
    names
}

struct Dist {
    argv: Vec<String>,
    installer: Value,
}

fn convert_npx(d: &Value, entry_version: &str) -> Result<Dist, Reason> {
    let pkg = text(d, "package").ok_or(Reason::BadField)?;
    if !plain_token(pkg) || pkg.starts_with('-') {
        return Err(Reason::BadToken);
    }
    // `name@version` or `@scope/name@version`; a bare name takes the entry's version.
    let (name, version) = match pkg.rfind('@') {
        Some(i) if i > 0 => (&pkg[..i], &pkg[i + 1..]),
        _ => (pkg, entry_version),
    };
    if !exact_version(version) {
        return Err(Reason::UnpinnedVersion);
    }
    let args = args(d)?;
    let mut argv = vec![
        "npx".to_string(),
        "--yes".to_string(),
        format!("{name}@{version}"),
    ];
    argv.extend(args.iter().cloned());
    Ok(Dist {
        argv,
        installer: json!({
            "kind": "npx", "package": name, "version": version, "args": args,
            "env_names_not_copied": env_names(d),
            "installable": false, "not_installable_reason": "no_integrity_hash",
        }),
    })
}

fn convert_uvx(d: &Value) -> Result<Dist, Reason> {
    let pkg = text(d, "package").ok_or(Reason::BadField)?;
    if !plain_token(pkg) || pkg.starts_with('-') {
        return Err(Reason::BadToken);
    }
    let (name, version) = pkg.split_once("==").ok_or(Reason::UnpinnedVersion)?;
    if name.is_empty() || !exact_version(version) {
        return Err(Reason::UnpinnedVersion);
    }
    let args = args(d)?;
    let mut argv = vec!["uvx".to_string(), format!("{name}=={version}")];
    argv.extend(args.iter().cloned());
    Ok(Dist {
        argv,
        installer: json!({
            "kind": "uvx", "package": name, "version": version, "args": args,
            "env_names_not_copied": env_names(d),
            "installable": false, "not_installable_reason": "no_integrity_hash",
        }),
    })
}

fn convert_binary(d: &Value) -> Result<Dist, Reason> {
    let platforms = d.as_object().ok_or(Reason::BadField)?;
    let mut out = Map::new();
    let mut command: Option<(String, Vec<String>)> = None;
    let mut all_hashed = !platforms.is_empty();
    for (platform, p) in platforms {
        if !plain_token(platform) {
            return Err(Reason::BadToken);
        }
        let url = text(p, "archive").ok_or(Reason::BadField)?;
        if !url.starts_with("https://") {
            return Err(Reason::InsecureUrl);
        }
        if url
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '"' || c == '\'')
        {
            return Err(Reason::BadToken);
        }
        let cmd = text(p, "cmd").ok_or(Reason::BadField)?;
        // Windows commands use a backslash and an extension; the executable name is the same word without them.
        if !plain_token(&cmd.replace('\\', "/")) || cmd.contains("..") {
            return Err(Reason::BadToken);
        }
        let file = cmd.rsplit(['/', '\\']).next().unwrap_or(cmd);
        let exe = file
            .strip_suffix(".exe")
            .or_else(|| file.strip_suffix(".cmd"))
            .unwrap_or(file)
            .to_string();
        let a = args(p)?;
        match &command {
            None => command = Some((exe, a.clone())),
            Some((e, ea)) if *e == exe && *ea == a => {}
            Some(_) => return Err(Reason::InconsistentCommand),
        }
        let sha = text(p, "sha256")
            .filter(|s| s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()))
            .map(str::to_ascii_lowercase);
        all_hashed &= sha.is_some();
        out.insert(
            platform.clone(),
            json!({ "url": url, "cmd": cmd, "sha256": sha, "env_names_not_copied": env_names(p) }),
        );
    }
    let (exe, a) = command.ok_or(Reason::BadField)?;
    let mut argv = vec![exe.clone()];
    argv.extend(a.iter().cloned());
    let mut installer = json!({
        "kind": "binary-download", "provides": [exe], "args": a, "platforms": out,
        "installable": all_hashed,
    });
    if !all_hashed {
        installer["not_installable_reason"] = json!("missing_sha256");
    }
    Ok(Dist { argv, installer })
}

/// Convert the text of one registry `agent.json`.
pub fn convert(src: &str) -> Result<Converted, Reason> {
    let v: Value = serde_json::from_str(src).map_err(|_| Reason::NotJson)?;
    let id = text(&v, "id").ok_or(Reason::BadId)?;
    let kind = format!("registry-{id}");
    if !valid_name(id) || !valid_name(&kind) {
        return Err(Reason::BadId);
    }
    let name = text(&v, "name").ok_or(Reason::BadField)?;
    let version = text(&v, "version").ok_or(Reason::BadField)?;
    if name.is_empty()
        || name.len() > 100
        || name.chars().any(char::is_control)
        || !plain_token(version)
    {
        return Err(Reason::BadField);
    }
    let dist = v.get("distribution").ok_or(Reason::BadField)?;
    let mut notes = Vec::new();
    let mut chosen: Option<Vec<String>> = None;
    let mut installers = Vec::new();
    // Order of preference for what the manifest runs: npx, uvx, binary.
    for kind_name in ["npx", "uvx", "binary"] {
        let Some(d) = dist.get(kind_name) else {
            continue;
        };
        let r = match kind_name {
            "npx" => convert_npx(d, version),
            "uvx" => convert_uvx(d),
            _ => convert_binary(d),
        };
        match r {
            Ok(c) => {
                chosen.get_or_insert(c.argv);
                installers.push(c.installer);
            }
            Err(reason) => notes.push(Note {
                distribution: kind_name,
                reason,
            }),
        }
    }
    let Some(argv) = chosen else {
        // Unknown distribution kinds are not guessed at; a known one that failed keeps its own reason.
        return Err(notes
            .first()
            .map_or(Reason::NoSupportedDistribution, |n| n.reason));
    };
    let quoted = |s: &str| Value::String(s.to_string()).to_string();
    let argv_toml = argv
        .iter()
        .map(|a| quoted(a))
        .collect::<Vec<_>>()
        .join(", ");
    let display = toml::Value::String(name.to_string()).to_string();
    let manifest = format!(
        "# {GENERATED}\n# Registry id {id}, version {version}. Untested in Agora until someone runs it.\nkind = {}\ndisplay_name = {display}\nversion = {}\nmin_engine = 1\n\n[structured]\nacp = {{ argv = [{argv_toml}] }}\n",
        toml::Value::String(kind.clone()),
        toml::Value::String(version.to_string()),
    );
    let mut source = Map::new();
    for k in [
        "id",
        "version",
        "repository",
        "website",
        "license",
        "license_url",
    ] {
        if let Some(s) = text(&v, k).filter(|s| !s.chars().any(char::is_control)) {
            source.insert(k.to_string(), json!(s));
        }
    }
    let package = json!({
        "_generated": GENERATED,
        "name": kind,
        "display": name,
        "tested": false,
        "source": source,
        "installers": installers,
        "skipped": notes.iter().map(|n| json!({"distribution": n.distribution, "reason": n.reason.code()})).collect::<Vec<_>>(),
    });
    Ok(Converted {
        kind,
        manifest,
        package,
        notes,
    })
}
