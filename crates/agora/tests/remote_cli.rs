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
      --bg) [ -e "$D/denied" ] && { echo "Access denied: serve config denied" >&2; exit 1; }
            port=${3#--https=}; mkdir -p "$D/rules"; printf '%s' "$4" > "$D/rules/$port"; exit 0 ;;
      --https=*) port=${2#--https=}; [ "$3" = off ] && rm -f "$D/rules/$port"; exit 0 ;;
    esac ;;
  debug)
    [ "$2" = prefs ] && { printf '{"OperatorUser":"%s"}\n' "$(cat "$D/operator" 2>/dev/null)"; exit 0; } ;;
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
            .env("USER", "dev")
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

/// The maintainer's blackview (2026-11-27): Tailscale refused `serve` ("Access denied: serve config denied") because the user was not
/// its operator, and Agora said only that the answer could not be used. Now it names the fix.
#[cfg(unix)]
#[test]
fn a_serve_refused_for_want_of_operator_rights_says_how_to_fix_it() {
    let e = Env::new("notoperator");
    e.set_password();
    std::fs::write(e.fake.join("denied"), "").unwrap();
    let (code, out) = e.run(&["remote", "tailscale", "--serve"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("remote.tailscale_not_operator"), "{out}");
    assert!(out.contains("sudo tailscale set --operator=dev"), "{out}");
    assert!(!e.hub.join("remote-serve").exists());
    // Refused for another reason while the user is the operator: the failure as it was.
    std::fs::write(e.fake.join("operator"), "dev").unwrap();
    let (code, out) = e.run(&["remote", "tailscale", "--serve"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("remote.tailscale_unreadable"), "{out}");
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
    assert!(
        out.contains("Tailscale Funnel is on, so something"),
        "{out}"
    );
    assert!(!out.contains("listens on a public address"), "{out}");
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
    assert!(out.contains("start it on a local address"), "{out}");
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

/// A running `agora hub run`, its output in a file: when the test fails, the hub's exit status and output are printed, so a hub that
/// exited, never bound, or listens somewhere other than where the test looked is visible in the log.
#[cfg(unix)]
struct Proc {
    child: Child,
    log: PathBuf,
}

#[cfg(unix)]
impl Proc {
    /// `agora hub run --listen 127.0.0.1:0` in `e`'s hub folder. The `listen` file `Env::new` seeds (a hub that is not running) is
    /// removed first: the hub writes its certificate before it writes `listen`, so with the seed still there a wait for "certificate
    /// and `listen`" could end with the seed's address, where nothing listens (#978).
    fn hub(e: &Env) -> Self {
        let _ = std::fs::remove_file(e.hub.join("listen"));
        let log = e.hub.with_file_name("hub-run.log");
        let out = std::fs::File::create(&log).unwrap();
        let mut c = e.agora(&["hub", "run", "--listen", "127.0.0.1:0"]);
        c.stdout(out.try_clone().unwrap()).stderr(out);
        Self {
            child: c.spawn().unwrap(),
            log,
        }
    }
}

#[cfg(unix)]
impl Drop for Proc {
    fn drop(&mut self) {
        if std::thread::panicking() {
            let status = self.child.try_wait();
            let log = std::fs::read_to_string(&self.log).unwrap_or_default();
            eprintln!("hub status: {status:?} (Ok(None) = still running)\nhub output:\n{log}");
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
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

/// The address in `<hub dir>/listen`, once it parses as one (an empty or partial file is not yet an address).
#[cfg(unix)]
fn hub_listening(hub: &Path) -> Option<std::net::SocketAddr> {
    std::fs::read_to_string(hub.join("listen"))
        .ok()?
        .trim()
        .parse()
        .ok()
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
    let _hub = Proc::hub(&e);
    wait_until("the hub wrote its certificate", || {
        !cert_names(&e.hub).is_empty() && hub_listening(&e.hub).is_some()
    });
    assert!(!cert_names(&e.hub).contains(&"box.tail1234.ts.net".to_string()));
    let ca_before = std::fs::read(e.hub.join("ca.pem")).unwrap();

    let (code, out) = e.run(&["remote", "tailscale", "--serve"]);
    assert_eq!(code, 0, "{out}");
    // The hub listens on a random loopback port: the rule points at the port it really bound.
    let bound = hub_listening(&e.hub).unwrap();
    assert!(
        e.calls().contains(&format!("https+insecure://{bound}")),
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

// ---- ADR-0067 rung 4: a trusted reverse proxy (`agora remote proxy`). No `tailscale` and no real tunnel is involved. ----

const PROXY: [&str; 6] = [
    "remote",
    "proxy",
    "--trust-header",
    "CF-Connecting-IP",
    "--host",
    "agora.example.com",
];

#[test]
fn a_proxy_is_not_trusted_without_a_screen_password() {
    let e = Env::new("proxynopw");
    let (code, out) = e.run(&PROXY);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("remote.password_required"), "{out}");
    assert!(out.contains("agora password set"), "{out}");
    assert!(!e.hub.join("remote-proxy").exists(), "a file was written");
    // A hub folder with a database and still no password is the same.
    drop(SqliteStore::open(&e.hub.join("hub.db")).unwrap());
    let (code, out) = e.run(&PROXY);
    assert_eq!(
        (code, out.contains("remote.password_required")),
        (1, true),
        "{out}"
    );
    assert!(!e.hub.join("remote-proxy").exists());
}

#[test]
fn a_proxy_needs_a_header_and_a_name_that_are_plain() {
    let e = Env::new("proxybad");
    e.set_password();
    // Missing parts are a usage error.
    for args in [
        &["remote", "proxy"][..],
        &["remote", "proxy", "--host", "agora.example.com"],
        &["remote", "proxy", "--trust-header", "CF-Connecting-IP"],
    ] {
        assert_eq!(e.run(args).0, 2, "{args:?}");
    }
    let with = |header: &str, host: &str| {
        e.run(&["remote", "proxy", "--trust-header", header, "--host", host])
    };
    for header in ["Host", "origin", "x y", "a:b"] {
        let (code, out) = with(header, "agora.example.com");
        assert_eq!(code, 1, "{header}: {out}");
        assert!(
            out.contains("remote.proxy_header_invalid"),
            "{header}: {out}"
        );
    }
    for host in [
        "localhost",
        "127.0.0.1",
        "*.example.com",
        "agora",
        "agora.example.com:443",
        "a b.example.com",
    ] {
        let (code, out) = with("CF-Connecting-IP", host);
        assert_eq!(code, 1, "{host}: {out}");
        assert!(out.contains("remote.proxy_host_invalid"), "{host}: {out}");
    }
    let (code, out) = e.run(&[
        "remote",
        "proxy",
        "--trust-header",
        "CF-Connecting-IP",
        "--host",
        "agora.example.com",
        "--proxy-addr",
        "not-an-address",
    ]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("remote.proxy_addr_invalid"), "{out}");
    let (code, out) = e.run(&[
        "remote",
        "proxy",
        "--trust-header",
        "CF-Connecting-IP",
        "--host",
        "a.example.com",
        "--host",
        "b.example.com",
        "--host",
        "c.example.com",
        "--host",
        "d.example.com",
        "--host",
        "e.example.com",
    ]);
    assert_eq!(code, 1, "at most four names: {out}");
    assert!(
        !e.hub.join("remote-proxy").exists(),
        "nothing was written by a refusal"
    );
}

#[test]
fn turning_the_proxy_on_is_visible_everywhere_and_off_takes_it_back() {
    let e = Env::new("proxyon");
    e.set_password();
    // Nothing on: no proxy lines.
    let (_, out) = e.run(&["remote", "status"]);
    assert!(!out.contains("Trusted proxy"), "{out}");
    let (_, out) = e.run(&["doctor"]);
    assert!(!out.contains("trusted proxy"), "{out}");

    let (code, out) = e.run(&PROXY);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("agora.example.com"), "{out}");
    assert!(
        out.contains("single sign-on"),
        "the Access recommendation: {out}"
    );
    assert_eq!(
        std::fs::read_to_string(e.hub.join("remote-proxy")).unwrap(),
        "header=cf-connecting-ip\nhost=agora.example.com\n"
    );
    // Visible in status and in doctor, every time, as a warning that is not a failure while the password is set.
    for _ in 0..2 {
        let (_, out) = e.run(&["remote", "status"]);
        assert!(
            out.contains("Trusted proxy: on, answers agora.example.com"),
            "{out}"
        );
        assert!(out.contains("Reachable from the internet"), "{out}");
        let (code, out) = e.run(&["doctor"]);
        assert!(
            out.contains("reachable from the internet through a trusted proxy"),
            "{out}"
        );
        assert!(out.contains("a screen password protects it"), "{out}");
        assert!(!out.contains("fail"), "{out}");
        let _ = code;
    }
    // The password stays while the proxy is on, and the way out is named.
    let (code, out) = e.run(&["password", "clear"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("Error remote.password_in_use"), "{out}");
    assert!(
        out.contains("Turn that off first with `agora remote off`"),
        "the way out is named: {out}"
    );
    assert!(e.password_is_set());

    // The hub also listening beyond this machine is warned about.
    std::fs::write(e.hub.join("listen"), "203.0.113.7:7443").unwrap();
    let (_, out) = e.run(&["doctor"]);
    assert!(out.contains("beyond this machine"), "{out}");
    std::fs::write(e.hub.join("listen"), "127.0.0.1:7443").unwrap();
    let (_, out) = e.run(&["doctor"]);
    assert!(!out.contains("beyond this machine"), "{out}");

    // Without a password (removed by hand) doctor fails it.
    let nopw = Env::new("proxyon-nopw");
    std::fs::write(
        nopw.hub.join("remote-proxy"),
        "header=x-ip\nhost=a.example.com\n",
    )
    .unwrap();
    let (code, out) = nopw.run(&["doctor"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("no screen password is set"), "{out}");

    // Off removes it.
    let (code, out) = e.run(&["remote", "off"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("no longer trusts a reverse proxy"), "{out}");
    assert!(!e.hub.join("remote-proxy").exists());
    let (_, out) = e.run(&["remote", "status"]);
    assert!(!out.contains("Trusted proxy"), "{out}");
    let (_, out) = e.run(&["remote", "off"]);
    assert!(out.contains("has not turned on any way in"), "{out}");
}

#[test]
fn a_damaged_proxy_file_trusts_nothing_and_says_so() {
    let e = Env::new("proxydamaged");
    e.set_password();
    std::fs::write(
        e.hub.join("remote-proxy"),
        "header=x-ip\nhost=*.example.com\n",
    )
    .unwrap();
    let (_, out) = e.run(&["remote", "status"]);
    assert!(
        out.contains("file that turns on the trusted proxy is damaged"),
        "{out}"
    );
    let (_, out) = e.run(&["doctor"]);
    assert!(
        out.contains("file that turns on the trusted proxy is damaged"),
        "{out}"
    );
    let (code, out) = e.run(&["remote", "off"]);
    assert_eq!(code, 0, "{out}");
    assert!(!e.hub.join("remote-proxy").exists());
}

#[test]
fn the_proxy_text_is_in_both_languages() {
    let e = Env::new("proxylang");
    e.set_password();
    let mut c = e.agora(&PROXY);
    c.env("AGORA_LANG", "tr");
    let o = c.output().unwrap();
    let out = String::from_utf8_lossy(&o.stdout);
    assert_eq!(o.status.code(), Some(0), "{out}");
    assert!(out.contains("ters vekile güveniyor"), "{out}");
    assert!(!out.contains("reverse proxy"), "{out}");
}

/// A real hub process, a real TLS connection and fake proxy headers: the file `agora remote proxy` writes reaches the running hub, a
/// foreign Host is refused while it is on, a request that names the owner's own address is still not the owner, and `remote off`
/// is followed without a restart. This is the only test that proves the hub command wires the proxy in.
#[cfg(unix)]
#[test]
fn a_real_hub_follows_remote_proxy_and_remote_off() {
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, ServerName};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let e = Env::new("proxylive");
    e.set_password();
    let _hub = Proc::hub(&e);
    wait_until("the hub is listening", || {
        hub_listening(&e.hub).is_some() && !cert_names(&e.hub).is_empty()
    });
    let addr = hub_listening(&e.hub).unwrap();
    let ca = std::fs::read_to_string(e.hub.join("ca.pem")).unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let ask = |headers: &[&str]| -> String {
        let request = format!(
            "GET /api/session HTTP/1.1\r\n{}\r\n\r\n",
            headers.join("\r\n")
        );
        rt.block_on(async {
            let mut roots = rustls::RootCertStore::empty();
            for c in CertificateDer::pem_slice_iter(ca.as_bytes()) {
                roots.add(c.unwrap()).unwrap();
            }
            let cfg = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
            // The hub has written its address a moment before it answers TLS: ask again until it does (up to 30 s; a loaded CI machine
            // took longer than the earlier 4 s).
            let connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(cfg));
            let mut tries = 0;
            let mut tls = loop {
                tries += 1;
                let attempt = async {
                    let tcp = tokio::net::TcpStream::connect(addr).await?;
                    connector
                        .connect(ServerName::try_from("localhost").unwrap(), tcp)
                        .await
                };
                match attempt.await {
                    Ok(tls) => break tls,
                    Err(e) if tries >= 300 => panic!("hub did not answer TLS at {addr}: {e}"),
                    Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
                }
            };
            tls.write_all(request.as_bytes()).await.unwrap();
            let mut out = Vec::new();
            let _ = tokio::time::timeout(Duration::from_secs(5), tls.read_to_end(&mut out)).await;
            String::from_utf8_lossy(&out).to_string()
        })
    };
    let status = |out: &str| out.get(9..12).unwrap_or("").to_string();

    assert_eq!(status(&ask(&["Host: evil.example.org"])), "200");
    let (code, out) = e.run(&PROXY);
    assert_eq!(code, 0, "{out}");
    wait_until("the hub took the proxy file", || {
        status(&ask(&["Host: evil.example.org"])) == "421"
    });
    for value in ["203.0.113.9", "127.0.0.1"] {
        let out = ask(&[
            "Host: agora.example.com",
            &format!("CF-Connecting-IP: {value}"),
        ]);
        assert!(out.contains(r#""passwordRequired":true"#), "{value}: {out}");
    }
    let out = ask(&["Host: localhost"]);
    assert!(out.contains(r#""passwordRequired":false"#), "{out}");
    let (code, out) = e.run(&["remote", "off"]);
    assert_eq!(code, 0, "{out}");
    wait_until("the hub dropped the proxy", || {
        status(&ask(&["Host: evil.example.org"])) == "200"
    });
}

#[test]
fn an_address_nothing_connects_from_is_not_a_proxy() {
    let e = Env::new("proxyaddr");
    e.set_password();
    for addr in [
        "0.0.0.0",
        "::",
        "224.0.0.1",
        "ff02::1",
        "::ffff:0.0.0.0",
        "::ffff:224.0.0.1",
    ] {
        let mut args = PROXY.to_vec();
        args.extend(["--proxy-addr", addr]);
        let (code, out) = e.run(&args);
        assert_eq!(code, 1, "{addr}: {out}");
        assert!(out.contains("remote.proxy_addr_invalid"), "{addr}: {out}");
    }
    assert!(!e.hub.join("remote-proxy").exists());
    let mut args = PROXY.to_vec();
    args.extend(["--proxy-addr", "::ffff:10.1.2.3"]);
    let (code, out) = e.run(&args);
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        std::fs::read_to_string(e.hub.join("remote-proxy")).unwrap(),
        "header=cf-connecting-ip\nhost=agora.example.com\nproxy=10.1.2.3\n",
        "one spelling"
    );
}

/// A link, a FIFO and an over-long file in place of the proxy file are damaged, say so, and never hang the command.
#[cfg(unix)]
#[test]
fn a_link_a_fifo_or_a_long_file_is_a_damaged_proxy_file() {
    let e = Env::new("proxyshapes");
    e.set_password();
    let file = e.hub.join("remote-proxy");
    let good = "header=x-ip\nhost=a.example.com\n";
    let damaged = |e: &Env| {
        let (_, out) = e.run(&["remote", "status"]);
        out.contains("file that turns on the trusted proxy is damaged")
    };
    std::fs::write(e.hub.join("real"), good).unwrap();
    std::os::unix::fs::symlink(e.hub.join("real"), &file).unwrap();
    assert!(damaged(&e), "a link");
    std::fs::remove_file(&file).unwrap();
    let made = Command::new("mkfifo")
        .arg(&file)
        .status()
        .map(|s| s.success());
    if made.unwrap_or(false) {
        assert!(damaged(&e), "a fifo");
        std::fs::remove_file(&file).unwrap();
    }
    std::fs::write(&file, format!("{good}{}", "\n".repeat(40))).unwrap();
    assert!(damaged(&e), "more than 32 lines");
    std::fs::write(&file, good).unwrap();
    assert!(!damaged(&e), "the good one is fine");
}
