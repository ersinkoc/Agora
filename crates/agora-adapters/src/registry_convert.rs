//! ADR-0092: one ACP registry `agent.json` becomes one L2 manifest and one ADR-0087 package entry, by a pure conversion.
//! Nothing here reads the network or the disk, and nodes never call it: it runs in the repository (see
//! `examples/registry_convert.rs`) and its output is reviewed before it ships. The registry is unsigned third-party input, so
//! every name and argument must be a plain token, `env` values are never copied, and a distribution this does not understand
//! is skipped with a reason code, never guessed.

use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

use crate::agent_packages::https_url;
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
    /// A `repository`, `website` or `license_url` that is not an `https://` address a vendor could serve from.
    BadUrl,
    /// The registry itself holds the entry back (its `quarantine.json`).
    Quarantined,
    /// The icon is not a plain drawing (see [`check_icon`]).
    BadIcon,
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
            Reason::BadUrl => "registry.bad_url",
            Reason::Quarantined => "registry.quarantined",
            Reason::BadIcon => "registry.bad_icon",
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
pub(crate) fn clean_text(s: &str) -> bool {
    !s.chars()
        .any(|c| c.is_control() || agora_core::text::is_hidden(c))
}

/// An npm package name: `name` or `@scope/name`, lower-case letters, digits, `.`, `_`, `-`; no URL, path, alias or tag syntax.
pub(crate) fn npm_name(n: &str) -> bool {
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

pub(crate) fn exact_version(v: &str) -> bool {
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
    // `name==version`, or uvx's own `name@version` (the same exact pin); the manifest always says `==`.
    let (name, version) = pkg
        .split_once("==")
        .or_else(|| pkg.split_once('@'))
        .ok_or(Reason::UnpinnedVersion)?;
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
pub(crate) fn forbidden_executable(exe: &str) -> bool {
    // Whatever extension a platform runs it by is the same program: `node.exe`, `bash.exe`, `powershell.exe -c` are a shell too.
    let mut e = exe.to_ascii_lowercase();
    while let Some(bare) = [".exe", ".cmd", ".bat", ".ps1", ".com"]
        .iter()
        .find_map(|x| e.strip_suffix(x))
    {
        e = bare.to_string();
    }
    let e = e.as_str();
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

/// A `binary` distribution. Every platform must give the same arguments. When every platform's program has the same name (an
/// extension and a folder aside), the manifest runs that name; when they differ (`tool-linux-amd64`, `tool-windows-arm64.exe`), it runs
/// the registry id, the one name the installer leaves on the path for each of them (`provides`).
fn convert_binary(d: &Value, id: &str) -> Result<Dist, Reason> {
    let platforms = d.as_object().ok_or(Reason::BadField)?;
    let mut out = Map::new();
    let mut names: Vec<String> = Vec::new();
    let mut command: Option<Vec<String>> = None;
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
            None => command = Some(a.clone()),
            Some(ea) if *ea == a => {}
            Some(_) => return Err(Reason::InconsistentCommand),
        }
        if !names.contains(&exe) {
            names.push(exe);
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
    let a = command.ok_or(Reason::BadField)?;
    let exe = match names.as_slice() {
        [one] => one.clone(),
        _ => id.to_string(),
    };
    if forbidden_executable(&exe) {
        return Err(Reason::ForbiddenExecutable);
    }
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
            _ => convert_binary(d, id),
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
            if !clean_text(s) || (k == "license" && (s.is_empty() || s.chars().count() > 64)) {
                return Err(Reason::BadField);
            }
            // The window turns these into links: an `https://` address only, never `javascript:`, a plain-HTTP or a local one.
            if ["repository", "website", "license_url"].contains(&k) && https_url(s).is_err() {
                return Err(Reason::BadUrl);
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

/// The registry's `quarantine.json`: an object from agent id to the registry's own short reason. The conversion skips those ids
/// (`registry.quarantined`): the registry itself does not offer them. Anything but an object of valid ids to text is refused whole.
pub fn quarantine_list(src: &str) -> Result<BTreeMap<String, String>, Reason> {
    let v: Value = serde_json::from_str(src).map_err(|_| Reason::NotJson)?;
    let obj = v.as_object().ok_or(Reason::BadField)?;
    let mut out = BTreeMap::new();
    for (id, why) in obj {
        let why = why.as_str().ok_or(Reason::BadField)?;
        if !valid_name(id) {
            return Err(Reason::BadId);
        }
        out.insert(
            id.clone(),
            why.chars().filter(|c| !c.is_control()).collect(),
        );
    }
    Ok(out)
}

/// What the window's list needs of one converted entry (`manifests/agents/registry/catalog.json`), joined to a machine's kinds by
/// `kind`. `runs` is how the manifest starts it (`npx`, `uvx` or `binary`); `icon` says whether `icons/<id>.svg` was vendored.
pub fn catalog_entry(c: &Converted, icon: bool) -> Value {
    let src = &c.package["source"];
    let runs = c.package["installers"]
        .as_array()
        .and_then(|i| i.first())
        .and_then(|i| i["kind"].as_str())
        .map_or(
            "binary",
            |k| if k == "binary-download" { "binary" } else { k },
        );
    let mut line = json!({
        "kind": c.kind, "id": src["id"], "name": c.package["display"], "version": src["version"],
        "runs": runs, "icon": icon, "tested": false,
    });
    for k in ["license", "repository", "website"] {
        if let Some(s) = src[k].as_str() {
            line[k] = json!(s);
        }
    }
    line
}

/// Largest icon accepted, in bytes.
pub const MAX_ICON_BYTES: usize = 16 * 1024;

/// SVG elements an icon may use: shapes, groups, gradients, clipping and text. No script, style, image, link, foreign object or
/// animation.
const ICON_ELEMENTS: &[&str] = &[
    "svg",
    "g",
    "path",
    "circle",
    "ellipse",
    "line",
    "rect",
    "polygon",
    "polyline",
    "title",
    "desc",
    "text",
    "tspan",
    "defs",
    "linearGradient",
    "radialGradient",
    "stop",
    "clipPath",
    "mask",
    "use",
];

/// Attributes an icon may use: geometry and paint. No event handler and no `style`; `href` only to a local `#id`.
const ICON_ATTRIBUTES: &[&str] = &[
    "xmlns",
    "xmlns:xlink",
    "version",
    "viewBox",
    "width",
    "height",
    "x",
    "y",
    "x1",
    "y1",
    "x2",
    "y2",
    "cx",
    "cy",
    "r",
    "rx",
    "ry",
    "fx",
    "fy",
    "d",
    "points",
    "fill",
    "fill-rule",
    "fill-opacity",
    "clip-rule",
    "clip-path",
    "mask",
    "stroke",
    "stroke-width",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-opacity",
    "stroke-miterlimit",
    "stroke-dasharray",
    "stroke-dashoffset",
    "opacity",
    "transform",
    "id",
    "class",
    "preserveAspectRatio",
    "overflow",
    "font-size",
    "font-family",
    "font-weight",
    "font-style",
    "text-anchor",
    "dominant-baseline",
    "letter-spacing",
    "offset",
    "stop-color",
    "stop-opacity",
    "gradientUnits",
    "gradientTransform",
    "maskUnits",
    "clipPathUnits",
    "href",
    "xlink:href",
    "xml:space",
    "data-name",
    "p-id",
    "t",
];

/// The entities XML itself defines; any other reference (named or numeric) is refused, so no value can spell a word in hiding.
fn entities_ok(s: &str) -> bool {
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        rest = &rest[i..];
        let Some(end) = rest.find(';') else {
            return false;
        };
        if !["&amp;", "&lt;", "&gt;", "&quot;", "&apos;"].contains(&&rest[..=end]) {
            return false;
        }
        rest = &rest[end + 1..];
    }
    true
}

/// An attribute value: no markup, only the five XML entities, `url(...)` only to a local `#id`, no script scheme.
fn icon_value_ok(name: &str, value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let urls_local = lower
        .match_indices("url(")
        .all(|(i, _)| lower[i + 4..].trim_start().starts_with('#'));
    let plain = !value.contains('<') && entities_ok(value) && urls_local;
    let scheme_free = !lower.contains("javascript:") && !lower.contains("data:");
    match name {
        "xmlns" => value == "http://www.w3.org/2000/svg",
        "xmlns:xlink" => value == "http://www.w3.org/1999/xlink",
        "href" | "xlink:href" => {
            value.starts_with('#')
                && value[1..]
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
        }
        _ => {
            plain
                && scheme_free
                && !value
                    .chars()
                    .any(|c| c.is_control() && c != '\t' && c != '\n' && c != '\r')
        }
    }
}

/// Drops a leading `<?xml ...?>` declaration, a `<!DOCTYPE svg ...>` without an internal subset and comments; whatever else starts
/// with `<!` or `<?` is refused.
fn strip_prolog(s: &str) -> Result<String, Reason> {
    let mut rest = s.trim_start_matches('\u{feff}').trim_start();
    if rest.starts_with("<?xml") {
        let end = rest.find("?>").ok_or(Reason::BadIcon)?;
        if rest[1..end].contains(['<', '>']) {
            return Err(Reason::BadIcon);
        }
        rest = rest[end + 2..].trim_start();
    }
    let mut out = String::with_capacity(rest.len());
    let mut doctype_seen = false;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        if let Some(after) = rest.strip_prefix("<!--") {
            let end = after.find("-->").ok_or(Reason::BadIcon)?;
            rest = &after[end + 3..];
        } else if rest.starts_with("<!DOCTYPE") && !doctype_seen && out.trim().is_empty() {
            let end = rest.find('>').ok_or(Reason::BadIcon)?;
            // An internal subset (`[ ... ]`) is where entities are declared: refused.
            if rest[1..end].contains(['[', ']', '<'])
                || !rest[9..end].trim_start().starts_with("svg")
            {
                return Err(Reason::BadIcon);
            }
            doctype_seen = true;
            rest = &rest[end + 1..];
        } else if rest.starts_with("<!") || rest.starts_with("<?") {
            return Err(Reason::BadIcon);
        } else {
            out.push('<');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    Ok(out.trim().to_string())
}

/// ADR-0092 open question 2: an agent's icon is vendored at conversion time only when it is a plain drawing. At most
/// [`MAX_ICON_BYTES`], UTF-8, one `<svg>` root in the SVG namespace, only the elements and attributes above, every attribute
/// quoted, `href` and `url(...)` only to a local `#id`, only the five XML entities, no CDATA, processing instruction or internal
/// doctype subset. Returns the drawing without its prolog and comments. The window shows it through an `<img>` (which never runs a
/// script nor loads anything an SVG names), so this check is a second wall, not the only one.
pub fn check_icon(bytes: &[u8]) -> Result<String, Reason> {
    if bytes.is_empty() || bytes.len() > MAX_ICON_BYTES {
        return Err(Reason::BadIcon);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| Reason::BadIcon)?;
    let svg = strip_prolog(text)?;
    let mut stack: Vec<&str> = Vec::new();
    let mut roots = 0;
    let mut rest = svg.as_str();
    loop {
        let (before, tag_start) = match rest.find('<') {
            Some(i) => (&rest[..i], Some(i)),
            None => (rest, None),
        };
        if !entities_ok(before) || before.contains('>') {
            return Err(Reason::BadIcon);
        }
        if !before.trim().is_empty() && stack.is_empty() {
            return Err(Reason::BadIcon);
        }
        let Some(i) = tag_start else { break };
        rest = &rest[i + 1..];
        let end = rest.find('>').ok_or(Reason::BadIcon)?;
        let tag = &rest[..end];
        rest = &rest[end + 1..];
        if let Some(name) = tag.strip_prefix('/') {
            if stack.pop() != Some(name.trim_end()) {
                return Err(Reason::BadIcon);
            }
            continue;
        }
        let (body, closed) = match tag.strip_suffix('/') {
            Some(b) => (b, true),
            None => (tag, false),
        };
        let name_end = body
            .find(|c: char| c.is_ascii_whitespace())
            .unwrap_or(body.len());
        let name = &body[..name_end];
        if !ICON_ELEMENTS.contains(&name) {
            return Err(Reason::BadIcon);
        }
        if stack.is_empty() {
            roots += 1;
            if name != "svg" || roots > 1 {
                return Err(Reason::BadIcon);
            }
        }
        let mut attrs = &body[name_end..];
        let mut has_ns = false;
        loop {
            attrs = attrs.trim_start();
            if attrs.is_empty() {
                break;
            }
            let eq = attrs.find('=').ok_or(Reason::BadIcon)?;
            let an = attrs[..eq].trim_end();
            if !ICON_ATTRIBUTES.contains(&an) {
                return Err(Reason::BadIcon);
            }
            let v = attrs[eq + 1..].trim_start();
            let quote = v
                .chars()
                .next()
                .filter(|q| *q == '"' || *q == '\'')
                .ok_or(Reason::BadIcon)?;
            let close = v[1..].find(quote).ok_or(Reason::BadIcon)?;
            let value = &v[1..=close];
            if !icon_value_ok(an, value) {
                return Err(Reason::BadIcon);
            }
            has_ns |= an == "xmlns";
            attrs = &v[close + 2..];
            if !attrs.is_empty() && !attrs.starts_with(|c: char| c.is_ascii_whitespace()) {
                return Err(Reason::BadIcon);
            }
        }
        if name == "svg" && stack.is_empty() && !has_ns {
            return Err(Reason::BadIcon);
        }
        if !closed {
            stack.push(name);
        }
    }
    if !stack.is_empty() || roots != 1 {
        return Err(Reason::BadIcon);
    }
    Ok(svg)
}
