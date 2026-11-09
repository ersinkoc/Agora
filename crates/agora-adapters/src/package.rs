//! Registry packages on disk (ADR-0044): a package is a plain tar of a manifest and, for an L3 adapter, the files its command runs.
//! Nothing in it is trusted: this reads only regular files with safe relative paths, within fixed limits, and never executes anything.

use std::path::{Path, PathBuf};

const MAX_FILES: usize = 64;
const MAX_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageError {
    /// Not a tar this reader accepts (links, devices, extended headers and truncated archives are refused).
    Malformed,
    /// An absolute path, a `..`, an empty name or a name with a backslash or a NUL.
    UnsafePath(String),
    TooManyFiles,
    TooLarge,
    /// No `manifest.toml` at the top.
    NoManifest,
    /// The manifest is not TOML, or its `kind` is not the package's name.
    BadManifest,
    /// The package name is not a plain lowercase name.
    BadName,
    /// An `[adapter] command` entry names a file the package does not hold.
    MissingFile(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub data: Vec<u8>,
    pub executable: bool,
}

/// A package or kind name: lowercase letters, digits, `-` and `_`, starting with a letter or digit, at most 64 characters.
pub fn valid_name(name: &str) -> bool {
    let mut c = name.chars();
    matches!(c.next(), Some(f) if f.is_ascii_lowercase() || f.is_ascii_digit())
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

fn octal(field: &[u8]) -> Option<usize> {
    let s: String = field
        .iter()
        .take_while(|b| **b != 0 && **b != b' ')
        .map(|b| *b as char)
        .collect();
    let s = s.trim();
    if s.is_empty() {
        return Some(0);
    }
    usize::from_str_radix(s, 8).ok()
}

fn text(field: &[u8]) -> String {
    String::from_utf8_lossy(&field[..field.iter().position(|b| *b == 0).unwrap_or(field.len())])
        .into_owned()
}

fn safe(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains(['\\', '\0'])
        && path.split('/').all(|p| p != ".." && !p.is_empty())
}

/// Reads the regular files of a plain (uncompressed, ustar) tar.
pub fn unpack(bytes: &[u8]) -> Result<Vec<Entry>, PackageError> {
    let mut out = Vec::new();
    let (mut at, mut total) = (0usize, 0usize);
    loop {
        let Some(h) = bytes.get(at..at + 512) else {
            return Err(PackageError::Malformed);
        };
        if h.iter().all(|b| *b == 0) {
            return Ok(out);
        }
        let size = octal(&h[124..136]).ok_or(PackageError::Malformed)?;
        let mode = octal(&h[100..108]).ok_or(PackageError::Malformed)?;
        let name = text(&h[0..100]);
        let prefix = text(&h[345..500]);
        let path = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let start = at + 512;
        at = start + size.div_ceil(512) * 512;
        match h[156] {
            b'0' | 0 => {
                let path = path.trim_start_matches("./").to_string();
                if !safe(&path) {
                    return Err(PackageError::UnsafePath(path));
                }
                total = total.saturating_add(size);
                if total > MAX_BYTES {
                    return Err(PackageError::TooLarge);
                }
                if out.len() >= MAX_FILES {
                    return Err(PackageError::TooManyFiles);
                }
                let data = bytes
                    .get(start..start + size)
                    .ok_or(PackageError::Malformed)?
                    .to_vec();
                out.push(Entry {
                    path,
                    data,
                    executable: mode & 0o111 != 0,
                });
            }
            b'5' => {}
            _ => return Err(PackageError::Malformed),
        }
    }
}

/// The manifest to register: the package's own `manifest.toml`, checked to be the kind `name`, with each `[adapter] command` entry
/// that starts with `./` pointing into `dir` (and required to be a file of the package).
pub fn registered_manifest(
    name: &str,
    entries: &[Entry],
    dir: &Path,
) -> Result<String, PackageError> {
    if !valid_name(name) {
        return Err(PackageError::BadName);
    }
    let m = entries
        .iter()
        .find(|e| e.path == "manifest.toml")
        .ok_or(PackageError::NoManifest)?;
    let src = std::str::from_utf8(&m.data).map_err(|_| PackageError::BadManifest)?;
    let mut doc: toml::Table = src.parse().map_err(|_| PackageError::BadManifest)?;
    if doc.get("kind").and_then(|k| k.as_str()) != Some(name) {
        return Err(PackageError::BadManifest);
    }
    if let Some(cmd) = doc
        .get_mut("adapter")
        .and_then(|a| a.get_mut("command"))
        .and_then(|c| c.as_array_mut())
    {
        for item in cmd.iter_mut() {
            let Some(s) = item.as_str() else { continue };
            if let Some(rel) = s.strip_prefix("./") {
                if !safe(rel) || !entries.iter().any(|e| e.path == rel) {
                    return Err(PackageError::MissingFile(s.to_string()));
                }
                *item = toml::Value::String(dir.join(rel).to_string_lossy().into_owned());
            }
        }
    }
    toml::to_string(&doc).map_err(|_| PackageError::BadManifest)
}

/// The program an adapter manifest runs, for the question "do you want to install this".
pub fn adapter_command(entries: &[Entry]) -> Option<Vec<String>> {
    let m = entries.iter().find(|e| e.path == "manifest.toml")?;
    let doc: toml::Table = std::str::from_utf8(&m.data).ok()?.parse().ok()?;
    let cmd = doc.get("adapter")?.get("command")?.as_array()?;
    Some(
        cmd.iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
    )
}

/// Where a package's files go: `<home>/adapters/<name>/<version>`.
pub fn package_dir(home: &Path, name: &str, version: &str) -> PathBuf {
    home.join("adapters").join(name).join(version)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    pub fn tar(files: &[(&str, &[u8], u8)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (name, data, kind) in files {
            let mut h = [0u8; 512];
            h[..name.len()].copy_from_slice(name.as_bytes());
            h[100..107].copy_from_slice(b"0000755");
            h[124..135].copy_from_slice(format!("{:011o}", data.len()).as_bytes());
            h[156] = *kind;
            out.extend_from_slice(&h);
            out.extend_from_slice(data);
            out.resize(out.len().div_ceil(512) * 512, 0);
        }
        out.extend_from_slice(&[0u8; 1024]);
        out
    }

    #[test]
    fn regular_files_come_out_and_everything_else_is_refused() {
        let ok = unpack(&tar(&[
            ("manifest.toml", b"kind = \"x\"", b'0'),
            ("bin/run", b"#!", b'0'),
        ]))
        .unwrap();
        assert_eq!(ok.len(), 2);
        assert!(ok[1].executable && ok[1].data == b"#!");
        for bad in ["/etc/passwd", "../x", "a/../b", "a//b"] {
            assert!(
                matches!(
                    unpack(&tar(&[(bad, b"x", b'0')])),
                    Err(PackageError::UnsafePath(_))
                ),
                "{bad}"
            );
        }
        assert_eq!(
            unpack(&tar(&[("l", b"", b'2')])),
            Err(PackageError::Malformed)
        );
        assert_eq!(
            unpack(&tar(&[("p", b"", b'x')])),
            Err(PackageError::Malformed)
        );
        assert_eq!(unpack(&[1u8; 100]), Err(PackageError::Malformed));
    }

    #[test]
    fn a_package_may_not_hold_more_than_sixty_four_files() {
        let names: Vec<String> = (0..65).map(|i| format!("f{i}")).collect();
        let files: Vec<(&str, &[u8], u8)> = names
            .iter()
            .map(|n| (n.as_str(), &b"x"[..], b'0'))
            .collect();
        assert_eq!(unpack(&tar(&files)), Err(PackageError::TooManyFiles));
    }

    #[test]
    fn the_registered_manifest_points_into_the_install_folder() {
        let manifest = b"kind = \"demo\"\n[adapter]\ncommand = [\"./run\", \"--x\", \"node\"]\n";
        let e = unpack(&tar(&[
            ("manifest.toml", manifest, b'0'),
            ("run", b"#!", b'0'),
        ]))
        .unwrap();
        let out = registered_manifest("demo", &e, Path::new("/h/adapters/demo/1")).unwrap();
        assert!(
            out.contains("/h/adapters/demo/1/run")
                && out.contains("\"--x\"")
                && out.contains("\"node\"")
        );
        assert_eq!(adapter_command(&e).unwrap()[0], "./run");
        // A different kind, a missing file and a bad name are refused.
        assert_eq!(
            registered_manifest("other", &e, Path::new("/h")),
            Err(PackageError::BadManifest)
        );
        let e2 = unpack(&tar(&[("manifest.toml", manifest, b'0')])).unwrap();
        assert!(matches!(
            registered_manifest("demo", &e2, Path::new("/h")),
            Err(PackageError::MissingFile(_))
        ));
        assert_eq!(
            registered_manifest("../x", &e, Path::new("/h")),
            Err(PackageError::BadName)
        );
        assert!(
            valid_name("wrong-stack_2")
                && !valid_name("Up")
                && !valid_name("")
                && !valid_name("a/b")
                && !valid_name("-a")
        );
    }
}
