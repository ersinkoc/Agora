//! ADR-0062 item 6, Linux half: `agora service install|status|remove` writes a systemd user unit and drives `systemctl --user`.
//! Everything is a scratch folder: `HOME`, `XDG_CONFIG_HOME`, `XDG_RUNTIME_DIR` and the Agora folder, with a fake `systemctl` first on
//! `PATH` that records its arguments and answers from files. The real systemctl and the user's own systemd are never reached (the
//! session bus address names nothing, in case a real one were found).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const AGORA: &str = env!("CARGO_BIN_EXE_agora");

const FAKE: &str = r#"#!/bin/sh
d=$(dirname "$0")
printf '%s\n' "$*" >> "$d/log"
mode=$(cat "$d/mode" 2>/dev/null)
if [ "$mode" = bus ]; then
  echo "Failed to connect to bus: \$DBUS_SESSION_BUS_ADDRESS and \$XDG_RUNTIME_DIR not defined" >&2; exit 1
fi
[ "$1" = --user ] || { echo "not --user" >&2; exit 99; }
shift
case "$1" in
  probe) exit 0;;
  is-enabled)
    if [ -f "$XDG_CONFIG_HOME/systemd/user/$2" ]; then
      if [ -e "$d/enabled-$2" ]; then echo enabled; exit 0; fi
      echo disabled; exit 1
    fi
    echo "Failed to get unit file state for $2: No such file or directory" >&2; exit 1;;
  enable) touch "$d/enabled-$2";;
  disable) [ "$2" = --now ] && shift; rm -f "$d/enabled-$2";;
  show)
    u="$6"; dir="$XDG_CONFIG_HOME/systemd/user"; f=""
    [ -f "$dir/$u" ] && f="$dir/$u"
    echo "FragmentPath=$f"
    di=""
    for x in "$dir/$u.d/"*.conf; do [ -e "$x" ] && di="$di $x"; done
    echo "DropInPaths=${di# }";;
  daemon-reload|start) ;;
  *) exit 98;;
esac
"#;

/// `stat -c <format>` of a file (this test runs on Linux only, and the crate has no Unix-only code).
fn stat(format: &str, path: &Path) -> String {
    let out = Command::new("stat")
        .arg("-c")
        .arg(format)
        .arg(path)
        .output()
        .unwrap();
    assert!(out.status.success(), "stat {}", path.display());
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn chmod(mode: &str, path: &Path) {
    assert!(Command::new("chmod")
        .arg(mode)
        .arg(path)
        .status()
        .unwrap()
        .success());
}

/// The words of an `ExecStart=` line as systemd reads the ones Agora writes: each in double quotes, `\\` and `\"` unescaped, `%%` a
/// `%` and `$$` a `$`.
fn exec_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '"' {
            assert_eq!(c, ' ', "unquoted text in {line}");
            continue;
        }
        let mut w = String::new();
        loop {
            match chars.next().expect("a closing quote") {
                '"' => break,
                '\\' => w.push(chars.next().unwrap()),
                '%' | '$' => {
                    let d = chars.next().unwrap();
                    assert!(
                        d == '%' || d == '$',
                        "a lone specifier or variable in {line}"
                    );
                    w.push(d);
                }
                c => w.push(c),
            }
        }
        words.push(w);
    }
    words
}

struct Scratch {
    base: PathBuf,
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

impl Scratch {
    fn new() -> Scratch {
        let base = std::env::temp_dir().join(format!("agsvc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        for d in ["bin", "user", "cfg", "run", "agora 50% $HOME home"] {
            std::fs::create_dir_all(base.join(d)).unwrap();
        }
        chmod("700", &base.join("run"));
        let script = base.join("bin/systemctl");
        std::fs::write(&script, FAKE).unwrap();
        chmod("755", &script);
        // A thread forking while the script was open for writing can keep it busy a moment (ETXTBSY).
        for _ in 0..100 {
            if Command::new(&script)
                .args(["--user", "probe"])
                .status()
                .is_ok()
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        std::fs::write(base.join("bin/log"), "").unwrap();
        std::fs::write(
            base.join("agora 50% $HOME home/machine.toml"),
            "roles = [\"node\"]\nui = \"none\"\n",
        )
        .unwrap();
        Scratch { base }
    }

    fn home(&self) -> PathBuf {
        self.base.join("agora 50% $HOME home")
    }

    fn unit(&self) -> PathBuf {
        self.base.join("cfg/systemd/user/agora-node.service")
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.base.join("bin/log")).unwrap()
    }

    fn run_with_path(&self, path: &Path, args: &[&str]) -> (i32, String) {
        self.run_full(path, &self.home(), args)
    }

    fn run_in(&self, home: &Path, args: &[&str]) -> (i32, String) {
        self.run_full(&self.path(), home, args)
    }

    fn run_full(&self, path: &Path, home: &Path, args: &[&str]) -> (i32, String) {
        let out = Command::new(AGORA)
            .args(args)
            .env_clear()
            .env("PATH", path)
            .env("HOME", self.base.join("user"))
            .env("XDG_CONFIG_HOME", self.base.join("cfg"))
            .env("XDG_RUNTIME_DIR", self.base.join("run"))
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                "unix:path=/nonexistent/agora-test-bus",
            )
            .env("AGORA_HOME", home)
            .env("AGORA_LANG", "en")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        (
            out.status.code().unwrap_or(-1),
            format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
        )
    }

    fn run(&self, args: &[&str]) -> (i32, String) {
        self.run_with_path(&self.path(), args)
    }

    fn path(&self) -> PathBuf {
        let path = std::env::join_paths(
            std::iter::once(self.base.join("bin")).chain(
                ["/usr/local/bin", "/usr/bin", "/bin"]
                    .into_iter()
                    .map(PathBuf::from),
            ),
        )
        .unwrap();
        PathBuf::from(path)
    }
}

#[test]
fn service_install_status_remove_drive_a_user_unit_through_systemctl() {
    if std::env::consts::OS != "linux" {
        return;
    }
    let s = Scratch::new();

    let (code, out) = s.run(&["service", "status"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("does not start by itself"), "{out}");

    let (code, out) = s.run(&["service", "install"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("Registering \"agora-node\""), "{out}");
    assert!(out.contains("and it was started now"), "{out}");
    assert!(out.contains("loginctl enable-linger"), "{out}");
    let unit = std::fs::read_to_string(s.unit()).unwrap();
    assert_eq!(stat("%a", &s.unit()), "600");
    let home = s.home().canonicalize().unwrap();
    let home_word = home.to_str().unwrap().replace('%', "%%").replace('$', "$$");
    assert!(
        unit.contains(&format!(
            "ExecStart=\"{AGORA}\" \"supervise\" \"node\" \"--home\" \"{home_word}\" \"--log\"\n"
        )),
        "{unit}"
    );
    assert!(
        unit.contains(&format!(
            "Environment=\"PATH={}:/usr/local/bin:/usr/bin:/bin\"\n",
            s.base.join("bin").display()
        )),
        "{unit}"
    );
    assert!(unit.contains("Restart=on-failure\n") && unit.contains("TimeoutStopSec=25\n"));
    // The node's unit ends only the supervisor on stop, so the holder and its agents survive (ADR-0079).
    assert!(
        unit.contains("KillMode=process\n") && !unit.contains("control-group"),
        "{unit}"
    );
    assert_eq!(
        s.log(),
        "--user is-enabled agora-node.service\n--user is-enabled agora-node.service\n\
         --user daemon-reload\n--user show -p FragmentPath -p DropInPaths agora-node.service\n\
         --user enable agora-node.service\n--user start agora-node.service\n"
    );

    // The unit's own command, read back as systemd reads it, is the supervisor with this folder; a planned end of it (here: the
    // folder is not set up) exits 0, which `Restart=on-failure` never restarts, so it cannot loop (#564).
    let exec = unit
        .lines()
        .find_map(|l| l.strip_prefix("ExecStart="))
        .unwrap();
    let words = exec_words(exec);
    assert_eq!(
        words,
        [
            AGORA,
            "supervise",
            "node",
            "--home",
            home.to_str().unwrap(),
            "--log"
        ]
    );
    let toml = s.home().join("machine.toml");
    std::fs::rename(&toml, s.base.join("machine.toml.aside")).unwrap();
    let ended = Command::new(&words[0])
        .args(&words[1..])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("AGORA_LANG", "en")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    std::fs::rename(s.base.join("machine.toml.aside"), &toml).unwrap();
    assert_eq!(
        ended.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&ended.stderr)
    );
    assert!(unit.contains("Restart=on-failure\n") && !unit.contains("Restart=always"));

    // Again: nothing changes, and it says so.
    let inode = stat("%i", &s.unit());
    let (code, out) = s.run(&["service", "install"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("nothing changed"), "{out}");
    assert_eq!(stat("%i", &s.unit()), inode);

    // Turned off outside Agora (`systemctl --user disable`): install never says "already starts"; it turns it on again and says so.
    std::fs::remove_file(s.base.join("bin/enabled-agora-node.service")).unwrap();
    let (code, out) = s.run(&["service", "status"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("turned off"), "{out}");
    let (code, out) = s.run(&["service", "install"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("turning it on again"), "{out}");
    assert!(!out.contains("already starts"), "{out}");
    assert!(s.base.join("bin/enabled-agora-node.service").exists());
    assert_eq!(stat("%i", &s.unit()), inode);

    // A drop-in added after the install (in a folder with a space): status names it, quoted and whole, and install says it is there
    // instead of "already".
    let late = s.base.join("cfg/systemd/user/agora-node.service.d");
    std::fs::create_dir_all(&late).unwrap();
    std::fs::write(late.join("my late.conf"), "[Service]\nExecStart=\n").unwrap();
    let quoted = format!("\"{}\"", late.join("my late.conf").display());
    let (code, out) = s.run(&["service", "status"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("systemd also reads") && out.contains(&quoted),
        "{out}"
    );
    let (code, out) = s.run(&["service", "install"]);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains("but systemd also reads") && out.contains(&quoted),
        "{out}"
    );
    assert!(!out.contains("nothing changed;"), "{out}");
    std::fs::remove_dir_all(&late).unwrap();

    let (code, out) = s.run(&["service", "status"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("starts when you log in (\"agora-node\")"),
        "{out}"
    );
    assert!(out.contains("not running now"), "{out}");

    let (code, out) = s.run(&["service", "remove"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("\"agora-node\" is removed"), "{out}");
    assert!(!s.unit().exists());
    assert!(s
        .log()
        .contains("--user disable --now agora-node.service\n--user daemon-reload\n"));

    let (code, out) = s.run(&["service", "remove"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("was not set to start"), "{out}");

    // A unit of the same name written by hand (as on the maintainer's machine) is left exactly as it is: not replaced, not stopped,
    // not removed.
    let theirs =
        "[Unit]\nDescription=Local agent node\n\n[Service]\nExecStart=/usr/bin/agora node run\n";
    std::fs::write(s.unit(), theirs).unwrap();
    std::fs::write(s.base.join("bin/enabled-agora-node.service"), "").unwrap();
    let (code, out) = s.run(&["service", "install"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("but not by Agora"), "{out}");
    let (code, out) = s.run(&["service", "remove"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("but not by Agora"), "{out}");
    assert_eq!(std::fs::read_to_string(s.unit()).unwrap(), theirs);
    assert_eq!(s.log().matches("disable").count(), 1, "{}", s.log());
    std::fs::remove_file(s.unit()).unwrap();
    std::fs::remove_file(s.base.join("bin/enabled-agora-node.service")).unwrap();

    // The hand-made unit is gone but its drop-in stays: it would change what Agora's unit runs, so nothing is registered.
    let drop_in_dir = s.base.join("cfg/systemd/user/agora-node.service.d");
    std::fs::create_dir_all(&drop_in_dir).unwrap();
    let drop_in = drop_in_dir.join("10-local-build.conf");
    std::fs::write(
        &drop_in,
        "[Service]\nExecStart=\nExecStart=/usr/bin/agora node run\n",
    )
    .unwrap();
    let (code, out) = s.run(&["service", "install"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("would not run only what Agora wrote"), "{out}");
    assert!(out.contains("10-local-build.conf"), "{out}");
    assert!(!s.unit().exists(), "the refused unit was left behind");
    // The first install and the one that turned it on again; the refused one enabled nothing.
    assert_eq!(
        s.log().matches("enable agora-node").count(),
        2,
        "{}",
        s.log()
    );
    // A drop-in that appears after the install is named by remove, and left.
    std::fs::rename(&drop_in, s.base.join("aside.conf")).unwrap();
    let (code, out) = s.run(&["service", "install"]);
    assert_eq!(code, 0, "{out}");
    std::fs::rename(s.base.join("aside.conf"), &drop_in).unwrap();
    let (code, out) = s.run(&["service", "remove"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("were not written by Agora and are left"),
        "{out}"
    );
    assert!(drop_in.exists() && !s.unit().exists());
    std::fs::remove_dir_all(&drop_in_dir).unwrap();

    // Never anything but the user's own manager: no root, no lingering.
    assert!(
        s.log().lines().all(|l| l.starts_with("--user ")),
        "{}",
        s.log()
    );

    // No user bus in this session: a clear error, never "not installed".
    std::fs::write(s.base.join("bin/mode"), "bus").unwrap();
    let (code, out) = s.run(&["service", "status"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("no systemd user bus"), "{out}");
    assert!(!out.contains("does not start by itself"), "{out}");
    let (code, out) = s.run(&["service", "install"]);
    assert_eq!(code, 1, "{out}");
    assert!(!s.unit().exists());

    std::fs::remove_file(s.base.join("bin/mode")).unwrap();

    // A folder ending in a backslash would break the unit (a line continuation): refused, nothing written.
    let odd = s.base.join("odd\\");
    std::fs::create_dir_all(&odd).unwrap();
    std::fs::write(
        odd.join("machine.toml"),
        "roles = [\"node\"]\nui = \"none\"\n",
    )
    .unwrap();
    let (code, out) = s.run_in(&odd, &["service", "install"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("backslash"), "{out}");
    assert!(!s.unit().exists());

    // No systemctl at all: not available on this machine.
    let (code, out) = s.run_with_path(&s.base.join("user"), &["service", "status"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("not available on this machine"), "{out}");
}
