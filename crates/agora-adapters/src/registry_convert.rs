//! ADR-0092: one ACP registry `agent.json` becomes one L2 manifest and one ADR-0087 package entry, by a pure conversion.
//! Nothing here reads the network or the disk, and nodes never call it: it runs in the repository (see
//! `examples/registry_convert.rs`) and its output is reviewed before it ships. The registry is unsigned third-party input, so
//! every name and argument must be a plain token, `env` values are never copied, and a distribution this does not understand
//! is skipped with a reason code, never guessed.

use serde_json::{json, Map, Value};

use crate::package::valid_name;
use crate::perm::says_it_skips;

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
    ForbiddenExecutable,
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
            Reason::ForbiddenExecutable => "registry.forbidden_executable",
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

/// Display text (name, licence, repository): no control, format, bidirectional, separator or private-use character (ADR-0057 list).
fn clean_text(s: &str) -> bool {
    !s.chars()
        .any(|c| c.is_control() || agora_core::text::is_hidden(c))
}

/// An npm package name: `name` or `@scope/name`, lower-case letters, digits, `.`, `_`, `-`; no URL, path, alias or tag syntax.
fn npm_name(n: &str) -> bool {
    let part = |p: &str| {
        !p.is_empty()
            && !p.starts_with(['.', '_', '-'])
            && p.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "._-".contains(c))
    };
    match n.strip_prefix('@') {
        Some(rest) => rest
            .split_once('/')
            .is_some_and(|(a, b)| part(a) && part(b)),
        None => part(n),
    }
}

/// A PyPI project name: letters, digits, `.`, `_`, `-`.
fn pypi_name(n: &str) -> bool {
    !n.is_empty()
        && !n.starts_with(['.', '_', '-'])
        && n.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

fn exact_version(v: &str) -> bool {
    let core = v.split(['-', '+']).next().unwrap_or("");
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-+".contains(c))
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// Words (squashed) that mean an argument approves or trusts without asking, beyond what `perm::says_it_skips` knows.
const EXTRA_SKIP_WORDS: &[&str] = &[
    "approve",
    "fullauto",
    "alwaysallow",
    "trust",
    "autorun",
    "force",
];

/// Whether an argument, as a whole or in any of its words (split on `-`, `=`, `_`, `/`, `:`, `.`), says the agent skips permissions:
/// `--auto-approve`, `--full-auto`, `--allow-all-tools`, `--approval-mode=yolo`, `--permission-mode bypassPermissions`, a bare `yolo`.
fn skips_permissions_arg(a: &str) -> bool {
    let words = a
        .split(|c: char| "-=_/:.,".contains(c))
        .filter(|w| !w.is_empty());
    std::iter::once(a).chain(words).any(|w| {
        let squashed: String = w
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .flat_map(char::to_lowercase)
            .collect();
        says_it_skips(w, w, "", true) || EXTRA_SKIP_WORDS.iter().any(|x| squashed.contains(x))
    })
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
        if skips_permissions_arg(a) {
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
    if !npm_name(name) {
        return Err(Reason::BadToken);
    }
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
    if !pypi_name(name) {
        return Err(Reason::BadToken);
    }
    if !exact_version(version) {
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

/// A shell or interpreter would run whatever its arguments say, so a registry entry may not name one as its executable.
fn forbidden_executable(exe: &str) -> bool {
    let e = exe.to_ascii_lowercase();
    let e = e.strip_suffix(".bat").unwrap_or(&e);
    [
        "sh",
        "bash",
        "zsh",
        "dash",
        "fish",
        "ksh",
        "csh",
        "tcsh",
        "cmd",
        "powershell",
        "pwsh",
        "node",
        "nodejs",
        "perl",
        "ruby",
        "env",
        "deno",
        "bun",
        "php",
        "lua",
        "wscript",
        "cscript",
        "busybox",
    ]
    .contains(&e)
        || e.starts_with("python")
        || e.starts_with("pypy")
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
        let drive = cmd.chars().nth(1) == Some(':');
        if !plain_token(&cmd.replace('\\', "/"))
            || cmd.contains("..")
            || cmd.contains(':')
            || drive
            || cmd.starts_with(['/', '\\'])
        {
            return Err(Reason::BadToken);
        }
        let file = cmd.rsplit(['/', '\\']).next().unwrap_or(cmd);
        let exe = file
            .strip_suffix(".exe")
            .or_else(|| file.strip_suffix(".cmd"))
            .unwrap_or(file)
            .to_string();
        if forbidden_executable(&exe) {
            return Err(Reason::ForbiddenExecutable);
        }
        let a = args(p)?;
        match &command {
            None => command = Some((exe, a.clone())),
            Some((e, ea)) if *e == exe && *ea == a => {}
            Some(_) => return Err(Reason::InconsistentCommand),
        }
        // A hash that is there but is not 64 hex digits is a malformed entry, not a missing hash.
        let sha = match text(p, "sha256") {
            Some(s) if s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()) => {
                Some(s.to_ascii_lowercase())
            }
            Some(_) => return Err(Reason::BadField),
            None if p.get("sha256").is_some() => return Err(Reason::BadField),
            None => None,
        };
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
        || !clean_text(name)
        || !plain_token(version)
        || !clean_text(version)
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
        if let Some(s) = text(&v, k) {
            if !clean_text(s) {
                return Err(Reason::BadField);
            }
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
