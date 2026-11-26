//! The plugin author kit (roadmap M5.7; ADR-0114 decision 5; docs/ux/plugins.md, "Writing a plugin"), run on the author's own machine:
//!
//! - `agora plugin new ID [FOLDER]` makes a plugin folder from the starter (`apps/web/plugins/starter/`, built into the binary): a
//!   manifest, an entry script, a stylesheet, a README and the types of `agora` (`agora-plugin.d.ts`). It never writes into anything that
//!   exists.
//! - `agora plugin check FOLDER|FILE` runs the window's own package checks (`agora_hub::plugin_package`, a port of
//!   `apps/web/src/plugins/package.ts` held to it by shared fixtures) on a folder (packed in memory) or a package, and says what would be
//!   refused as a stable code and a sentence.
//! - `agora plugin pack FOLDER [--out FILE] [--write-hashes] [--watch]` makes the package `scripts/pack-plugin.mjs` makes, byte for byte
//!   (tests/fixtures/plugin-kit/ holds both to the same packages), then runs the same checks on it; `--watch` makes it again on every
//!   change in the folder, so a window loading it for development starts the plugin again (docs/ux/plugins.md, "Reload on save").
//!
//! `agora hub plugin …` and `agora hub plugins …` are the hub owner's commands (installing on a hub); these are the author's and touch no
//! hub. What is read is bounded: the folder's entries, each file (opened without following a link, a link or anything but a plain file
//! refused) and all of them together, by the package's 2 MiB. Only names the folder lists are read, never a name from the manifest, so
//! nothing outside the folder is reached. Everything printed that came from a folder or a package goes through [`printable`].

mod json;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime};

use agora_hub::plugin_package::{self, Refused, MANIFEST, MAX_FILES, MAX_PACKAGE_BYTES};
use agora_i18n::{key, Arg};

use crate::{Opts, Say};
use json::{Obj, Val};

const AGORA_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Files of a plugin folder that are never packed: its README and the types for the author's editor (`NOT_PACKED` in the mjs).
pub(crate) const NOT_PACKED: [&str; 2] = ["README.md", "agora-plugin.d.ts"];
/// The most entries of a folder looked at (hidden ones included), so a huge folder is refused rather than listed.
const MAX_ENTRIES: usize = 1024;
/// How often `--watch` looks at the folder.
const WATCH_EVERY: Duration = Duration::from_millis(500);

/// The starter plugin `agora plugin new` copies, as it is in the repository (`-text`, so the bytes are the same on every system).
const STARTER: [(&str, &str); 5] = [
    (
        MANIFEST,
        include_str!("../../../apps/web/plugins/starter/agora-plugin.json"),
    ),
    (
        "panel.js",
        include_str!("../../../apps/web/plugins/starter/panel.js"),
    ),
    (
        "panel.css",
        include_str!("../../../apps/web/plugins/starter/panel.css"),
    ),
    (
        "README.md",
        include_str!("../../../apps/web/plugins/starter/README.md"),
    ),
    (
        "agora-plugin.d.ts",
        include_str!("../../../apps/web/plugins/starter/agora-plugin.d.ts"),
    ),
];

/// Why a folder or a package is refused: one of the window's codes (`plugin-error-*`), or `not-a-file` (a sub-folder, a link or a
/// device in the folder) or `unreadable`. `detail` is the file or field it is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Problem {
    pub code: &'static str,
    pub detail: Option<String>,
}

fn problem(code: &'static str, detail: impl Into<String>) -> Problem {
    Problem {
        code,
        detail: Some(detail.into()),
    }
}

impl From<Refused> for Problem {
    fn from(r: Refused) -> Self {
        Problem {
            code: r.code.as_str(),
            detail: r.detail,
        }
    }
}

/// A folder packed: the package, and the manifest as `--write-hashes` rewrote it when that changed it.
#[derive(Debug)]
pub(crate) struct Packed {
    pub bundle: Vec<u8>,
    pub manifest: Option<Vec<u8>>,
}

/// `agora plugin new|check|pack` with the options of the command line ([`crate::Opts`]).
pub(crate) fn plugin(say: &Say, op: &str, o: &Opts) -> ExitCode {
    let pos: Vec<&str> = o.positional.iter().map(String::as_str).collect();
    let pack_flags = o.out.is_some() || o.write_hashes || o.watch;
    match (op, pos.as_slice()) {
        ("new", [id]) if !pack_flags => new(say, id, None),
        ("new", [id, folder]) if !pack_flags => new(say, id, Some(folder)),
        ("check", [path]) if !pack_flags => check(say, path),
        ("pack", [folder]) => pack(
            say,
            Path::new(folder),
            o.out.as_deref(),
            o.write_hashes,
            o.watch,
        ),
        _ => {
            say.err(key!("cli-bad-arguments"), &[]);
            say.err(key!("cli-plugin-usage"), &[]);
            ExitCode::from(2)
        }
    }
}

// ---- new

/// The window's id rule: lower-case letters, digits and `-`, at most 40, not starting with `-`.
fn is_plugin_id(id: &str) -> bool {
    let b = id.as_bytes();
    (1..=40).contains(&b.len())
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
        && b[0] != b'-'
}

/// The starter's files with the manifest's `id` and `name` set to `id`: every file, the README and the types included.
pub(crate) fn starter(id: &str) -> Result<Vec<(String, Vec<u8>)>, Problem> {
    let mut manifest = match json::parse(STARTER[0].1.as_bytes()) {
        Some(Val::Obj(o)) => o,
        _ => return Err(problem("manifest", MANIFEST)),
    };
    manifest.set("id", Val::Str(id.to_string()));
    manifest.set("name", Val::Str(id.to_string()));
    let mut files = vec![(
        MANIFEST.to_string(),
        format!("{}\n", json::stringify_pretty(&Val::Obj(manifest))).into_bytes(),
    )];
    files.extend(
        STARTER[1..]
            .iter()
            .map(|(n, t)| (n.to_string(), t.as_bytes().to_vec())),
    );
    Ok(files)
}

fn new(say: &Say, id: &str, folder: Option<&str>) -> ExitCode {
    if !is_plugin_id(id) {
        say.err(
            key!("cli-plugin-new-bad-id"),
            &[("id", printable(id).into())],
        );
        return ExitCode::from(1);
    }
    let folder = PathBuf::from(folder.unwrap_or(id));
    let shown = printable(&folder.display().to_string());
    // The new folder passes the window's checks before anything is written.
    let checked = starter(id).and_then(|files| {
        let packed = pack_files(&sorted(files.clone()), false)?;
        plugin_package::check(&packed.bundle, AGORA_VERSION).map_err(Problem::from)?;
        Ok(files)
    });
    let files = match checked {
        Ok(f) => f,
        Err(p) => return refused(say, &shown, &p),
    };
    if let Err(e) = std::fs::create_dir(&folder) {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            say.err(key!("cli-plugin-new-exists"), &[("folder", shown.into())]);
        } else {
            say.err(
                key!("cli-plugin-write-failed"),
                &[("path", shown.into()), ("reason", e.to_string().into())],
            );
        }
        return ExitCode::from(1);
    }
    for (name, bytes) in &files {
        let path = folder.join(name);
        let written = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .and_then(|mut f| f.write_all(bytes));
        if let Err(e) = written {
            say.err(
                key!("cli-plugin-write-failed"),
                &[
                    ("path", printable(&path.display().to_string()).into()),
                    ("reason", e.to_string().into()),
                ],
            );
            return ExitCode::from(1);
        }
    }
    say.out(
        key!("cli-plugin-new-done"),
        &[("id", id.into()), ("folder", shown.into())],
    );
    ExitCode::SUCCESS
}

// ---- reading a folder

/// Names sorted as JavaScript's default sort does (by UTF-16 units), as the mjs sorts a folder's names.
fn sorted(mut files: Vec<(String, Vec<u8>)>) -> Vec<(String, Vec<u8>)> {
    files.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
    files
}

/// The folder's files that a package is made of (the manifest and every other file but [`NOT_PACKED`]), each read once, in the mjs's
/// order. Hidden names (a leading dot) are left out, as the mjs leaves them. A sub-folder, a link or anything but a plain file is
/// refused, whatever its name; nothing it points at is read.
pub(crate) fn read_folder(dir: &Path) -> Result<Vec<(String, Vec<u8>)>, Problem> {
    let unreadable = |e: std::io::Error| problem("unreadable", e.to_string());
    let mut names = Vec::new();
    for (n, entry) in std::fs::read_dir(dir).map_err(unreadable)?.enumerate() {
        if n >= MAX_ENTRIES {
            return Err(Problem {
                code: "too-many",
                detail: None,
            });
        }
        let entry = entry.map_err(unreadable)?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            return Err(problem("path", entry.file_name().to_string_lossy()));
        };
        if name.starts_with('.') {
            continue;
        }
        // `DirEntry::file_type` does not follow a link.
        let is_file = entry.file_type().map_err(unreadable)?.is_file();
        names.push((name, is_file));
    }
    names.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
    if names.len() > MAX_FILES + 1 + NOT_PACKED.len() {
        return Err(Problem {
            code: "too-many",
            detail: None,
        });
    }
    if let Some((name, _)) = names.iter().find(|(_, is_file)| !is_file) {
        return Err(problem("not-a-file", name.clone()));
    }
    let mut left = MAX_PACKAGE_BYTES as u64;
    let mut files = Vec::new();
    for (name, _) in names
        .into_iter()
        .filter(|(n, _)| !NOT_PACKED.contains(&n.as_str()))
    {
        let bytes = agora_platform::read_small_regular_file_no_follow(&dir.join(&name), left)
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::InvalidData => problem("too-large", name.clone()),
                std::io::ErrorKind::InvalidInput => problem("not-a-file", name.clone()),
                _ => problem("unreadable", name.clone()),
            })?;
        left -= bytes.len() as u64;
        files.push((name, bytes));
    }
    Ok(files)
}

// ---- packing

fn sha256_hex(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Base64 with padding, as Node's `Buffer.toString('base64')`.
fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ABC[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// What `pack()` of `scripts/pack-plugin.mjs` does with a folder's files (sorted, the manifest among them, [`NOT_PACKED`] left out):
/// with `write_hashes`, the manifest's `files` becomes every other file with its SHA-256 (rewritten as `JSON.stringify(manifest, null,
/// 2)` and a line break); then every other file must be listed with its hash, and the package is the manifest's bytes as they are and
/// each listed file, in the list's order, as base64 in `{"agora-plugin-bundle":1,"files":{…}}`. The window's checks are not run here.
pub(crate) fn pack_files(
    files: &[(String, Vec<u8>)],
    write_hashes: bool,
) -> Result<Packed, Problem> {
    let Some((_, raw)) = files.iter().find(|(n, _)| n == MANIFEST) else {
        return Err(problem("manifest", MANIFEST));
    };
    let Some(Val::Obj(mut manifest)) = json::parse(raw) else {
        return Err(problem("manifest", MANIFEST));
    };
    let others: Vec<&(String, Vec<u8>)> = files
        .iter()
        .filter(|(n, _)| n != MANIFEST && !NOT_PACKED.contains(&n.as_str()))
        .collect();
    let mut rewritten = None;
    if write_hashes {
        let mut list = Obj::default();
        for (n, b) in &others {
            list.set(n, Val::Str(sha256_hex(b)));
        }
        manifest.set("files", Val::Obj(list));
        let text =
            format!("{}\n", json::stringify_pretty(&Val::Obj(manifest.clone()))).into_bytes();
        if &text != raw {
            rewritten = Some(text);
        }
    }
    let empty = Obj::default();
    let list = match manifest.get("files") {
        None | Some(Val::Null) => &empty,
        Some(Val::Obj(o)) => o,
        Some(_) => return Err(problem("manifest", "files")),
    };
    let listed = list.keys();
    if let Some((n, _)) = others.iter().find(|(n, _)| !listed.contains(&n.as_str())) {
        return Err(problem("unlisted", n.clone()));
    }
    let mut entries: Vec<(&str, &[u8])> = Vec::new();
    for n in &listed {
        let Some((_, bytes)) = others.iter().find(|(o, _)| o == n) else {
            return Err(problem("missing", *n));
        };
        if list.get(n) != Some(&Val::Str(sha256_hex(bytes))) {
            return Err(problem("hash", *n));
        }
        entries.push((n, bytes));
    }
    let manifest_bytes = rewritten.as_deref().unwrap_or(raw);
    // `Object.fromEntries([MANIFEST, ...listed])`: in JavaScript's key order too.
    let mut order = Obj::default();
    order.set(MANIFEST, Val::Null);
    for (n, _) in &entries {
        order.set(n, Val::Null);
    }
    let mut bundle = String::from("{\"agora-plugin-bundle\":1,\"files\":{");
    for (i, name) in order.keys().into_iter().enumerate() {
        let bytes = if name == MANIFEST {
            manifest_bytes
        } else {
            entries
                .iter()
                .find(|(n, _)| *n == name)
                .map_or(&[][..], |(_, b)| *b)
        };
        if i > 0 {
            bundle.push(',');
        }
        bundle.push_str(&json::quote(name));
        bundle.push(':');
        bundle.push('"');
        bundle.push_str(&base64(bytes));
        bundle.push('"');
    }
    bundle.push_str("}}");
    Ok(Packed {
        bundle: bundle.into_bytes(),
        manifest: rewritten,
    })
}

/// A folder packed as `scripts/pack-plugin.mjs` packs it, the rewritten manifest (with `write_hashes`) not yet written.
pub(crate) fn pack_folder(dir: &Path, write_hashes: bool) -> Result<Packed, Problem> {
    pack_files(&read_folder(dir)?, write_hashes)
}

// ---- check

fn check(say: &Say, path: &str) -> ExitCode {
    let shown = printable(path);
    let p = Path::new(path);
    let bytes = match std::fs::metadata(p) {
        Ok(m) if m.is_dir() => pack_folder(p, false).map(|x| x.bundle),
        Ok(_) => read_package(p),
        Err(e) => Err(problem("unreadable", e.to_string())),
    };
    let checked =
        bytes.and_then(|b| plugin_package::check(&b, AGORA_VERSION).map_err(Problem::from));
    match checked {
        Ok(c) => {
            passes(say, &shown, &c.manifest);
            ExitCode::SUCCESS
        }
        Err(e) => refused(say, &shown, &e),
    }
}

/// A package file, at most its 2 MiB read (one byte more says it is over).
fn read_package(p: &Path) -> Result<Vec<u8>, Problem> {
    let unreadable = |e: std::io::Error| problem("unreadable", e.to_string());
    let f = std::fs::File::open(p).map_err(unreadable)?;
    if !f.metadata().map_err(unreadable)?.is_file() {
        return Err(Problem {
            code: "not-a-file",
            detail: None,
        });
    }
    let mut out = Vec::new();
    f.take(MAX_PACKAGE_BYTES as u64 + 1)
        .read_to_end(&mut out)
        .map_err(unreadable)?;
    if out.len() > MAX_PACKAGE_BYTES {
        return Err(Problem {
            code: "too-large",
            detail: None,
        });
    }
    Ok(out)
}

fn passes(say: &Say, shown: &str, m: &plugin_package::Manifest) {
    let caps = if m.capabilities.is_empty() {
        say.text(key!("cli-plugin-sees-nothing"), &[])
    } else {
        m.capabilities.join(", ")
    };
    say.out(
        key!("cli-plugin-ok"),
        &[
            ("path", shown.into()),
            ("id", m.id.as_str().into()),
            ("name", printable(&m.name).into()),
            ("version", m.version.as_str().into()),
            ("panels", Arg::Num(m.panels.len() as i64)),
            ("capabilities", caps.into()),
        ],
    );
}

fn refused(say: &Say, shown: &str, p: &Problem) -> ExitCode {
    say.err(
        key!("cli-plugin-refused"),
        &[
            ("path", shown.into()),
            ("code", p.code.into()),
            ("reason", reason(say, p).into()),
        ],
    );
    ExitCode::from(1)
}

/// The sentence for a problem's code (`cli-plugin-error-*`), with the file or field it is about.
fn reason(say: &Say, p: &Problem) -> String {
    let id = match p.code {
        "too-large" => key!("cli-plugin-error-too-large"),
        "format" => key!("cli-plugin-error-format"),
        "newer-format" => key!("cli-plugin-error-newer-format"),
        "too-many" => key!("cli-plugin-error-too-many"),
        "path" => key!("cli-plugin-error-path"),
        "link" => key!("cli-plugin-error-link"),
        "type" => key!("cli-plugin-error-type"),
        "manifest" => key!("cli-plugin-error-manifest"),
        "capability" => key!("cli-plugin-error-capability"),
        "unlisted" => key!("cli-plugin-error-unlisted"),
        "missing" => key!("cli-plugin-error-missing"),
        "hash" => key!("cli-plugin-error-hash"),
        "newer" => key!("cli-plugin-error-newer"),
        "not-a-file" => key!("cli-plugin-error-not-a-file"),
        _ => key!("cli-plugin-error-unreadable"),
    };
    say.text(
        id,
        &[
            (
                "detail",
                printable(p.detail.as_deref().unwrap_or("")).into(),
            ),
            ("version", AGORA_VERSION.into()),
        ],
    )
}

/// Text from a folder, a package or the command line made safe for a terminal: no control characters (escape sequences), no
/// invisible or direction-changing ones, at most 100 characters.
pub(crate) fn printable(s: &str) -> String {
    s.chars()
        .take(100)
        .map(|c| {
            let invisible = matches!(c, '\u{ad}' | '\u{61c}' | '\u{180e}' | '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}' | '\u{fff9}'..='\u{fffb}');
            if c.is_control() || invisible {
                '\u{fffd}'
            } else {
                c
            }
        })
        .collect()
}

// ---- pack

fn pack(say: &Say, folder: &Path, out: Option<&Path>, write_hashes: bool, watch: bool) -> ExitCode {
    let shown = printable(&folder.display().to_string());
    if watch && out.is_none() {
        say.err(key!("cli-plugin-watch-needs-out"), &[]);
        return ExitCode::from(2);
    }
    if let Some(out) = out {
        if inside(folder, out) {
            say.err(
                key!("cli-plugin-out-inside"),
                &[
                    ("folder", shown.into()),
                    ("out", printable(&out.display().to_string()).into()),
                ],
            );
            return ExitCode::from(2);
        }
    }
    if !watch {
        return if pack_once(say, folder, &shown, out, write_hashes, &mut None) {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(1)
        };
    }
    say.out(
        key!("cli-plugin-watching"),
        &[("folder", shown.clone().into())],
    );
    let mut seen = None;
    let mut last = None;
    loop {
        let now = snapshot(folder);
        if seen.as_ref() != Some(&now) {
            seen = Some(now);
            pack_once(say, folder, &shown, out, write_hashes, &mut last);
        }
        std::thread::sleep(WATCH_EVERY);
    }
}

/// Whether `out` would be written into `folder` (a package there is a file the folder does not list, and `--write-hashes` would list
/// it).
fn inside(folder: &Path, out: &Path) -> bool {
    let parent = match out.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    match (std::fs::canonicalize(folder), std::fs::canonicalize(parent)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// What `--watch` compares: each entry's name, kind, length and time of change (a link is not followed).
fn snapshot(folder: &Path) -> Vec<(std::ffi::OsString, bool, u64, Option<SystemTime>)> {
    let mut out: Vec<_> = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .take(MAX_ENTRIES + 1)
        .flatten()
        .map(|e| {
            let m = e.metadata().ok();
            (
                e.file_name(),
                m.as_ref().is_some_and(std::fs::Metadata::is_file),
                m.as_ref().map_or(0, std::fs::Metadata::len),
                m.and_then(|m| m.modified().ok()),
            )
        })
        .collect();
    out.sort();
    out
}

/// Packs once: writes the manifest when `--write-hashes` changed it, checks the package as the window will, and writes it (to `out`,
/// unless it is what `last` says was written there; else to stdout). Says what happened.
fn pack_once(
    say: &Say,
    folder: &Path,
    shown: &str,
    out: Option<&Path>,
    write_hashes: bool,
    last: &mut Option<Vec<u8>>,
) -> bool {
    let packed = match pack_folder(folder, write_hashes) {
        Ok(p) => p,
        Err(p) => {
            refused(say, shown, &p);
            return false;
        }
    };
    if let Some(m) = &packed.manifest {
        let path = folder.join(MANIFEST);
        if let Err(e) = std::fs::write(&path, m) {
            say.err(
                key!("cli-plugin-write-failed"),
                &[
                    ("path", printable(&path.display().to_string()).into()),
                    ("reason", e.to_string().into()),
                ],
            );
            return false;
        }
        // The package itself may be going to stdout: what was done is said on stderr then.
        let args: &[(&str, Arg)] = &[("folder", shown.into())];
        if out.is_some() {
            say.out(key!("cli-plugin-hashes-written"), args);
        } else {
            say.err(key!("cli-plugin-hashes-written"), args);
        }
    }
    if let Err(p) = plugin_package::check(&packed.bundle, AGORA_VERSION) {
        refused(say, shown, &p.into());
        return false;
    }
    let Some(out) = out else {
        let mut o = std::io::stdout().lock();
        return o.write_all(&packed.bundle).and_then(|()| o.flush()).is_ok();
    };
    if last.as_ref() == Some(&packed.bundle) {
        return true;
    }
    let shown_out = printable(&out.display().to_string());
    if let Err(e) = write_whole(out, &packed.bundle) {
        say.err(
            key!("cli-plugin-write-failed"),
            &[("path", shown_out.into()), ("reason", e.to_string().into())],
        );
        return false;
    }
    say.out(
        key!("cli-plugin-packed"),
        &[
            ("folder", shown.into()),
            ("out", shown_out.into()),
            ("bytes", Arg::Num(packed.bundle.len() as i64)),
        ],
    );
    *last = Some(packed.bundle);
    true
}

/// Writes a file whole: into a hidden file beside it, then renamed over it, so a window watching it never reads half a package.
fn write_whole(out: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let name = out
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = out.with_file_name(format!(".{name}.agora-tmp"));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, out).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn file(name: &str, text: &str) -> (String, Vec<u8>) {
        (name.to_string(), text.as_bytes().to_vec())
    }

    #[test]
    fn base64_is_nodes() {
        for (raw, b64) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
        ] {
            assert_eq!(base64(raw.as_bytes()), b64);
        }
        assert_eq!(base64(&[0xfb, 0xff, 0xfe]), "+//+");
    }

    #[test]
    fn plugin_ids_follow_the_window() {
        for ok in ["a", "my-plugin", "0x", &"a".repeat(40)] {
            assert!(is_plugin_id(ok), "{ok}");
        }
        for bad in ["", "-a", "A", "a_b", "a.b", "../a", &"a".repeat(41), "é"] {
            assert!(!is_plugin_id(bad), "{bad}");
        }
    }

    #[test]
    fn the_starter_passes_the_windows_checks_with_any_id() {
        for id in ["x", "my-plugin", &"z".repeat(40)] {
            let files = starter(id).unwrap();
            assert_eq!(files.len(), 5);
            let packed = pack_files(&sorted(files), false).unwrap();
            assert!(packed.manifest.is_none());
            let c = plugin_package::check(&packed.bundle, AGORA_VERSION).unwrap();
            assert_eq!((c.manifest.id.as_str(), c.manifest.name.as_str()), (id, id));
        }
        // The starter as it is in the repository has its hashes written: --write-hashes changes nothing.
        let files = sorted(STARTER.iter().map(|(n, t)| file(n, t)).collect());
        assert!(pack_files(&files, true).unwrap().manifest.is_none());
    }

    #[test]
    fn a_listed_name_is_never_read_from_outside_the_folder() {
        let m = r#"{"files":{"../secret.js":"00"}}"#;
        assert_eq!(
            pack_files(&[file(MANIFEST, m)], false).unwrap_err(),
            problem("missing", "../secret.js")
        );
        assert_eq!(
            pack_files(&[file(MANIFEST, m), file("a.js", "x")], false).unwrap_err(),
            problem("unlisted", "a.js")
        );
        assert_eq!(
            pack_files(&[file("a.js", "x")], false).unwrap_err(),
            problem("manifest", MANIFEST)
        );
        assert_eq!(
            pack_files(&[file(MANIFEST, "{")], false).unwrap_err(),
            problem("manifest", MANIFEST)
        );
        assert_eq!(
            pack_files(&[file(MANIFEST, r#"{"files":"x"}"#)], false).unwrap_err(),
            problem("manifest", "files")
        );
        let wrong = r#"{"files":{"a.js":"00"}}"#;
        assert_eq!(
            pack_files(&[file(MANIFEST, wrong), file("a.js", "x")], false).unwrap_err(),
            problem("hash", "a.js")
        );
        // No `files` at all packs the manifest alone (the window then refuses it for its missing fields).
        let only = pack_files(&[file(MANIFEST, "{}")], false).unwrap();
        assert_eq!(
            String::from_utf8(only.bundle).unwrap(),
            r#"{"agora-plugin-bundle":1,"files":{"agora-plugin.json":"e30="}}"#
        );
    }

    #[test]
    fn printable_takes_out_escapes_and_invisible_characters() {
        assert_eq!(
            printable("a\u{1b}[31mb\u{202e}c\u{7f}"),
            "a\u{fffd}[31mb\u{fffd}c\u{fffd}"
        );
        assert_eq!(printable(&"x".repeat(300)).chars().count(), 100);
    }
}
