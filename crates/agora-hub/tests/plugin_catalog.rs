//! ADR-0114 decision 7 at the hub: the community plugin catalog against a local HTTP server. An index signed with a test key (made
//! here, never kept) is kept; a bad signature, another key, a lower serial (under the same key, even pinned again or saved late), the
//! same serial with other bytes, a redirect, an answer too large or too slow change nothing; what the owner was asked about is checked
//! again when the answer comes; an archive whose bytes are not the signed entry's, or whose manifest says something else, is never
//! installed; an upgrade says what it adds and removes, and a version not newer is refused; every step is a row of the audit trail as
//! the owner's. When `ssh-keygen` is on the machine, an index signed by the tool itself (the procedure in `docs/ux/plugins.md`)
//! verifies too.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agora_hub::plugin_catalog::{self, InstallError, Limits, Mode, UpdateError};
use agora_hub::plugins::Plugins;
use agora_hub::sshsig::{self, PublicKey, SigError};
use agora_hub::store::SqliteStore;
use agora_hub::theme_catalog::IndexError;
use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair};
use sha2::{Digest, Sha256};

const VERSION: &str = "0.3.0";

/// What the test server answers for a path.
#[derive(Clone)]
enum Answer {
    Body(u16, String, Vec<u8>),
    /// A byte every 50 ms, for ever (a server that never finishes).
    Drip,
}

fn ok(body: impl Into<Vec<u8>>) -> Answer {
    Answer::Body(200, String::new(), body.into())
}

/// A plain HTTP server on this machine answering each path from `answers` (which the test may change); returns its address.
fn serve(answers: Arc<Mutex<HashMap<String, Answer>>>) -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { continue };
            let answers = Arc::clone(&answers);
            std::thread::spawn(move || {
                let mut got = [0u8; 2048];
                let n = s.read(&mut got).unwrap_or(0);
                let req = String::from_utf8_lossy(&got[..n]).to_string();
                let path = req.split(' ').nth(1).unwrap_or("").to_string();
                let answer = answers.lock().unwrap().get(&path).cloned();
                match answer {
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
                        let _ = s.write_all(b"HTTP/1.0 404 Not Found\r\nContent-Length: 0\r\n\r\n");
                    }
                }
            });
        }
    });
    base
}

struct Key(Ed25519KeyPair);

impl Key {
    fn new() -> Key {
        let doc = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        Key(Ed25519KeyPair::from_pkcs8(doc.as_ref()).unwrap())
    }

    fn public(&self) -> PublicKey {
        let mut blob = Vec::new();
        for part in [b"ssh-ed25519".as_slice(), self.0.public_key().as_ref()] {
            blob.extend_from_slice(&(part.len() as u32).to_be_bytes());
            blob.extend_from_slice(part);
        }
        PublicKey::parse(&format!(
            "ssh-ed25519 {} test@catalog",
            sshsig::base64_encode(&blob)
        ))
        .unwrap()
    }

    /// What `ssh-keygen -Y sign -n <ns>` writes for `msg`.
    fn sign(&self, ns: &str, msg: &[u8]) -> String {
        let sig = self.0.sign(&sshsig::signed_data(
            ns,
            b"",
            b"sha512",
            &sshsig::sha512(msg),
        ));
        sshsig::armour(&self.public().blob(), ns, sig.as_ref())
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// A package of one entry script, asking for `caps`.
fn package(id: &str, version: &str, caps: &[&str]) -> Vec<u8> {
    let entry = format!("agora.watch('agents.read', function () {{}}); // {version}");
    let caps: Vec<String> = caps.iter().map(|c| format!("\"{c}\"")).collect();
    let manifest = format!(
        r#"{{"id":"{id}","name":"Demo {id}","version":"{version}","author":"Someone","licence":"MIT","min_agora":"0.1.0","panels":[{{"id":"main"}}],"capabilities":[{}],"entry":"panel.js","files":{{"panel.js":"{}"}}}}"#,
        caps.join(","),
        hex(&Sha256::digest(entry.as_bytes()))
    );
    format!(
        r#"{{"agora-plugin-bundle":1,"files":{{"agora-plugin.json":"{}","panel.js":"{}"}}}}"#,
        sshsig::base64_encode(manifest.as_bytes()),
        sshsig::base64_encode(entry.as_bytes())
    )
    .into_bytes()
}

fn entry(id: &str, version: &str, caps: &[&str], archive: &[u8]) -> serde_json::Value {
    serde_json::json!({"id": id, "name": format!("Demo {id}"), "version": version, "author": "Someone", "licence": "MIT",
        "description": "Lists the agents.", "capabilities": caps, "sha256": hex(&Sha256::digest(archive)),
        "size": archive.len(), "file": format!("plugins/{id}/{id}-{version}.json")})
}

fn index(serial: u64, entries: Vec<serde_json::Value>) -> Vec<u8> {
    serde_json::json!({"agora-plugin-catalog": 1, "serial": serial, "plugins": entries})
        .to_string()
        .into_bytes()
}

/// A catalog on a local server: the index at `/cat/index.json`, signed with `key`, and each archive at its path.
struct Catalog {
    answers: Arc<Mutex<HashMap<String, Answer>>>,
    url: String,
}

impl Catalog {
    fn new() -> Catalog {
        let answers = Arc::new(Mutex::new(HashMap::new()));
        let base = serve(Arc::clone(&answers));
        Catalog {
            answers,
            url: format!("{base}/cat/index.json"),
        }
    }

    fn set(&self, path: &str, a: Answer) {
        self.answers.lock().unwrap().insert(path.to_string(), a);
    }

    fn publish(&self, key: &Key, index: &[u8]) {
        self.set("/cat/index.json", ok(index.to_vec()));
        self.set(
            "/cat/index.json.sig",
            ok(key.sign(plugin_catalog::SIG_NAMESPACE, index)),
        );
    }

    fn archive(&self, id: &str, version: &str, bytes: &[u8]) {
        self.set(
            &format!("/cat/plugins/{id}/{id}-{version}.json"),
            ok(bytes.to_vec()),
        );
    }
}

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

fn store_with(key: &Key) -> SqliteStore {
    let s = SqliteStore::open_in_memory().unwrap();
    plugin_catalog::set_key(&s, &key.public().line(), 1).unwrap();
    plugin_catalog::set_on(&s, true, 2).unwrap();
    s
}

fn trail(s: &SqliteStore) -> Vec<(String, String, String, String)> {
    agora_hub::audit::Audit::new(s)
        .list(0, None, 200)
        .unwrap()
        .into_iter()
        .map(|e| (e.who, e.action, e.node, e.outcome))
        .collect()
}

fn has(s: &SqliteStore, action: &str, target: &str, outcome: &str) -> bool {
    trail(s)
        .iter()
        .any(|(w, a, n, o)| w == "owner" && a == action && n == target && o == outcome)
}

#[test]
fn a_signed_index_is_kept_and_listed_only_while_the_catalog_is_on() {
    let key = Key::new();
    let cat = Catalog::new();
    let a = package("demo", "1.0.0", &["agents.read"]);
    let mut bad = entry("bad", "1.0.0", &["agents.read"], &a);
    bad["capabilities"] = serde_json::json!(["terminal.write"]);
    cat.publish(
        &key,
        &index(4, vec![entry("demo", "1.0.0", &["agents.read"], &a), bad]),
    );
    let s = store_with(&key);
    let f = plugin_catalog::update(&s, &cat.url, roomy(), 10).unwrap();
    assert_eq!(f.serial, 4);
    assert_eq!(f.entries.len(), 1);
    assert_eq!(f.refused.len(), 1, "the bad entry is refused alone");
    assert!(has(&s, "plugin_catalog.update", "", "partial"));
    let l = plugin_catalog::list(&s).unwrap();
    assert!(l.on);
    assert_eq!(l.updated_at_ms, 10);
    assert_eq!(l.entries.len(), 1);
    assert_eq!(l.entries[0].plugin_id, "demo");
    assert_eq!(l.entries[0].capabilities, ["agents.read"]);
    assert_eq!(l.entries[0].description, "Lists the agents.");
    plugin_catalog::set_on(&s, false, 11).unwrap();
    assert_eq!(
        plugin_catalog::list(&s).unwrap(),
        agora_protocol::v1::PluginCatalogListResult::default()
    );
    assert!(has(&s, "plugin_catalog.off", "", "ok"));
}

#[test]
fn a_bad_signature_another_key_or_a_lower_serial_changes_nothing() {
    let key = Key::new();
    let cat = Catalog::new();
    let a = package("demo", "1.0.0", &["agents.read"]);
    let good = index(5, vec![entry("demo", "1.0.0", &["agents.read"], &a)]);
    cat.publish(&key, &good);
    let s = store_with(&key);
    plugin_catalog::update(&s, &cat.url, roomy(), 10).unwrap();

    // The index changed after it was signed.
    let tampered = index(6, vec![entry("demo", "1.0.1", &["agents.read"], &a)]);
    cat.set("/cat/index.json", ok(tampered.clone()));
    assert_eq!(
        plugin_catalog::update(&s, &cat.url, roomy(), 20).unwrap_err(),
        UpdateError::Signature(SigError::Invalid)
    );
    // Signed by another key.
    cat.publish(&Key::new(), &tampered);
    assert_eq!(
        plugin_catalog::update(&s, &cat.url, roomy(), 21).unwrap_err(),
        UpdateError::Signature(SigError::WrongKey)
    );
    // Signed for another use.
    cat.set("/cat/index.json.sig", ok(key.sign("file", &tampered)));
    assert_eq!(
        plugin_catalog::update(&s, &cat.url, roomy(), 22).unwrap_err(),
        UpdateError::Signature(SigError::Namespace)
    );
    // No signature at all.
    cat.answers.lock().unwrap().remove("/cat/index.json.sig");
    assert_eq!(
        plugin_catalog::update(&s, &cat.url, roomy(), 23).unwrap_err(),
        UpdateError::SignatureFetch(IndexError::Status(404))
    );
    // An older index, validly signed: refused, so a withdrawn version cannot be served again.
    cat.publish(&key, &index(4, vec![entry("demo", "0.9.0", &[], &a)]));
    assert_eq!(
        plugin_catalog::update(&s, &cat.url, roomy(), 24).unwrap_err(),
        UpdateError::Rollback {
            seen: 5,
            offered: 4
        }
    );
    // Nothing of the above was kept.
    let l = plugin_catalog::list(&s).unwrap();
    assert_eq!(l.entries[0].version, "1.0.0");
    assert_eq!(l.updated_at_ms, 10);
    assert_eq!(plugin_catalog::status(&s).unwrap().serial, 5);
    for outcome in [
        "failed.signature.invalid",
        "failed.signature.wrong-key",
        "failed.signature.namespace",
        "failed.signature-fetch.status",
        "failed.rollback",
    ] {
        assert!(has(&s, "plugin_catalog.update", "", outcome), "{outcome}");
    }
    // The same serial again is fine (fetching twice).
    cat.publish(&key, &good);
    plugin_catalog::update(&s, &cat.url, roomy(), 30).unwrap();
}

#[test]
fn without_a_key_the_catalog_cannot_be_used_and_a_new_key_forgets_what_the_old_one_verified() {
    let key = Key::new();
    let cat = Catalog::new();
    let a = package("demo", "1.0.0", &["agents.read"]);
    let first = index(9, vec![entry("demo", "1.0.0", &["agents.read"], &a)]);
    cat.publish(&key, &first);
    cat.archive("demo", "1.0.0", &a);
    let s = SqliteStore::open_in_memory().unwrap();
    plugin_catalog::set_on(&s, true, 1).unwrap();
    assert_eq!(
        plugin_catalog::update(&s, &cat.url, roomy(), 2).unwrap_err(),
        UpdateError::NoKey
    );
    assert_eq!(
        plugin_catalog::prepare(&s, "demo", Mode::Install, VERSION, roomy(), 3).unwrap_err(),
        InstallError::NoKey
    );
    assert!(has(&s, "plugin_catalog.update", "", "failed.no-key"));
    assert!(has(&s, "plugin_catalog.install", "demo", "no-key"));

    // A private key is refused and nothing of it is kept.
    let e = plugin_catalog::set_key(
        &s,
        "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----\n",
        4,
    )
    .unwrap_err();
    assert_eq!(
        e,
        plugin_catalog::SetKeyError::Key(sshsig::KeyError::Private)
    );
    assert_eq!(plugin_catalog::key(&s).unwrap(), None);
    assert!(has(&s, "plugin_catalog.key", "", "key.private"));

    plugin_catalog::set_key(&s, &key.public().line(), 5).unwrap();
    assert!(has(
        &s,
        "plugin_catalog.key",
        &key.public().fingerprint(),
        "ok"
    ));
    plugin_catalog::update(&s, &cat.url, roomy(), 6).unwrap();
    assert_eq!(plugin_catalog::list(&s).unwrap().entries.len(), 1);
    // The same key again keeps the catalog.
    plugin_catalog::set_key(&s, &key.public().line(), 7).unwrap();
    assert_eq!(plugin_catalog::list(&s).unwrap().entries.len(), 1);
    // Another key: the catalog verified under the old one is forgotten, its serial too, and nothing is installed from it.
    let next = Key::new();
    plugin_catalog::set_key(&s, &next.public().line(), 8).unwrap();
    let l = plugin_catalog::list(&s).unwrap();
    assert!(l.on && l.entries.is_empty() && l.updated_at_ms == 0);
    assert_eq!(plugin_catalog::status(&s).unwrap().serial, 0);
    assert_eq!(
        plugin_catalog::prepare(&s, "demo", Mode::Install, VERSION, roomy(), 9).unwrap_err(),
        InstallError::NotVerified
    );
    // Under the new key, never pinned before, an index with a lower serial is accepted: the old key's floor is not the new key's.
    cat.publish(
        &next,
        &index(1, vec![entry("demo", "1.0.0", &["agents.read"], &a)]),
    );
    plugin_catalog::update(&s, &cat.url, roomy(), 10).unwrap();
    assert_eq!(plugin_catalog::list(&s).unwrap().entries.len(), 1);

    // The first key pinned again: what it verified is forgotten, but its floor is where it was, so its older index stays refused.
    plugin_catalog::set_key(&s, &key.public().line(), 11).unwrap();
    assert!(plugin_catalog::list(&s).unwrap().entries.is_empty());
    cat.publish(
        &key,
        &index(8, vec![entry("demo", "0.9.0", &["agents.read"], &a)]),
    );
    assert_eq!(
        plugin_catalog::update(&s, &cat.url, roomy(), 12).unwrap_err(),
        UpdateError::Rollback {
            seen: 9,
            offered: 8
        }
    );
    // Its serial again with other bytes is refused too; with the same bytes it is fine, and a higher one is taken.
    cat.publish(
        &key,
        &index(9, vec![entry("demo", "0.9.0", &["agents.read"], &a)]),
    );
    assert_eq!(
        plugin_catalog::update(&s, &cat.url, roomy(), 13).unwrap_err(),
        UpdateError::SerialReused { serial: 9 }
    );
    assert!(has(&s, "plugin_catalog.update", "", "failed.serial-reused"));
    assert!(plugin_catalog::list(&s).unwrap().entries.is_empty());
    cat.publish(&key, &first);
    plugin_catalog::update(&s, &cat.url, roomy(), 14).unwrap();
    assert_eq!(
        plugin_catalog::list(&s).unwrap().entries[0].version,
        "1.0.0"
    );
    cat.publish(
        &key,
        &index(10, vec![entry("demo", "1.0.0", &["agents.read"], &a)]),
    );
    plugin_catalog::update(&s, &cat.url, roomy(), 15).unwrap();
    assert_eq!(plugin_catalog::status(&s).unwrap().serial, 10);
}

/// Two updates at once: the one that read the floor before the other kept a newer index cannot lower it when it saves.
#[test]
fn an_update_that_saves_late_never_lowers_the_floor() {
    let key = Key::new();
    let cat = Catalog::new();
    let a = package("demo", "1.0.0", &["agents.read"]);
    let b = package("demo", "1.1.0", &["agents.read"]);
    cat.publish(
        &key,
        &index(5, vec![entry("demo", "1.0.0", &["agents.read"], &a)]),
    );
    let s = store_with(&key);
    let at = plugin_catalog::index_url(&cat.url).unwrap();
    // The first update read no floor and fetched serial 5; before it saves, a second one keeps serial 6.
    let late = plugin_catalog::fetch(&at, &key.public(), None, roomy()).unwrap();
    assert_eq!(late.serial, 5);
    cat.publish(
        &key,
        &index(6, vec![entry("demo", "1.1.0", &["agents.read"], &b)]),
    );
    plugin_catalog::update(&s, &cat.url, roomy(), 10).unwrap();
    assert_eq!(
        plugin_catalog::save(&s, &late, &cat.url, &key.public(), 11).unwrap_err(),
        UpdateError::Rollback {
            seen: 6,
            offered: 5
        }
    );
    // The same serial with other bytes is refused there too.
    let mut other = late.clone();
    other.serial = 6;
    assert_eq!(
        plugin_catalog::save(&s, &other, &cat.url, &key.public(), 12).unwrap_err(),
        UpdateError::SerialReused { serial: 6 }
    );
    let st = plugin_catalog::status(&s).unwrap();
    assert_eq!((st.serial, st.updated_ms), (6, 10), "nothing was changed");
    assert_eq!(
        plugin_catalog::list(&s).unwrap().entries[0].version,
        "1.1.0"
    );
}

#[test]
fn an_archive_is_installed_only_when_it_is_exactly_what_the_signed_index_says() {
    let key = Key::new();
    let cat = Catalog::new();
    let demo = package("demo", "1.0.0", &["agents.read"]);
    let other = package("other", "1.0.0", &["agents.read"]);
    let liar = package("liar", "1.0.0", &["agents.read", "chat.read"]);
    let swapped = package("demo", "1.0.0", &["agents.read"]);
    cat.publish(
        &key,
        &index(
            1,
            vec![
                entry("demo", "1.0.0", &["agents.read"], &demo),
                entry("other", "1.0.0", &["agents.read"], &other),
                entry("liar", "1.0.0", &["agents.read"], &liar),
                entry("moved", "1.0.0", &["agents.read"], &demo),
                entry("slow", "1.0.0", &["agents.read"], &demo),
                entry("huge", "1.0.0", &["agents.read"], &demo),
                entry("renamed", "1.0.0", &["agents.read"], &swapped),
                entry("padded", "1.0.0", &["agents.read"], &demo),
            ],
        ),
    );
    // The archive of `other` was changed after the index was signed.
    let mut tampered = other.clone();
    let at = tampered.len() - 3;
    tampered[at] ^= 1;
    cat.archive("demo", "1.0.0", &demo);
    cat.archive("other", "1.0.0", &tampered);
    // `liar`'s archive has the size and hash the index gives, but asks for more than the signed entry says.
    cat.set("/cat/plugins/liar/liar-1.0.0.json", ok(liar.clone()));
    cat.set(
        "/cat/plugins/moved/moved-1.0.0.json",
        Answer::Body(
            302,
            "Location: /cat/plugins/demo/demo-1.0.0.json\r\n".into(),
            Vec::new(),
        ),
    );
    cat.set("/cat/plugins/slow/slow-1.0.0.json", Answer::Drip);
    cat.set(
        "/cat/plugins/huge/huge-1.0.0.json",
        ok(vec![
            b' ';
            agora_hub::plugin_package::MAX_PACKAGE_BYTES + 20 * 1024
        ]),
    );
    // `renamed` is listed under one id but its archive installs as `demo`: refused, or it would replace another plugin.
    cat.archive("renamed", "1.0.0", &swapped);
    // `padded` has a byte more than the signed size.
    let mut padded = demo.clone();
    padded.push(b' ');
    cat.archive("padded", "1.0.0", &padded);
    let s = store_with(&key);
    plugin_catalog::update(&s, &cat.url, roomy(), 10).unwrap();
    let try_ = |id: &str, limits: Limits| {
        plugin_catalog::prepare(&s, id, Mode::Install, VERSION, limits, 20).unwrap_err()
    };
    assert_eq!(try_("other", roomy()), InstallError::Hash);
    assert_eq!(
        try_("liar", roomy()),
        InstallError::Mismatch("capabilities")
    );
    assert_eq!(try_("padded", roomy()), InstallError::Size);
    assert_eq!(
        try_("moved", roomy()),
        InstallError::Fetch(IndexError::Status(302))
    );
    assert_eq!(
        try_("slow", quick()),
        InstallError::Fetch(IndexError::TimedOut)
    );
    assert_eq!(
        try_("huge", roomy()),
        InstallError::Fetch(IndexError::TooLarge)
    );
    assert_eq!(try_("renamed", roomy()), InstallError::Mismatch("id"));
    assert_eq!(try_("nothing", roomy()), InstallError::NotInCatalog);
    assert!(
        Plugins::new(&s).list().unwrap().is_empty(),
        "nothing was installed"
    );
    for (target, outcome) in [
        ("other@1.0.0", "hash"),
        ("liar@1.0.0", "mismatch.capabilities"),
        ("padded@1.0.0", "size"),
        ("moved@1.0.0", "fetch.status"),
        ("slow@1.0.0", "fetch.timeout"),
        ("huge@1.0.0", "fetch.too-large"),
        ("renamed@1.0.0", "mismatch.id"),
        ("nothing", "not-in-catalog"),
    ] {
        assert!(
            has(&s, "plugin_catalog.install", target, outcome),
            "{target} {outcome}"
        );
    }

    // The good one installs, once the owner agrees, as the catalog's.
    let p = plugin_catalog::prepare(&s, "demo", Mode::Install, VERSION, roomy(), 30).unwrap();
    assert_eq!(p.diff.added, ["agents.read"]);
    assert!(
        Plugins::new(&s).list().unwrap().is_empty(),
        "nothing before the owner agrees"
    );
    plugin_catalog::declined(&s, &p, 31);
    assert!(has(&s, "plugin_catalog.install", "demo@1.0.0", "declined"));
    let added = plugin_catalog::commit(&s, &p, VERSION, 32).unwrap();
    assert_eq!(
        (added.id.as_str(), added.version.as_str()),
        ("demo", "1.0.0")
    );
    assert!(has(&s, "plugin_catalog.install", "demo@1.0.0", "ok"));
    assert_eq!(
        plugin_catalog::prepare(&s, "demo", Mode::Install, VERSION, roomy(), 33).unwrap_err(),
        InstallError::Installed("1.0.0".into())
    );
}

#[test]
fn an_upgrade_says_what_it_adds_and_removes_and_never_goes_back() {
    let key = Key::new();
    let cat = Catalog::new();
    let v1 = package("demo", "1.0.0", &["agents.read", "theme.read"]);
    let v2 = package("demo", "1.1.0", &["agents.read", "chat.read"]);
    cat.publish(
        &key,
        &index(
            1,
            vec![entry("demo", "1.0.0", &["agents.read", "theme.read"], &v1)],
        ),
    );
    cat.archive("demo", "1.0.0", &v1);
    cat.archive("demo", "1.1.0", &v2);
    let s = store_with(&key);
    plugin_catalog::update(&s, &cat.url, roomy(), 10).unwrap();
    assert_eq!(
        plugin_catalog::prepare(&s, "demo", Mode::Upgrade, VERSION, roomy(), 11).unwrap_err(),
        InstallError::NotInstalled
    );
    let p = plugin_catalog::prepare(&s, "demo", Mode::Install, VERSION, roomy(), 12).unwrap();
    plugin_catalog::commit(&s, &p, VERSION, 13).unwrap();
    assert_eq!(
        plugin_catalog::prepare(&s, "demo", Mode::Upgrade, VERSION, roomy(), 14).unwrap_err(),
        InstallError::NotNewer {
            installed: "1.0.0".into(),
            offered: "1.0.0".into()
        }
    );
    // The update is fetched; nothing changes until the owner upgrades.
    cat.publish(
        &key,
        &index(
            2,
            vec![entry("demo", "1.1.0", &["agents.read", "chat.read"], &v2)],
        ),
    );
    plugin_catalog::update(&s, &cat.url, roomy(), 20).unwrap();
    assert_eq!(
        Plugins::new(&s).list().unwrap()[0].version,
        "1.0.0",
        "never automatic"
    );
    let up = plugin_catalog::prepare(&s, "demo", Mode::Upgrade, VERSION, roomy(), 21).unwrap();
    assert_eq!(up.installed.as_deref(), Some("1.0.0"));
    assert_eq!(up.diff.added, ["chat.read"]);
    assert_eq!(up.diff.removed, ["theme.read"]);
    assert_eq!(up.diff.kept, ["agents.read"]);
    plugin_catalog::commit(&s, &up, VERSION, 22).unwrap();
    assert!(has(&s, "plugin_catalog.upgrade", "demo@1.1.0", "ok"));
    let installed = &Plugins::new(&s).list().unwrap()[0];
    assert_eq!(installed.version, "1.1.0");
    assert_eq!(installed.capabilities, ["agents.read", "chat.read"]);
    // A catalog that lists an older version (a new serial) offers no downgrade.
    cat.publish(
        &key,
        &index(
            3,
            vec![entry("demo", "1.0.0", &["agents.read", "theme.read"], &v1)],
        ),
    );
    plugin_catalog::update(&s, &cat.url, roomy(), 30).unwrap();
    assert_eq!(
        plugin_catalog::prepare(&s, "demo", Mode::Upgrade, VERSION, roomy(), 31).unwrap_err(),
        InstallError::NotNewer {
            installed: "1.1.0".into(),
            offered: "1.0.0".into()
        }
    );
    assert!(has(&s, "plugin_catalog.upgrade", "demo@1.0.0", "not-newer"));
}

/// The owner may answer long after the archive was fetched and shown: what changed meanwhile (the switch, the key, the catalog, the
/// installed version) is checked again when the answer comes, and then nothing is kept.
#[test]
fn what_the_owner_was_asked_about_is_checked_again_when_the_answer_comes() {
    let key = Key::new();
    let cat = Catalog::new();
    let v1 = package("demo", "1.0.0", &["agents.read"]);
    let v11 = package("demo", "1.1.0", &["agents.read"]);
    let v12 = package("demo", "1.2.0", &["agents.read"]);
    for (v, p) in [("1.0.0", &v1), ("1.1.0", &v11), ("1.2.0", &v12)] {
        cat.archive("demo", v, p);
    }
    let first = index(1, vec![entry("demo", "1.0.0", &["agents.read"], &v1)]);
    cat.publish(&key, &first);
    let s = store_with(&key);
    plugin_catalog::update(&s, &cat.url, roomy(), 10).unwrap();
    let installed = || -> Vec<String> {
        Plugins::new(&s)
            .list()
            .unwrap()
            .into_iter()
            .map(|p| p.version)
            .collect()
    };
    let prepare =
        |mode: Mode| plugin_catalog::prepare(&s, "demo", mode, VERSION, roomy(), 20).unwrap();

    // Turned off while the owner was asked.
    let p = prepare(Mode::Install);
    plugin_catalog::set_on(&s, false, 21).unwrap();
    assert_eq!(
        plugin_catalog::commit(&s, &p, VERSION, 22).unwrap_err(),
        InstallError::Off
    );
    assert!(has(&s, "plugin_catalog.install", "demo@1.0.0", "off"));
    plugin_catalog::set_on(&s, true, 23).unwrap();

    // Another key pinned while the owner was asked: the archive was verified under the old one.
    let p = prepare(Mode::Install);
    plugin_catalog::set_key(&s, &Key::new().public().line(), 24).unwrap();
    assert_eq!(
        plugin_catalog::commit(&s, &p, VERSION, 25).unwrap_err(),
        InstallError::NotVerified
    );
    assert!(has(
        &s,
        "plugin_catalog.install",
        "demo@1.0.0",
        "not-verified"
    ));
    assert!(installed().is_empty());
    plugin_catalog::set_key(&s, &key.public().line(), 26).unwrap();
    plugin_catalog::update(&s, &cat.url, roomy(), 27).unwrap();

    // Installed by another command while the owner was asked.
    let p = prepare(Mode::Install);
    let q = prepare(Mode::Install);
    plugin_catalog::commit(&s, &p, VERSION, 28).unwrap();
    assert_eq!(
        plugin_catalog::commit(&s, &q, VERSION, 29).unwrap_err(),
        InstallError::Changed("installed")
    );
    assert!(has(
        &s,
        "plugin_catalog.install",
        "demo@1.0.0",
        "changed.installed"
    ));

    // The catalog moved on while the owner was asked about 1.1.0: the archive shown is no longer the one it lists.
    cat.publish(
        &key,
        &index(2, vec![entry("demo", "1.1.0", &["agents.read"], &v11)]),
    );
    plugin_catalog::update(&s, &cat.url, roomy(), 30).unwrap();
    let up = prepare(Mode::Upgrade);
    cat.publish(
        &key,
        &index(3, vec![entry("demo", "1.2.0", &["agents.read"], &v12)]),
    );
    plugin_catalog::update(&s, &cat.url, roomy(), 31).unwrap();
    assert_eq!(
        plugin_catalog::commit(&s, &up, VERSION, 32).unwrap_err(),
        InstallError::Changed("catalog")
    );
    assert!(has(
        &s,
        "plugin_catalog.upgrade",
        "demo@1.1.0",
        "changed.catalog"
    ));
    assert_eq!(installed(), ["1.0.0"]);

    // A newer version installed while the owner was asked about 1.1.0: answering yes would go back, so nothing is kept.
    cat.publish(
        &key,
        &index(4, vec![entry("demo", "1.1.0", &["agents.read"], &v11)]),
    );
    plugin_catalog::update(&s, &cat.url, roomy(), 33).unwrap();
    let up = prepare(Mode::Upgrade);
    assert_eq!(up.installed.as_deref(), Some("1.0.0"));
    Plugins::new(&s).add(&v12, VERSION, 34).unwrap();
    assert_eq!(
        plugin_catalog::commit(&s, &up, VERSION, 35).unwrap_err(),
        InstallError::Changed("installed")
    );
    assert!(has(
        &s,
        "plugin_catalog.upgrade",
        "demo@1.1.0",
        "changed.installed"
    ));
    assert_eq!(installed(), ["1.2.0"], "never back");

    // Removed from the catalog while the owner was asked.
    Plugins::new(&s).remove("demo", 37).unwrap();
    let p = prepare(Mode::Install);
    cat.publish(&key, &index(5, vec![]));
    plugin_catalog::update(&s, &cat.url, roomy(), 38).unwrap();
    assert_eq!(
        plugin_catalog::commit(&s, &p, VERSION, 39).unwrap_err(),
        InstallError::NotInCatalog
    );
    assert!(installed().is_empty());
}

#[test]
fn an_index_that_redirects_is_too_large_or_too_slow_is_refused_and_nothing_is_installed_while_off()
{
    let key = Key::new();
    let cat = Catalog::new();
    let s = store_with(&key);
    cat.set(
        "/cat/index.json",
        Answer::Body(301, "Location: /elsewhere.json\r\n".into(), Vec::new()),
    );
    assert_eq!(
        plugin_catalog::update(&s, &cat.url, roomy(), 1).unwrap_err(),
        UpdateError::Index(IndexError::Status(301))
    );
    cat.set(
        "/cat/index.json",
        ok(vec![b' '; plugin_catalog::MAX_INDEX_BYTES + 20 * 1024]),
    );
    assert_eq!(
        plugin_catalog::update(&s, &cat.url, roomy(), 2).unwrap_err(),
        UpdateError::Index(IndexError::TooLarge)
    );
    cat.set("/cat/index.json", Answer::Drip);
    assert_eq!(
        plugin_catalog::update(&s, &cat.url, quick(), 3).unwrap_err(),
        UpdateError::Index(IndexError::TimedOut)
    );
    let a = package("demo", "1.0.0", &[]);
    cat.publish(&key, &index(1, vec![entry("demo", "1.0.0", &[], &a)]));
    cat.set("/cat/index.json.sig", Answer::Drip);
    assert_eq!(
        plugin_catalog::update(&s, &cat.url, quick(), 4).unwrap_err(),
        UpdateError::SignatureFetch(IndexError::TimedOut)
    );
    assert_eq!(
        plugin_catalog::update(&s, plugin_catalog::DEFAULT_INDEX_URL, roomy(), 5).unwrap_err(),
        UpdateError::Index(IndexError::Address)
    );
    cat.publish(&key, &index(1, vec![entry("demo", "1.0.0", &[], &a)]));
    cat.archive("demo", "1.0.0", &a);
    plugin_catalog::update(&s, &cat.url, roomy(), 6).unwrap();
    plugin_catalog::set_on(&s, false, 7).unwrap();
    assert_eq!(
        plugin_catalog::prepare(&s, "demo", Mode::Install, VERSION, roomy(), 8).unwrap_err(),
        InstallError::Off
    );
    assert!(has(&s, "plugin_catalog.install", "demo", "off"));
}

/// The maintainer's procedure, with the real tool: `ssh-keygen -t ed25519` makes the key, `ssh-keygen -Y sign -n agora-plugin-catalog`
/// signs the index, and the hub verifies it with the `.pub` pinned. Skipped where the tool is missing.
#[test]
fn an_index_signed_by_ssh_keygen_verifies() {
    use std::process::Command;
    if Command::new("ssh-keygen").arg("-?").output().is_err() {
        eprintln!("ssh-keygen is not installed: skipped");
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "agora-plugin-catalog-keygen-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let key_file = dir.join("catalog");
    assert!(Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", "catalog", "-f"])
        .arg(&key_file)
        .status()
        .unwrap()
        .success());
    let a = package("demo", "1.0.0", &["agents.read"]);
    let idx = index(1, vec![entry("demo", "1.0.0", &["agents.read"], &a)]);
    let idx_file = dir.join("index.json");
    std::fs::write(&idx_file, &idx).unwrap();
    assert!(Command::new("ssh-keygen")
        .args(["-Y", "sign", "-n", plugin_catalog::SIG_NAMESPACE, "-f"])
        .arg(&key_file)
        .arg(&idx_file)
        .output()
        .unwrap()
        .status
        .success());
    let sig = std::fs::read_to_string(dir.join("index.json.sig")).unwrap();
    let public = std::fs::read_to_string(dir.join("catalog.pub")).unwrap();
    let _ = std::fs::remove_dir_all(&dir);

    let cat = Catalog::new();
    cat.set("/cat/index.json", ok(idx.clone()));
    cat.set("/cat/index.json.sig", ok(sig.clone()));
    let s = SqliteStore::open_in_memory().unwrap();
    let k = plugin_catalog::set_key(&s, &public, 1).unwrap();
    plugin_catalog::set_on(&s, true, 2).unwrap();
    plugin_catalog::update(&s, &cat.url, roomy(), 3).unwrap();
    assert_eq!(plugin_catalog::list(&s).unwrap().entries.len(), 1);
    // A changed byte, or the tool's signature over another file, does not verify.
    let mut changed = idx.clone();
    changed[0] = b' ';
    assert_eq!(
        sshsig::verify(&k, plugin_catalog::SIG_NAMESPACE, &changed, &sig),
        Err(SigError::Invalid)
    );
}

// ---- ADR-0120: the gallery's date, popularity and screenshots ----

/// The first bytes of a PNG of `w` x `h` pixels (all the hub reads; only a browser decodes an image).
fn png(w: u32, h: u32) -> Vec<u8> {
    let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    b.extend_from_slice(&w.to_be_bytes());
    b.extend_from_slice(&h.to_be_bytes());
    b.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
    b
}

/// The first bytes of an extended WebP of `w` x `h` pixels.
fn webp(w: u32, h: u32) -> Vec<u8> {
    let mut b = b"RIFF\x20\0\0\0WEBPVP8X\x0a\0\0\0\0\0\0\0".to_vec();
    b.extend_from_slice(&(w - 1).to_le_bytes()[..3]);
    b.extend_from_slice(&(h - 1).to_le_bytes()[..3]);
    b
}

fn shot(file: &str, bytes: &[u8], alt: &str) -> serde_json::Value {
    serde_json::json!({"file": file, "sha256": hex(&Sha256::digest(bytes)), "size": bytes.len(), "alt": alt})
}

fn index2(serial: u64, entries: Vec<serde_json::Value>) -> Vec<u8> {
    serde_json::json!({"agora-plugin-catalog": 2, "serial": serial, "plugins": entries})
        .to_string()
        .into_bytes()
}

#[test]
fn a_format_2_index_gives_dates_popularity_and_checked_screenshots() {
    let key = Key::new();
    let cat = Catalog::new();
    let a = package("demo", "1.0.0", &["agents.read"]);
    let (light, dark, huge, wrong) = (png(1280, 800), webp(800, 600), png(4000, 800), png(10, 10));
    cat.set("/cat/shots/light.png", ok(light.clone()));
    cat.set("/cat/shots/dark.webp", ok(dark.clone()));
    cat.set("/cat/shots/huge.png", ok(huge.clone()));
    // A PNG named as a WebP, and one whose bytes are not the index's.
    cat.set("/cat/shots/named.webp", ok(wrong.clone()));
    cat.set("/cat/shots/other.png", ok(png(20, 20)));
    let mut e = entry("demo", "1.0.0", &["agents.read"], &a);
    e["added"] = "2026-11-20".into();
    e["popularity"] = 128.into();
    e["screenshots"] = serde_json::json!([
        shot("shots/light.png", &light, "Light"),
        shot("shots/huge.png", &huge, "Too big"),
        shot("shots/dark.webp", &dark, "Dark"),
        shot("shots/named.webp", &wrong, ""),
    ]);
    let mut second = entry("plain", "1.0.0", &["agents.read"], &a);
    second["screenshots"] = serde_json::json!([
        shot("shots/other.png", &png(30, 30), ""),
        shot("shots/gone.png", &light, "")
    ]);
    cat.publish(&key, &index2(5, vec![e, second]));
    let s = store_with(&key);
    // 2026-11-26, so 2026-11-20 is in the past.
    let f = plugin_catalog::update(&s, &cat.url, roomy(), 1_795_651_200_000).unwrap();
    assert!(f.refused.is_empty(), "{:?}", f.refused);
    let mut why: Vec<(String, usize, &str)> = f
        .shots_refused
        .iter()
        .map(|(id, n, r)| (id.clone(), *n, r.code()))
        .collect();
    why.sort();
    assert_eq!(
        why,
        [
            ("demo".to_string(), 1, "pixels"),
            ("demo".to_string(), 3, "kind"),
            ("plain".to_string(), 0, "hash"),
            ("plain".to_string(), 1, "fetch"),
        ]
    );
    let list = plugin_catalog::list(&s).unwrap();
    let demo = list.entries.iter().find(|e| e.plugin_id == "demo").unwrap();
    assert_eq!((demo.added.as_str(), demo.popularity), ("2026-11-20", 128));
    assert_eq!(
        demo.screenshots,
        ["Light", "Dark"],
        "the kept ones, renumbered in order"
    );
    let plain = list
        .entries
        .iter()
        .find(|e| e.plugin_id == "plain")
        .unwrap();
    assert!(plain.screenshots.is_empty() && plain.added.is_empty() && plain.popularity == 0);
    let first = plugin_catalog::screenshot(&s, "demo", 0).unwrap();
    assert_eq!(
        (
            first.media_type.as_str(),
            first.data.as_slice(),
            first.alt.as_str()
        ),
        ("image/png", light.as_slice(), "Light")
    );
    let second = plugin_catalog::screenshot(&s, "demo", 1).unwrap();
    assert_eq!(
        (second.media_type.as_str(), second.data.as_slice()),
        ("image/webp", dark.as_slice())
    );
    assert_eq!(
        plugin_catalog::screenshot(&s, "demo", 2).unwrap_err().code,
        agora_protocol::ErrorCode::PluginNotFound
    );
    assert_eq!(
        plugin_catalog::screenshot(&s, "nope", 0).unwrap_err().code,
        agora_protocol::ErrorCode::PluginNotFound
    );
    plugin_catalog::set_on(&s, false, 3).unwrap();
    assert_eq!(
        plugin_catalog::screenshot(&s, "demo", 0).unwrap_err().code,
        agora_protocol::ErrorCode::PluginCatalogOff
    );
    // The next update replaces them with its catalog: one without screenshots leaves none.
    plugin_catalog::set_on(&s, true, 4).unwrap();
    cat.publish(
        &key,
        &index2(6, vec![entry("demo", "1.0.0", &["agents.read"], &a)]),
    );
    plugin_catalog::update(&s, &cat.url, roomy(), 1_795_651_200_000).unwrap();
    assert!(plugin_catalog::list(&s).unwrap().entries[0]
        .screenshots
        .is_empty());
    assert_eq!(
        plugin_catalog::screenshot(&s, "demo", 0).unwrap_err().code,
        agora_protocol::ErrorCode::PluginNotFound
    );
}

#[test]
fn format_1_knows_no_gallery_field_and_a_date_after_today_is_refused() {
    let key = Key::new();
    let cat = Catalog::new();
    let a = package("demo", "1.0.0", &["agents.read"]);
    let mut e = entry("demo", "1.0.0", &["agents.read"], &a);
    e["popularity"] = 3.into();
    cat.publish(&key, &index(7, vec![e.clone()]));
    let s = store_with(&key);
    let f = plugin_catalog::update(&s, &cat.url, roomy(), 1_795_651_200_000).unwrap();
    assert_eq!(f.refused.len(), 1);
    assert_eq!(f.refused[0].1.code(), "unknown-field");
    // Format 2: a day after the update's (UTC) is refused, today's is kept.
    let mut later = e.clone();
    later["added"] = "2026-11-27".into();
    let mut today = entry("today", "1.0.0", &["agents.read"], &a);
    today["added"] = "2026-11-26".into();
    let mut bad = entry("bad", "1.0.0", &["agents.read"], &a);
    bad["added"] = "2026-02-30".into();
    cat.publish(&key, &index2(8, vec![later, today, bad]));
    let f = plugin_catalog::update(&s, &cat.url, roomy(), 1_795_651_200_000 + 3_600_000).unwrap();
    let mut refused: Vec<(String, &str)> = f
        .refused
        .iter()
        .map(|(id, r)| (id.clone(), r.code()))
        .collect();
    refused.sort();
    assert_eq!(
        refused,
        [("bad".to_string(), "field"), ("demo".to_string(), "field")]
    );
    assert_eq!(f.entries.len(), 1);
    assert_eq!(f.entries[0].id, "today");
}
