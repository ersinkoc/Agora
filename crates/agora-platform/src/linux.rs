//! Process inspection through `/proc`.

use std::fs;
use std::io;
use std::path::PathBuf;

pub(crate) fn name(pid: u32) -> io::Result<String> {
    let comm = fs::read_to_string(format!("/proc/{pid}/comm"))?;
    Ok(comm.trim_end_matches('\n').to_string())
}

pub(crate) fn cwd(pid: u32) -> io::Result<PathBuf> {
    fs::read_link(format!("/proc/{pid}/cwd"))
}

/// Parent pid from `/proc/<pid>/stat`: the field after the last `)` (the name may contain parentheses).
fn ppid(pid: u32) -> Option<u32> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(1)?.parse().ok()
}

pub(crate) fn children(pid: u32) -> io::Result<Vec<u32>> {
    let mut out = Vec::new();
    for entry in fs::read_dir("/proc")? {
        let Ok(entry) = entry else { continue };
        let Some(child) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        if ppid(child) == Some(pid) {
            out.push(child);
        }
    }
    out.sort_unstable();
    Ok(out)
}
