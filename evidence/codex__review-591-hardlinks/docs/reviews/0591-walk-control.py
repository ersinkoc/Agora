"""Exercise the unchanged Windows walk on Linux with native hard links.

Only Windows DACL/owner queries are stubbed; link count uses Linux metadata.
This proves the walk's name/type decision, not Windows ACL or confinement behavior.
"""
from pathlib import Path
import re
import subprocess
import tempfile

root = Path(__file__).resolve().parents[2]
source = (root / "crates/agora-platform/src/winconfine.rs").read_text()
start = source.index("fn broad_writer_below(")
brace = source.index("{", start)
depth = 1
end = brace + 1
while depth:
    depth += (source[end] == "{") - (source[end] == "}")
    end += 1
walk = source[start:end]
exemptions = re.search(r'const LINKS_NOT_CHECKED:.*?;', source, re.S).group(0)
shim = r'''
use std::{io, path::{Path, PathBuf}};
use std::os::unix::fs::MetadataExt;
#[derive(Debug, PartialEq, Eq)]
enum SandboxRefusal { BroadWriteGrant, LabelFailed, WritableHasLinks }
struct Sid;
impl Sid { fn from_string(_: &str) -> io::Result<Self> { Ok(Self) } }
const REFUSED_WRITERS: [&str; 0] = [];
fn self_grant_enabled() -> bool { true }
fn grants_write_to(_: &Path, _: &[Sid]) -> bool { false }
fn may_write_owner(_: &Path) -> bool { true }
fn has_other_names(p: &Path) -> io::Result<bool> { Ok(std::fs::metadata(p)?.nlink() > 1) }
'''
test = r'''
fn main() {
    let root = PathBuf::from(std::env::args().nth(1).unwrap());
    std::fs::create_dir_all(&root).unwrap();
    let outside = root.join("outside.txt");
    std::fs::write(&outside, "keep").unwrap();
    let control = root.join("control");
    std::fs::create_dir_all(&control).unwrap();
    std::fs::hard_link(&outside, control.join("plain.txt")).unwrap();
    assert_eq!(broad_writer_below(&control, true).unwrap(), Some((control.join("plain.txt"), SandboxRefusal::WritableHasLinks)));
    println!("normal_file_refused=true");
    let mut refused = 0;
    for (i, name) in LINKS_NOT_CHECKED.iter().enumerate() {
        let folder_case = root.join(format!("folder-{i}"));
        let package = folder_case.join(name).join("pkg");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::hard_link(&outside, package.join("linked")).unwrap();
        assert_eq!(broad_writer_below(&folder_case, true).unwrap(), None);
        println!("directory_name={name} exempt=true");
        let work = root.join(format!("case-{i}"));
        std::fs::create_dir_all(&work).unwrap();
        let link = work.join(name);
        std::fs::hard_link(&outside, &link).unwrap();
        assert!(link.is_file());
        let hit = broad_writer_below(&work, true).unwrap();
        println!("regular_file_name={name} refused={}", hit.is_some());
        refused += usize::from(hit.is_some());
    }
    assert_eq!(refused, LINKS_NOT_CHECKED.len(), "directory exemption also bypasses checks for regular files");
}
'''
with tempfile.TemporaryDirectory(prefix="agora-review-591-") as temporary:
    temp = Path(temporary)
    rust = temp / "walk.rs"
    rust.write_text(shim + exemptions + "\n" + walk + "\n" + test)
    executable = temp / "walk"
    subprocess.run(["rustc", "--edition=2021", str(rust), "-o", str(executable)], check=True)
    p = subprocess.run([str(executable), str(temp / "files")], text=True, capture_output=True)
    print(p.stdout, end="")
    if p.returncode:
        print(p.stderr, end="")
    raise SystemExit(p.returncode)
