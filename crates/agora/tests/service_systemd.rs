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
            .env("AGORA_HOME", self.home())
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
        let path = std::env::join_paths(
            std::iter::once(self.base.join("bin")).chain(
                ["/usr/local/bin", "/usr/bin", "/bin"]
                    .into_iter()
                    .map(PathBuf::from),
            ),
        )
        .unwrap();
        self.run_with_path(Path::new(&path), args)
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
    assert_eq!(
        s.log(),
        "--user is-enabled agora-node.service\n--user is-enabled agora-node.service\n\
         --user daemon-reload\n--user enable agora-node.service\n--user start agora-node.service\n"
    );

    // Again: nothing changes, and it says so.
    let inode = stat("%i", &s.unit());
    let (code, out) = s.run(&["service", "install"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("nothing changed"), "{out}");
    assert_eq!(stat("%i", &s.unit()), inode);

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
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("but not by Agora"), "{out}");
    let (code, out) = s.run(&["service", "remove"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("but not by Agora"), "{out}");
    assert_eq!(std::fs::read_to_string(s.unit()).unwrap(), theirs);
    assert_eq!(s.log().matches("disable").count(), 1, "{}", s.log());
    std::fs::remove_file(s.unit()).unwrap();

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

    // No systemctl at all: not available on this machine.
    let (code, out) = s.run_with_path(&s.base.join("user"), &["service", "status"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("not available on this machine"), "{out}");
}
