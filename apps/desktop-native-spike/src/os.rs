//! What the driver reads about a process from outside: resident memory and CPU time, per operating system. The spike's only
//! OS-specific code; it shells out to the system's own tools (`/proc`, `ps`, PowerShell) instead of adding a dependency. In the
//! product this belongs in `agora-platform` (AGENTS.md); the spike is outside the main workspace and throwaway.

use std::process::Command;

fn out(cmd: &str, args: &[&str]) -> Option<String> {
    let o = Command::new(cmd).args(args).output().ok()?;
    o.status
        .success()
        .then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}

/// Resident memory of one process in bytes.
pub fn rss(pid: u32) -> Option<u64> {
    match std::env::consts::OS {
        "linux" => {
            let s = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
            let line = s.lines().find(|l| l.starts_with("VmRSS:"))?;
            line.split_whitespace()
                .nth(1)?
                .parse::<u64>()
                .ok()
                .map(|kb| kb * 1024)
        }
        "windows" => {
            let s = out(
                "powershell",
                &[
                    "-NoProfile",
                    "-Command",
                    &format!("(Get-Process -Id {pid}).WorkingSet64"),
                ],
            )?;
            s.trim().parse().ok()
        }
        _ => out("ps", &["-o", "rss=", "-p", &pid.to_string()])?
            .trim()
            .parse::<u64>()
            .ok()
            .map(|kb| kb * 1024),
    }
}

/// (pid, parent, resident bytes) of every process, for summing a process tree (the Tauri app's webview helpers).
fn table() -> Vec<(u32, u32, u64)> {
    match std::env::consts::OS {
        "linux" => std::fs::read_dir("/proc")
            .map(|d| {
                d.filter_map(Result::ok)
                    .filter_map(|e| e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()))
                    .filter_map(|pid| {
                        let st = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
                        let after = st.rsplit_once(')')?.1;
                        let ppid = after.split_whitespace().nth(1)?.parse().ok()?;
                        Some((pid, ppid, rss(pid).unwrap_or(0)))
                    })
                    .collect()
            })
            .unwrap_or_default(),
        "windows" => out(
            "powershell",
            &["-NoProfile", "-Command", "Get-CimInstance Win32_Process | ForEach-Object { \"$($_.ProcessId) $($_.ParentProcessId) $($_.WorkingSetSize)\" }"],
        )
        .map(|s| parse_table(&s))
        .unwrap_or_default(),
        _ => out("ps", &["-A", "-o", "pid=,ppid=,rss="]).map(|s| parse_table(&s).into_iter().map(|(a, b, kb)| (a, b, kb * 1024)).collect()).unwrap_or_default(),
    }
}

fn parse_table(s: &str) -> Vec<(u32, u32, u64)> {
    s.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some((
                it.next()?.parse().ok()?,
                it.next()?.parse().ok()?,
                it.next()?.parse().ok()?,
            ))
        })
        .collect()
}

/// Resident memory of a process and all its descendants, in bytes.
pub fn tree_rss(pid: u32) -> u64 {
    let t = table();
    let mut total = 0;
    let mut todo = vec![pid];
    while let Some(p) = todo.pop() {
        for (c, parent, r) in &t {
            if *c == p {
                total += r;
            }
            if *parent == p && *c != p {
                todo.push(*c);
            }
        }
    }
    total
}

/// CPU time (user + system) of a process in milliseconds.
pub fn cpu_ms(pid: u32) -> Option<f64> {
    match std::env::consts::OS {
        "linux" => {
            let st = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
            let f: Vec<&str> = st.rsplit_once(')')?.1.split_whitespace().collect();
            // Fields 14 and 15 (utime, stime) are at 11 and 12 after the command; clock ticks are 100 Hz on Linux.
            let ticks: f64 = f.get(11)?.parse::<f64>().ok()? + f.get(12)?.parse::<f64>().ok()?;
            let hz = out("getconf", &["CLK_TCK"])
                .and_then(|s| s.trim().parse::<f64>().ok())
                .unwrap_or(100.0);
            Some(ticks * 1000.0 / hz)
        }
        "windows" => {
            let s = out(
                "powershell",
                &[
                    "-NoProfile",
                    "-Command",
                    &format!("(Get-Process -Id {pid}).TotalProcessorTime.TotalMilliseconds"),
                ],
            )?;
            s.trim().replace(',', ".").parse().ok()
        }
        _ => {
            // `ps -o time=` prints [[dd-]hh:]mm:ss.cc on macOS.
            let s = out("ps", &["-o", "time=", "-p", &pid.to_string()])?;
            let mut total = 0.0;
            for part in s.trim().split(':') {
                total = total * 60.0 + part.parse::<f64>().ok()?;
            }
            Some(total * 1000.0)
        }
    }
}

/// A short description of the machine for the result file (no host or user names).
pub fn machine() -> serde_json::Value {
    let cpu = match std::env::consts::OS {
        "linux" => std::fs::read_to_string("/proc/cpuinfo").ok().and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("model name"))
                .map(|l| l.split(':').nth(1).unwrap_or("").trim().to_string())
        }),
        "macos" => out("sysctl", &["-n", "machdep.cpu.brand_string"]).map(|s| s.trim().to_string()),
        "windows" => out(
            "powershell",
            &[
                "-NoProfile",
                "-Command",
                "(Get-CimInstance Win32_Processor).Name",
            ],
        )
        .map(|s| s.trim().to_string()),
        _ => None,
    };
    let load = std::fs::read_to_string("/proc/loadavg")
        .ok()
        .map(|s| s.split_whitespace().take(3).collect::<Vec<_>>().join(" "));
    let session = std::env::var("XDG_SESSION_TYPE").ok();
    serde_json::json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "os_version": match std::env::consts::OS {
            "linux" => std::fs::read_to_string("/etc/os-release").ok().and_then(|s| s.lines().find(|l| l.starts_with("PRETTY_NAME=")).map(|l| l[12..].trim_matches('"').to_string())),
            "macos" => out("sw_vers", &["-productVersion"]).map(|s| s.trim().to_string()),
            "windows" => out("cmd", &["/C", "ver"]).map(|s| s.trim().to_string()),
            _ => None,
        },
        "cpu": cpu,
        "logical_cpus": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0),
        "load_average_at_start": load,
        "linux_session_type": session,
        "wayland_display": std::env::var("WAYLAND_DISPLAY").ok(),
        "x11_display": std::env::var("DISPLAY").ok(),
    })
}

/// Drops a file's pages from the page cache where the system allows it without privileges (Linux, `posix_fadvise` through
/// Python, if present). Returns whether it was attempted.
pub fn evict(paths: &[String]) -> bool {
    if std::env::consts::OS != "linux" || paths.is_empty() {
        return false;
    }
    let script = "import os,sys\nfor p in sys.argv[1:]:\n  try:\n    fd=os.open(p,os.O_RDONLY); os.posix_fadvise(fd,0,0,os.POSIX_FADV_DONTNEED); os.close(fd)\n  except Exception: pass\n";
    let mut args = vec!["-I", "-c", script];
    args.extend(paths.iter().map(String::as_str));
    Command::new("python3")
        .args(&args)
        .status()
        .is_ok_and(|s| s.success())
}

/// Files a running process has mapped (Linux), to evict before a cold start.
pub fn mapped_files(pid: u32) -> Vec<String> {
    let Ok(s) = std::fs::read_to_string(format!("/proc/{pid}/maps")) else {
        return Vec::new();
    };
    let mut v: Vec<String> = s
        .lines()
        .filter_map(|l| l.split_whitespace().nth(5))
        .filter(|p| p.starts_with('/'))
        .map(str::to_string)
        .collect();
    v.sort();
    v.dedup();
    v
}
