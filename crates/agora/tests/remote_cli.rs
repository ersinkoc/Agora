//! Reaching the hub from outside this machine (ADR-0067 rung 3), with real `agora` processes and a fake `tailscale` program on the
//! path. The real `tailscale` is never run and no real tailnet is touched: the fake is a shell script that keeps its rules in a folder
//! and writes every call it gets to a log. The shell-script tests run on Linux and macOS; the checks that need no `tailscale` run
//! everywhere.
#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg_attr(not(unix), allow(unused_imports, dead_code))]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use agora_hub::store::SqliteStore;

const AGORA: &str = env!("CARGO_BIN_EXE_agora");

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("agora-remote-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A throwaway Agora home with a hub folder, and the fake `tailscale` folder.
struct Env {
    home: PathBuf,
    hub: PathBuf,
    fake: PathBuf,
    path: std::ffi::OsString,
}

#[cfg(unix)]
const FAKE: &str = r#"#!/bin/sh
D="$FAKE_TS_DIR"
echo "$@" >> "$D/calls"
serve_json() {
  printf '{"TCP":{'; first=1
  for f in "$D"/rules/*; do [ -e "$f" ] || continue; first_=$first; first=0
    [ "$first_" = 1 ] || printf ','; printf '"%s":{"HTTPS":true}' "$(basename "$f")"; done
  printf '},"Web":{'; first=1
  for f in "$D"/rules/*; do [ -e "$f" ] || continue; first_=$first; first=0
    [ "$first_" = 1 ] || printf ','
    printf '"box.tail1234.ts.net:%s":{"Handlers":{"/":{"Proxy":"%s"}}}' "$(basename "$f")" "$(cat "$f")"; done
  printf '}'
  if [ -e "$D/funnel" ]; then printf ',"AllowFunnel":{"box.tail1234.ts.net:8443":true}'; fi
  printf '}\n'
}
case "$1" in
  status)
    if [ -e "$D/huge" ]; then head -c 20000000 /dev/zero | tr '\0' 'a'; exit 0; fi
    cat "$D/status.json"; exit 0 ;;
  serve)
    case "$2" in
      status) serve_json; exit 0 ;;
      --bg) port=${3#--https=}; mkdir -p "$D/rules"; printf '%s' "$4" > "$D/rules/$port"; exit 0 ;;
      --https=*) port=${2#--https=}; [ "$3" = off ] && rm -f "$D/rules/$port"; exit 0 ;;
    esac ;;
esac
exit 1
"#;

const GOOD: &str = r#"{"BackendState":"Running","Self":{"DNSName":"Box.tail1234.ts.net.","TailscaleIPs":["100.101.102.103","fd7a:115c:a1e0::1"]},"Peer":{}}"#;

impl Env {
    fn new(name: &str) -> Self {
        let root = tmp(name);
        let (home, hub, fake) = (root.join("home"), root.join("hub"), root.join("fake"));
        std::fs::create_dir_all(&hub).unwrap();
        std::fs::create_dir_all(fake.join("rules")).unwrap();
        std::fs::create_dir_all(fake.join("bin")).unwrap();
        std::fs::write(fake.join("status.json"), GOOD).unwrap();
        // A running hub on loopback (the file the hub writes at start).
        std::fs::write(hub.join("listen"), "127.0.0.1:7443").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let exe = fake.join("bin").join("tailscale");
            std::fs::write(&exe, FAKE).unwrap();
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = std::env::join_paths([
            fake.join("bin"),
            PathBuf::from("/usr/bin"),
            PathBuf::from("/bin"),
        ])
        .unwrap();
        Self {
            home,
            hub,
            fake,
            path,
        }
    }

    /// `agora ARGS --hub-dir ...` with the fake `tailscale` first on the path.
    fn agora(&self, args: &[&str]) -> Command {
        let mut c = Command::new(AGORA);
        c.args(args)
            .arg("--hub-dir")
            .arg(&self.hub)
            .env("AGORA_HOME", &self.home)
            .env("AGORA_LANG", "en")
            .env("PATH", &self.path)
            .env("FAKE_TS_DIR", &self.fake)
            .stdin(Stdio::null());
        c
    }

    fn run(&self, args: &[&str]) -> (i32, String) {
        let o: Output = self.agora(args).output().unwrap();
        (
            o.status.code().unwrap_or(-1),
            format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            ),
        )
    }

    fn set_password(&self) {
        let store = SqliteStore::open(&self.hub.join("hub.db")).unwrap();
        agora_hub::password::ScreenPassword::with_iterations(&store, 1_000)
            .set("long enough pw")
            .unwrap();
    }

    fn password_is_set(&self) -> bool {
        let store = SqliteStore::open(&self.hub.join("hub.db")).unwrap();
        agora_hub::password::ScreenPassword::new(&store).is_set()
    }

    fn calls(&self) -> String {
        std::fs::read_to_string(self.fake.join("calls")).unwrap_or_default()
    }

    fn names_file(&self) -> Option<String> {
        std::fs::read_to_string(self.hub.join("remote-names")).ok()
    }
}

fn nothing_written(e: &Env) {
    assert!(e.names_file().is_none(), "a name was written");
    assert!(!e.hub.join("remote-serve").exists());
}

#[test]
fn nothing_from_outside_is_turned_on_without_a_screen_password() {
    let e = Env::new("nopw");
    for args in [
        &["remote", "tailscale"][..],
        &["remote", "tailscale", "--serve"],
    ] {
        let (code, out) = e.run(args);
        assert_eq!(code, 1, "{out}");
        assert!(out.contains("remote.password_required"), "{out}");
        assert!(out.contains("agora password set"), "{out}");
    }
    nothing_written(&e);
    assert_eq!(e.calls(), "", "tailscale was not even asked");
    // A hub folder with a database and still no password is the same.
    drop(SqliteStore::open(&e.hub.join("hub.db")).unwrap());
    let (code, out) = e.run(&["remote", "tailscale", "--serve"]);
    assert_eq!(
        (code, out.contains("remote.password_required")),
        (1, true),
        "{out}"
    );
    nothing_written(&e);
}

#[test]
fn a_machine_without_tailscale_is_told_so_and_nothing_changes() {
    let e = Env::new("missing");
    e.set_password();
    let empty = tmp("missing-empty-path");
    let mut c = e.agora(&["remote", "tailscale", "--serve"]);
    c.env("PATH", &empty);
    let o = c.output().unwrap();
    let out = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(1), "{out}");
    assert!(out.contains("remote.tailscale_missing"), "{out}");
    nothing_written(&e);
}

#[cfg(unix)]
#[test]
fn a_tailnet_machine_gets_its_names_and_a_serve_rule_and_a_url() {
    let e = Env::new("good");
    e.set_password();
    let (code, out) = e.run(&["remote", "tailscale", "--serve"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("https://box.tail1234.ts.net:8443/"), "{out}");
    assert_eq!(
        e.names_file().unwrap(),
        "box.tail1234.ts.net\n100.101.102.103\nfd7a:115c:a1e0::1\n"
    );
    assert!(
        e.calls()
            .contains("serve --bg --https=8443 https+insecure://127.0.0.1:7443"),
        "{}",
        e.calls()
    );
    assert!(e.hub.join("remote-serve").exists());
    // `--port` picks the port.
    let (code, out) = e.run(&["remote", "tailscale", "--serve", "--port", "9443"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("https://box.tail1234.ts.net:9443/"), "{out}");
}

#[cfg(unix)]
#[test]
fn without_serve_only_the_names_are_added() {
    let e = Env::new("namesonly");
    e.set_password();
    let (code, out) = e.run(&["remote", "tailscale"]);
    assert_eq!(code, 0, "{out}");
    assert!(e.names_file().is_some());
    assert!(!e.calls().contains("serve"), "{}", e.calls());
    assert!(!e.hub.join("remote-serve").exists());
}

#[cfg(unix)]
#[test]
fn a_signed_out_garbled_or_huge_answer_changes_nothing() {
    let e = Env::new("bad");
    e.set_password();
    let cases: [(&str, &str); 6] = [
        (
            r#"{"BackendState":"NeedsLogin","Self":{}}"#,
            "remote.tailscale_logged_out",
        ),
        (
            r#"{"BackendState":"Stopped"}"#,
            "remote.tailscale_unreadable",
        ),
        (
            "this is not json at all \u{1}\u{2}",
            "remote.tailscale_unreadable",
        ),
        ("", "remote.tailscale_unreadable"),
        (
            r#"{"BackendState":"Running","Self":{"DNSName":"","TailscaleIPs":[]}}"#,
            "remote.tailscale_unreadable",
        ),
        (
            r#"[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[["#,
            "remote.tailscale_unreadable",
        ),
    ];
    for (json, code_word) in cases {
        std::fs::write(e.fake.join("status.json"), json).unwrap();
        for args in [
            &["remote", "tailscale"][..],
            &["remote", "tailscale", "--serve"],
        ] {
            let (code, out) = e.run(args);
            assert_eq!(code, 1, "{json:.30}: {out}");
            assert!(out.contains(code_word), "{json:.30}: {out}");
            nothing_written(&e);
        }
    }
    assert!(!e.calls().contains("serve"), "{}", e.calls());
    // More output than the limit: stopped, and said so, within a few seconds.
    std::fs::write(e.fake.join("huge"), "").unwrap();
    let started = Instant::now();
    let (code, out) = e.run(&["remote", "tailscale", "--serve"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("too_big"), "{out}");
    assert!(started.elapsed() < Duration::from_secs(10));
    nothing_written(&e);
}

#[cfg(unix)]
#[test]
fn a_hostile_name_in_the_status_never_reaches_a_certificate() {
    let e = Env::new("hostile");
    e.set_password();
    for dns in [
        "*.tail1234.ts.net.",
        "evil.example.com.",
        "box.tail1234.ts.net.evil.example.",
        "bo\\u202ex.tail1234.ts.net.",
        "bo\\nx.tail1234.ts.net.",
        "bo\\u0000x.tail1234.ts.net.",
        "b o.tail1234.ts.net.",
        "-x.tail1234.ts.net.",
        "x..tail1234.ts.net.",
        "100.64.0.1",
    ] {
        std::fs::write(
            e.fake.join("status.json"),
            format!(
                r#"{{"BackendState":"Running","Self":{{"DNSName":"{dns}","TailscaleIPs":["100.101.102.103"]}}}}"#
            ),
        )
        .unwrap();
        let (code, out) = e.run(&["remote", "tailscale", "--serve"]);
        assert_eq!(code, 1, "{dns}: {out}");
        assert!(out.contains("bad_name"), "{dns}: {out}");
        nothing_written(&e);
    }
    let long = format!("{}.tail1234.ts.net.", "a".repeat(80));
    std::fs::write(
        e.fake.join("status.json"),
        format!(r#"{{"BackendState":"Running","Self":{{"DNSName":"{long}"}}}}"#),
    )
    .unwrap();
    assert_eq!(e.run(&["remote", "tailscale"]).0, 1);
    nothing_written(&e);
    // Addresses that are not the tailnet's are dropped, the rest is used.
    std::fs::write(
        e.fake.join("status.json"),
        r#"{"BackendState":"Running","Self":{"DNSName":"box.tail1234.ts.net.","TailscaleIPs":["8.8.8.8","10.0.0.1","100.64.0.5"]}}"#,
    )
    .unwrap();
    assert_eq!(e.run(&["remote", "tailscale"]).0, 0);
    assert_eq!(e.names_file().unwrap(), "box.tail1234.ts.net\n100.64.0.5\n");
}

#[cfg(unix)]
#[test]
fn a_port_that_is_someone_elses_is_left_alone() {
    let e = Env::new("busy");
    e.set_password();
    std::fs::write(e.fake.join("rules").join("8443"), "http://127.0.0.1:3000").unwrap();
    let (code, out) = e.run(&["remote", "tailscale", "--serve"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("remote.serve_port_in_use"), "{out}");
    assert!(!e.calls().contains("--bg"), "{}", e.calls());
    nothing_written(&e);
    assert_eq!(
        std::fs::read_to_string(e.fake.join("rules").join("8443")).unwrap(),
        "http://127.0.0.1:3000"
    );
}

#[cfg(unix)]
#[test]
fn remote_off_removes_only_the_rule_agora_added() {
    let e = Env::new("off");
    e.set_password();
    // The person's own rules: one on 443, and one on another port.
    std::fs::write(e.fake.join("rules").join("443"), "http://127.0.0.1:3000").unwrap();
    std::fs::write(e.fake.join("rules").join("9000"), "http://127.0.0.1:9001").unwrap();
    assert_eq!(e.run(&["remote", "tailscale", "--serve"]).0, 0);
    assert!(e.fake.join("rules").join("8443").exists());
    let (code, out) = e.run(&["remote", "off"]);
    assert_eq!(code, 0, "{out}");
    assert!(!e.fake.join("rules").join("8443").exists(), "{out}");
    assert!(e.fake.join("rules").join("443").exists());
    assert!(e.fake.join("rules").join("9000").exists());
    nothing_written(&e);
    let offs: Vec<_> = e
        .calls()
        .lines()
        .filter(|l| l.contains(" off"))
        .map(String::from)
        .collect();
    assert_eq!(
        offs,
        ["serve --https=8443 off"],
        "exactly one rule was turned off"
    );
    // A second `off` has nothing to do and touches nothing.
    let before = e.calls();
    let (code, out) = e.run(&["remote", "off"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("has not turned on"), "{out}");
    assert_eq!(e.calls(), before);
}

#[cfg(unix)]
#[test]
fn remote_off_leaves_a_rule_the_person_changed() {
    let e = Env::new("offforeign");
    e.set_password();
    assert_eq!(e.run(&["remote", "tailscale", "--serve"]).0, 0);
    // The person pointed the port at something else after Agora made the rule.
    std::fs::write(e.fake.join("rules").join("8443"), "http://127.0.0.1:3000").unwrap();
    let (code, out) = e.run(&["remote", "off"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("left exactly as it is"), "{out}");
    assert!(e.fake.join("rules").join("8443").exists());
    assert!(!e.calls().contains("--https=8443 off"), "{}", e.calls());
    assert!(!e.hub.join("remote-serve").exists());
}

#[test]
fn remote_off_without_tailscale_keeps_the_record_and_says_so() {
    let e = Env::new("offmissing");
    std::fs::write(
        e.hub.join("remote-serve"),
        "port=8443\ntarget=https+insecure://127.0.0.1:7443\nname=box.tail1234.ts.net\n",
    )
    .unwrap();
    let empty = tmp("offmissing-empty");
    let mut c = e.agora(&["remote", "off"]);
    c.env("PATH", &empty);
    let o = c.output().unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(
        e.hub.join("remote-serve").exists(),
        "the record stays so the next off can finish"
    );
}

#[test]
fn status_says_what_is_reachable_how_and_whether_a_password_is_set() {
    let e = Env::new("status");
    std::fs::remove_file(e.hub.join("listen")).unwrap();
    let (code, out) = e.run(&["remote", "status"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("No running hub"), "{out}");
    assert!(out.contains("Screen password: not set"), "{out}");
    assert!(!out.contains("no screen password is set"), "{out}");

    std::fs::write(e.hub.join("listen"), "127.0.0.1:7443").unwrap();
    let (_, out) = e.run(&["remote", "status"]);
    assert!(out.contains("Local network: not reachable"), "{out}");
    assert!(out.contains("Public internet: not reachable"), "{out}");
    assert!(!out.contains("public address"), "{out}");

    // A hub on a public address, no password: the way in is named, the missing password is said, the warning stays.
    std::fs::write(e.hub.join("listen"), "203.0.113.7:7443").unwrap();
    let (_, out) = e.run(&["remote", "status"]);
    assert!(out.contains("Public internet: reachable"), "{out}");
    assert!(out.contains("no screen password is set"), "{out}");
    assert!(out.contains("agora password set"), "{out}");
    assert!(out.contains("listens on a public address"), "{out}");
    e.set_password();
    let (_, out) = e.run(&["remote", "status"]);
    assert!(out.contains("Screen password: set"), "{out}");
    assert!(!out.contains("no screen password is set"), "{out}");
    assert!(
        out.contains("listens on a public address"),
        "the warning is persistent: {out}"
    );

    // A tailnet address.
    std::fs::write(e.hub.join("listen"), "100.64.0.9:7443").unwrap();
    std::fs::write(e.hub.join("remote-names"), "box.tail1234.ts.net\n").unwrap();
    let (_, out) = e.run(&["remote", "status"]);
    assert!(out.contains("Tailscale: reachable"), "{out}");
    assert!(out.contains("box.tail1234.ts.net"), "{out}");
}

#[test]
fn a_hub_is_not_started_on_a_public_address_without_the_password() {
    let e = Env::new("publicbind");
    for listen in ["203.0.113.7:7444", "[2001:db8::7]:7444"] {
        let (code, out) = e.run(&["hub", "run", "--listen", listen]);
        assert_eq!(code, 1, "{listen}: {out}");
        assert!(out.contains("remote.password_required"), "{listen}: {out}");
        assert!(out.contains("agora password set"), "{out}");
    }
    // With the password the rule lets it through to binding (an address this machine does not own then fails on its own).
    e.set_password();
    let (code, out) = e.run(&["hub", "run", "--listen", "203.0.113.7:7444"]);
    assert_eq!(code, 1, "{out}");
    assert!(!out.contains("remote.password_required"), "{out}");
}

#[test]
fn the_password_is_not_cleared_while_a_way_in_is_on() {
    let e = Env::new("clear");
    e.set_password();
    std::fs::write(e.hub.join("remote-names"), "box.tail1234.ts.net\n").unwrap();
    let (code, out) = e.run(&["password", "clear"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("remote.password_in_use"), "{out}");
    assert!(e.password_is_set());
    std::fs::remove_file(e.hub.join("remote-names")).unwrap();
    let (code, out) = e.run(&["password", "clear"]);
    assert_eq!(code, 0, "{out}");
    assert!(!e.password_is_set());
}

#[test]
fn a_hub_address_that_is_not_known_is_never_guessed() {
    let e = Env::new("nolisten");
    e.set_password();
    std::fs::remove_file(e.hub.join("listen")).unwrap();
    let (code, out) = e.run(&["remote", "tailscale", "--serve"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("remote.hub_not_running"), "{out}");
    nothing_written(&e);
    assert_eq!(e.calls(), "");
}

#[cfg(unix)]
#[test]
fn agoras_own_rule_is_updated_when_the_hub_moved_and_another_ones_is_not() {
    let e = Env::new("moved");
    e.set_password();
    assert_eq!(e.run(&["remote", "tailscale", "--serve"]).0, 0);
    // The hub moved to another port (a restart on a new one).
    std::fs::write(e.hub.join("listen"), "127.0.0.1:7555").unwrap();
    let (code, out) = e.run(&["remote", "tailscale", "--serve"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        std::fs::read_to_string(e.fake.join("rules").join("8443")).unwrap(),
        "https+insecure://127.0.0.1:7555"
    );
    let record = std::fs::read_to_string(e.hub.join("remote-serve")).unwrap();
    assert!(record.contains("7555"), "{record}");
}

#[cfg(unix)]
#[test]
fn funnel_is_called_by_its_name_and_counts_as_public() {
    let e = Env::new("funnel");
    e.set_password();
    assert_eq!(e.run(&["remote", "tailscale", "--serve"]).0, 0);
    let (_, out) = e.run(&["remote", "status"]);
    assert!(out.contains("Public internet: not reachable"), "{out}");
    std::fs::write(e.fake.join("funnel"), "").unwrap();
    let (_, out) = e.run(&["remote", "status"]);
    assert!(
        out.contains("Public internet: reachable through Tailscale Funnel"),
        "{out}"
    );
    assert!(out.contains("listens on a public address"), "{out}");
    let (_, out) = e.run(&["doctor"]);
    assert!(out.contains("Tailscale Funnel is on"), "{out}");
    let (code, out) = e.run(&["password", "clear"]);
    assert_eq!(code, 1, "{out}");
}

#[test]
fn the_password_stays_only_for_a_way_in_that_was_turned_on_and_says_the_real_way_out() {
    // A tailnet or wildcard listen alone is warned about, not a reason to refuse: nothing Agora set up depends on the password.
    let e = Env::new("clearbranches");
    e.set_password();
    std::fs::write(e.hub.join("listen"), "100.64.0.9:7443").unwrap();
    let (_, out) = e.run(&["remote", "status"]);
    assert!(out.contains("Tailscale: reachable"), "{out}");
    let (code, out) = e.run(&["password", "clear"]);
    assert_eq!(code, 0, "{out}");
    assert!(!e.password_is_set());
    // `remote status` and doctor still warn about it.
    let (_, out) = e.run(&["remote", "status"]);
    assert!(out.contains("no screen password is set"), "{out}");
    let (_, out) = e.run(&["doctor"]);
    assert!(
        out.contains("can be reached over Tailscale and no screen password is set"),
        "{out}"
    );
    // A public listen: refused, and the way out is the hub's address, not `remote off`.
    e.set_password();
    std::fs::write(e.hub.join("listen"), "203.0.113.7:7443").unwrap();
    let (code, out) = e.run(&["password", "clear"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("Restart the hub on a local address"), "{out}");
    assert!(!out.contains("agora remote off"), "{out}");
    assert!(e.password_is_set());
    // A recorded name: refused, and `remote off` is the way out (and it works).
    std::fs::write(e.hub.join("listen"), "127.0.0.1:7443").unwrap();
    std::fs::write(e.hub.join("remote-names"), "box.tail1234.ts.net\n").unwrap();
    let (code, out) = e.run(&["password", "clear"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("agora remote off"), "{out}");
    let (code, _) = e.run(&["remote", "off"]);
    assert_eq!(code, 0);
    assert_eq!(e.run(&["password", "clear"]).0, 0);
}

#[test]
fn a_listen_address_that_is_not_an_address_is_a_usage_error() {
    let e = Env::new("badlisten");
    let (code, _) = e.run(&["--listen", "not an address"]);
    assert_eq!(code, 2);
}

#[cfg(unix)]
struct Proc(Child);

#[cfg(unix)]
impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(unix)]
fn wait_until(what: &str, mut ok: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(30);
    while !ok() {
        assert!(Instant::now() < end, "never: {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(unix)]
fn cert_names(hub: &Path) -> Vec<String> {
    std::fs::read_to_string(hub.join("server-cert.pem"))
        .ok()
        .and_then(|p| agora_hub::ca::cert_names(&p).ok())
        .unwrap_or_default()
}

/// The names of `agora remote tailscale` reach the running hub's certificate (from its own CA), `agora doctor` sees them, and a name
/// is not taken out of the certificate by `remote off` before the hub restarts.
#[cfg(unix)]
#[test]
fn the_tailnet_names_reach_the_running_hubs_certificate_and_doctor_sees_them() {
    let e = Env::new("leaf");
    e.set_password();
    let mut hub = e.agora(&["hub", "run", "--listen", "127.0.0.1:0"]);
    hub.stdout(Stdio::null()).stderr(Stdio::null());
    let _hub = Proc(hub.spawn().unwrap());
    wait_until("the hub wrote its certificate", || {
        !cert_names(&e.hub).is_empty() && e.hub.join("listen").exists()
    });
    assert!(!cert_names(&e.hub).contains(&"box.tail1234.ts.net".to_string()));
    let ca_before = std::fs::read(e.hub.join("ca.pem")).unwrap();

    let (code, out) = e.run(&["remote", "tailscale", "--serve"]);
    assert_eq!(code, 0, "{out}");
    // The hub listens on a random loopback port: the rule points at the port it really bound.
    let bound = std::fs::read_to_string(e.hub.join("listen")).unwrap();
    assert!(
        e.calls()
            .contains(&format!("https+insecure://{}", bound.trim())),
        "{}",
        e.calls()
    );
    wait_until("the names reached the certificate", || {
        let n = cert_names(&e.hub);
        n.contains(&"box.tail1234.ts.net".to_string()) && n.contains(&"100.101.102.103".to_string())
    });
    assert_eq!(
        std::fs::read(e.hub.join("ca.pem")).unwrap(),
        ca_before,
        "the same CA"
    );

    let (_, out) = e.run(&["doctor"]);
    assert!(
        out.contains("Tailscale works; this machine is box.tail1234.ts.net"),
        "{out}"
    );
    assert!(out.contains("a screen password protects it"), "{out}");
    assert!(!out.contains("does not name"), "{out}");

    // Signed out of the tailnet: doctor says so.
    std::fs::write(
        e.fake.join("status.json"),
        r#"{"BackendState":"NeedsLogin","Self":{}}"#,
    )
    .unwrap();
    let (_, out) = e.run(&["doctor"]);
    assert!(
        out.contains("Tailscale is not reachable from here (logged_out)"),
        "{out}"
    );

    // A names file the hub's certificate does not have yet.
    std::fs::write(e.hub.join("remote-names"), "other.tail1234.ts.net\n").unwrap();
    let (_, out) = e.run(&["doctor"]);
    assert!(out.contains("does not name other.tail1234.ts.net"), "{out}");
    std::fs::remove_file(e.hub.join("remote-names")).unwrap();
}

#[test]
fn doctor_fails_a_way_in_without_a_password_and_warns_about_a_public_listen() {
    let e = Env::new("doctor");
    std::fs::write(e.hub.join("listen"), "203.0.113.7:7443").unwrap();
    let (code, out) = e.run(&["doctor"]);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains("no screen password is set. Run `agora password set`"),
        "{out}"
    );
    assert!(
        out.contains(
            "listens on a public address: only the screen password and the certificate pin"
        ),
        "{out}"
    );
    e.set_password();
    let (_, out) = e.run(&["doctor"]);
    assert!(out.contains("a screen password protects it"), "{out}");
    assert!(out.contains("listens on a public address"), "{out}");
    // A machine that never turned anything on gets no remote lines at all.
    let quiet = Env::new("doctorquiet");
    let (_, out) = quiet.run(&["doctor"]);
    assert!(!out.contains("screen password"), "{out}");
    assert!(!out.contains("Tailscale"), "{out}");
}
