//! Process inspection through `/proc`.

use std::fs;
use std::io;
use std::path::PathBuf;

pub(crate) fn stats(pid: u32) -> io::Result<crate::ProcessStats> {
    let bad = || io::Error::new(io::ErrorKind::InvalidData, "unexpected /proc format");
    // statm: size resident shared ... (in pages)
    let statm = fs::read_to_string(format!("/proc/{pid}/statm"))?;
    let pages: u64 = statm
        .split_whitespace()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .ok_or_else(bad)?;
    // SAFETY: sysconf has no preconditions.
    let page = u64::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).unwrap_or(4096);
    let ticks_per_s = u64::try_from(unsafe { libc::sysconf(libc::_SC_CLK_TCK) }).unwrap_or(100);
    // stat: after the last ')': state ppid pgrp session tty tpgid flags minflt cminflt majflt cmajflt utime stime
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let rest = &stat[stat.rfind(')').ok_or_else(bad)? + 1..];
    let f: Vec<&str> = rest.split_whitespace().collect();
    let ticks: u64 = [11, 12]
        .iter()
        .map(|i| f.get(*i).and_then(|v| v.parse::<u64>().ok()))
        .sum::<Option<u64>>()
        .ok_or_else(bad)?;
    Ok(crate::ProcessStats {
        rss_bytes: pages * page,
        cpu_seconds: ticks as f64 / ticks_per_s.max(1) as f64,
    })
}

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
