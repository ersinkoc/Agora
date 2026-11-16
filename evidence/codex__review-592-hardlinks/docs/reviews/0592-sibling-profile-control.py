"""Execute the unchanged macOS profile generator on Linux with path-walk shims.

The file metadata/link counts are real. Darwin path walking and Seatbelt execution
are not emulated; native macOS regression coverage is supplied separately.
"""
from pathlib import Path
import json
import re
import subprocess
import tempfile

root = Path(__file__).resolve().parents[2]
source = (root / "crates/agora-platform/src/macos.rs").read_text()


def function(marker):
    start = source.index(marker)
    brace = source.index("{", start)
    depth, end = 1, brace + 1
    while depth:
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    return source[start:end]


parts = [function(marker) for marker in ["fn literal(", "fn quote(", "pub(crate) fn profile(", "fn sibling_rule("]]
private = re.search(r'const PRIVATE:[^\n]+;', source).group(0)
shim = r'''
#![allow(dead_code)]
use std::{io, path::{Path, PathBuf}};
use std::os::unix::fs::MetadataExt;
#[derive(Default)] pub struct Sandbox { read: Vec<PathBuf>, write: Vec<PathBuf>, deny: Vec<PathBuf>, reopen: Vec<PathBuf>, sockets: Vec<PathBuf>, offline: bool }
#[derive(Debug)] pub enum SandboxRefusal { ProtectedPathUnresolved, WorktreeNotFolder, WritableIsLink, WritableHasLinks }
impl std::fmt::Display for SandboxRefusal { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{self:?}") } }
impl std::error::Error for SandboxRefusal {}
struct Walked { path: PathBuf, dir: bool, file: bool, links: u64 }
enum Walk { Missing(PathBuf), AgentsLink, Failed }
fn walk(p: &Path, _: &dyn Fn(&Path, std::os::fd::RawFd) -> bool) -> Result<Walked, Walk> {
    let file = std::fs::File::open(p).map_err(|_| Walk::Failed)?;
    let m = file.metadata().map_err(|_| Walk::Failed)?;
    Ok(Walked { path: p.canonicalize().map_err(|_| Walk::Failed)?, dir: m.is_dir(), file: m.is_file(), links: m.nlink() })
}
fn agents_link(_: &Path, _: std::os::fd::RawFd, _: &[PathBuf]) -> bool { false }
fn real_or_future(p: &Path) -> Option<PathBuf> { p.canonicalize().ok() }
fn real_or_new(p: &Path) -> Option<PathBuf> { Some(p.to_path_buf()) }
'''
test = r'''
fn main() {
    let root = PathBuf::from(std::env::args().nth(1).unwrap());
    std::fs::create_dir_all(root.join("private")).unwrap();
    let config = root.join("state.json");
    let outside = root.join("private/zshrc");
    std::fs::write(&config, "{}").unwrap();
    std::fs::write(&outside, "keep").unwrap();
    let sb = Sandbox { write: vec![config.clone()], ..Default::default() };
    assert!(profile(&sb).is_ok());
    let direct_alias = root.join("direct-alias");
    std::fs::hard_link(&config, &direct_alias).unwrap();
    assert!(profile(&sb).is_err(), "direct-file guard control failed");
    std::fs::remove_file(direct_alias).unwrap();
    let sibling = root.join("state.json.tmp");
    std::fs::hard_link(&outside, &sibling).unwrap();
    println!("config_links={}", config.metadata().unwrap().nlink());
    println!("sibling_links={}", sibling.metadata().unwrap().nlink());
    match profile(&sb) {
        Ok(policy) => {
            println!("PROFILE_START\n{policy}PROFILE_END");
            println!("SIBLING={}", sibling.display());
            panic!("pre-existing hard-linked atomic-save sibling not refused");
        }
        Err(_) => println!("sibling_refused=true"),
    }
}
'''
with tempfile.TemporaryDirectory(prefix="agora-review-592-") as temporary:
    temp = Path(temporary)
    rust = temp / "profile.rs"
    rust.write_text(shim + private + "\n" + "\n".join(parts) + test)
    executable = temp / "profile"
    subprocess.run(["rustc", "--edition=2021", str(rust), "-o", str(executable)], check=True)
    p = subprocess.run([str(executable), str(temp / "fixture")], text=True, capture_output=True)
    matches = False
    if p.returncode:
        rule = re.search(r'\(regex #"([^\n]+)"\)', p.stdout).group(1)
        sibling = re.search(r'SIBLING=(.+)', p.stdout).group(1)
        matches = re.fullmatch(rule, sibling) is not None
        assert matches
    result = {"profile_control_exit": p.returncode, "direct_file_guard_control_passed": True,
              "config_links": 1, "sibling_links": 2, "sibling_allowed_by_generated_regex": matches,
              "scope": "unchanged profile/quoting/sibling functions; Linux metadata/path shims; no Seatbelt execution"}
    print(json.dumps(result, indent=2))
    raise SystemExit(p.returncode)
