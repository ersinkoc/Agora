//! Machines reachable only by SSH (ADR-0032). The hub keeps a reverse tunnel open to each one, so the node on that machine
//! dials the hub exactly like any other node (mTLS, events, acks, routed commands) through `wss://localhost:<port>/`.
//! Nothing about the protocol changes: this module only supervises `ssh -N -R ...` processes.
//!
//! The system `ssh` is used, so keys, agents, `ProxyJump` and `~/.ssh/config` work as the user already set them up. The hub
//! never prompts (`BatchMode`) and never trusts an unknown host key (`StrictHostKeyChecking=yes`).

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The port a tunnel opens on the machine unless its line says otherwise.
pub const DEFAULT_PORT: u16 = 17443;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Machine {
    /// A short name for lists and the status file.
    pub name: String,
    /// What `ssh` is given: `host`, `user@host` or an alias from `~/.ssh/config`.
    pub destination: String,
    /// The loopback port opened on the machine; the node there dials `wss://localhost:<port>/`.
    pub port: u16,
}

fn name_ok(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// A destination cannot become an `ssh` option: no leading `-`, no whitespace or control characters.
fn destination_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 255
        && !s.starts_with('-')
        && !s.chars().any(|c| c.is_whitespace() || c.is_control())
}

impl Machine {
    pub fn new(name: &str, destination: &str, port: u16) -> Option<Self> {
        (name_ok(name) && destination_ok(destination) && port != 0).then(|| Self {
            name: name.to_string(),
            destination: destination.to_string(),
            port,
        })
    }

    /// The line this machine has in the file.
    pub fn line(&self) -> String {
        format!("{} {} {}", self.name, self.destination, self.port)
    }
}

/// Reads `<hub dir>/ssh-machines`: `<name> <destination> [port]` per line. Blank lines and `#` comments are skipped; lines
/// that do not parse (or repeat a name) are counted, not guessed at.
pub fn parse_machines(text: &str) -> (Vec<Machine>, usize) {
    let (mut out, mut skipped): (Vec<Machine>, usize) = (Vec::new(), 0);
    for line in text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let f: Vec<&str> = line.split_whitespace().collect();
        let machine = match f.as_slice() {
            [n, d] => Machine::new(n, d, DEFAULT_PORT),
            [n, d, p] => p.parse().ok().and_then(|p| Machine::new(n, d, p)),
            _ => None,
        };
        match machine {
            Some(m) if !out.iter().any(|o| o.name == m.name) => out.push(m),
            _ => skipped += 1,
        }
    }
    (out, skipped)
}

/// The arguments of the tunnel's `ssh` process. `hub` is the address the hub listens on, as reachable from the hub itself.
pub fn ssh_args(m: &Machine, hub: &str) -> Vec<String> {
    let mut a: Vec<String> = [
        "-N",
        "-T",
        "-o",
        "BatchMode=yes",
        "-o",
        "StrictHostKeyChecking=yes",
        "-o",
        "ExitOnForwardFailure=yes",
        "-o",
        "ServerAliveInterval=15",
        "-o",
        "ServerAliveCountMax=3",
        "-R",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect();
    a.push(format!("127.0.0.1:{}:{hub}", m.port));
    a.push("--".into());
    a.push(m.destination.clone());
    a
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Connecting,
    /// The process has stayed up long enough to have opened its forward.
    Up,
    /// The process ended (or could not start); it is tried again after `retry_ms`.
    Down {
        reason: String,
        retry_ms: u64,
    },
}

/// How restarts are paced.
#[derive(Debug, Clone, Copy)]
pub struct Backoff {
    pub initial: Duration,
    pub max: Duration,
    /// A process that lived this long counts as having worked: the next wait starts from `initial` again.
    pub healthy: Duration,
    /// How long a process must live before the tunnel is called `Up` (`ExitOnForwardFailure` ends failures quickly).
    pub settle: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            initial: Duration::from_secs(1),
            max: Duration::from_secs(60),
            healthy: Duration::from_secs(60),
            settle: Duration::from_secs(3),
        }
    }
}

/// Hears every state change of every tunnel (machine name, state).
pub type OnState = Arc<dyn Fn(&str, &State) + Send + Sync>;

/// Runs one supervised tunnel per machine until dropped.
pub struct Dialer {
    stop: Arc<AtomicBool>,
}

impl Drop for Dialer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

impl Dialer {
    /// Starts a thread per machine. `program` is the `ssh` to run; `on_state` hears every change.
    pub fn start(
        machines: Vec<Machine>,
        hub: &str,
        program: &str,
        backoff: Backoff,
        on_state: OnState,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        for m in machines {
            let (stop, on_state) = (Arc::clone(&stop), Arc::clone(&on_state));
            let (args, program) = (ssh_args(&m, hub), program.to_string());
            std::thread::spawn(move || supervise(&m, &program, &args, backoff, &stop, &*on_state));
        }
        Self { stop }
    }
}

fn nap(total: Duration, stop: &AtomicBool) {
    let end = Instant::now() + total;
    while Instant::now() < end && !stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(20).min(total));
    }
}

fn supervise(
    m: &Machine,
    program: &str,
    args: &[String],
    b: Backoff,
    stop: &AtomicBool,
    on_state: &(dyn Fn(&str, &State) + Send + Sync),
) {
    let mut wait = b.initial;
    while !stop.load(Ordering::SeqCst) {
        on_state(&m.name, &State::Connecting);
        let started = Instant::now();
        let reason = match Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Err(e) => e.to_string(),
            Ok(mut child) => {
                let mut up = false;
                loop {
                    if stop.load(Ordering::SeqCst) {
                        let _ = child.kill();
                        let _ = child.wait();
                        return;
                    }
                    match child.try_wait() {
                        Ok(Some(status)) => break format!("ssh ended ({status})"),
                        Err(e) => break e.to_string(),
                        Ok(None) => {}
                    }
                    if !up && started.elapsed() >= b.settle {
                        up = true;
                        on_state(&m.name, &State::Up);
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        };
        if started.elapsed() >= b.healthy {
            wait = b.initial;
        }
        on_state(
            &m.name,
            &State::Down {
                reason,
                retry_ms: u64::try_from(wait.as_millis()).unwrap_or(u64::MAX),
            },
        );
        nap(wait, stop);
        wait = (wait * 2).min(b.max);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_parsed_strictly_and_a_destination_can_never_be_an_option() {
        let (m, skipped) = parse_machines(
            "# mine\nbox user@box.example\nvps vps 2222\n\n-evil host\nopt -oProxyCommand=x\nbox again\nbad host notaport\nslash a b c d\n",
        );
        assert_eq!(
            m,
            vec![
                Machine {
                    name: "box".into(),
                    destination: "user@box.example".into(),
                    port: DEFAULT_PORT
                },
                Machine {
                    name: "vps".into(),
                    destination: "vps".into(),
                    port: 2222
                },
            ]
        );
        assert_eq!(skipped, 5, "a leading dash, an option-looking destination, a repeated name, a bad port, too many words");
        assert!(Machine::new("a", "x y", 1).is_none());
        assert!(Machine::new("a", "x", 0).is_none());
    }

    #[test]
    fn the_command_line_is_fixed_safe_and_ends_with_the_destination_after_dashes() {
        let m = Machine::new("box", "u@h", 17443).unwrap();
        let a = ssh_args(&m, "127.0.0.1:7443");
        assert!(a.windows(2).any(|w| w == ["-o", "BatchMode=yes"]));
        assert!(a
            .windows(2)
            .any(|w| w == ["-o", "StrictHostKeyChecking=yes"]));
        assert!(a
            .windows(2)
            .any(|w| w == ["-o", "ExitOnForwardFailure=yes"]));
        assert!(a
            .windows(2)
            .any(|w| w == ["-R", "127.0.0.1:17443:127.0.0.1:7443"]));
        assert_eq!(&a[a.len() - 2..], ["--", "u@h"]);
    }

    #[cfg(unix)]
    mod with_a_fake_ssh {
        use super::*;
        use std::sync::Mutex;

        fn fake(dir: &std::path::Path, body: &str) -> String {
            use std::os::unix::fs::PermissionsExt;
            let p = dir.join("ssh");
            std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p.display().to_string()
        }

        fn quick() -> Backoff {
            Backoff {
                initial: Duration::from_millis(20),
                max: Duration::from_millis(80),
                healthy: Duration::from_secs(60),
                settle: Duration::from_millis(100),
            }
        }

        fn tmp(name: &str) -> std::path::PathBuf {
            let d = std::env::temp_dir().join(format!("agora-ssh-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            d
        }

        #[test]
        fn a_tunnel_that_keeps_failing_is_retried_with_growing_waits() {
            let dir = tmp("fail");
            let log = dir.join("log");
            let program = fake(&dir, &format!("echo \"$@\" >> {}\nexit 255", log.display()));
            let seen: Arc<Mutex<Vec<State>>> = Arc::default();
            let s = Arc::clone(&seen);
            let _d = Dialer::start(
                vec![Machine::new("box", "u@h", 17443).unwrap()],
                "127.0.0.1:7443",
                &program,
                quick(),
                Arc::new(move |_, st| s.lock().unwrap().push(st.clone())),
            );
            let t0 = Instant::now();
            while std::fs::read_to_string(&log).map_or(0, |l| l.lines().count()) < 3 {
                assert!(t0.elapsed() < Duration::from_secs(10), "no restarts");
                std::thread::sleep(Duration::from_millis(20));
            }
            let first = std::fs::read_to_string(&log).unwrap();
            assert!(first
                .lines()
                .all(|l| l.contains("-R 127.0.0.1:17443:127.0.0.1:7443 -- u@h")));
            let waits: Vec<u64> = seen
                .lock()
                .unwrap()
                .iter()
                .filter_map(|s| match s {
                    State::Down { retry_ms, .. } => Some(*retry_ms),
                    _ => None,
                })
                .collect();
            assert!(
                waits.len() >= 2 && waits[0] == 20 && waits[1] == 40,
                "{waits:?}"
            );
        }

        #[test]
        fn a_tunnel_that_stays_up_is_reported_up_and_stopped_when_the_dialer_is_dropped() {
            let dir = tmp("up");
            let pid = dir.join("pid");
            let program = fake(&dir, &format!("echo $$ > {}\nexec sleep 30", pid.display()));
            let seen: Arc<Mutex<Vec<State>>> = Arc::default();
            let s = Arc::clone(&seen);
            let d = Dialer::start(
                vec![Machine::new("box", "h", 17443).unwrap()],
                "127.0.0.1:7443",
                &program,
                quick(),
                Arc::new(move |_, st| s.lock().unwrap().push(st.clone())),
            );
            let t0 = Instant::now();
            while !seen.lock().unwrap().contains(&State::Up) {
                assert!(t0.elapsed() < Duration::from_secs(10), "never up");
                std::thread::sleep(Duration::from_millis(20));
            }
            // `Up` means the child stayed alive long enough, not that the fake script has written its pid yet (a slow shell
            // start, seen on macOS): wait for the file too.
            let t0 = Instant::now();
            let pid: String = loop {
                let p = std::fs::read_to_string(&pid).unwrap_or_default();
                if !p.trim().is_empty() {
                    break p.trim().to_string();
                }
                assert!(t0.elapsed() < Duration::from_secs(10), "no pid written");
                std::thread::sleep(Duration::from_millis(20));
            };
            let alive = |p: &str| std::path::Path::new(&format!("/proc/{p}")).exists();
            drop(d);
            // The supervisor kills its child; on systems without /proc the check is skipped.
            if std::path::Path::new("/proc/self").exists() {
                let t0 = Instant::now();
                while alive(&pid) {
                    assert!(
                        t0.elapsed() < Duration::from_secs(10),
                        "ssh was left running"
                    );
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }
    }
}
