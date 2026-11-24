#![allow(clippy::unwrap_used)]
//! ADR-0087 rung 3: the installer against a fake agent, a fake fetcher and a hostname table. Nothing here uses the network. The
//! program the fake fetcher "downloads" is a shell script, so the tests that run it are skipped where scripts do not run (Windows);
//! every test that fails before a program runs is checked on every system.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use agora_adapters::agent_install::{
    blocked_address, compare_versions, curl_args, install, last_seen_version, plan,
    remember_list_version, Approval, Change, Code, Fault, FetchRequest, Fetcher, InstallError,
    Plan, Resolver, SignatureCheck, SshKeygen, Tools,
};
use agora_adapters::agent_packages::{verify_list, AgentList};
use ring::digest;
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::json;

const PLATFORM: &str = "linux-x86_64";
const GOOD: &str = "#!/bin/sh\necho \"fake-agent 1.2.3\"\n";

struct Home(PathBuf);
impl Home {
    fn new() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "agora-install-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn agents(&self) -> PathBuf {
        self.0.join("agents")
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
fn sha(text: &str) -> String {
    hex(digest::digest(&digest::SHA256, text.as_bytes()).as_ref())
}

struct Opts {
    version: &'static str,
    sha: String,
    cmd: Option<&'static str>,
    signature: bool,
    revoked: bool,
    platform: &'static str,
    /// The one program the entry provides (the launcher's name).
    provides: &'static str,
}
impl Opts {
    fn new(version: &'static str, body: &str) -> Self {
        Self {
            version,
            sha: sha(body),
            cmd: None,
            signature: false,
            revoked: false,
            platform: PLATFORM,
            provides: "fake-agent",
        }
    }
}

/// A list signed by a fake key made from a public seed, opened by the real `verify_list`.
fn list(o: &Opts) -> AgentList {
    let (envelope, key) = signed_list(o, 3);
    verify_list(&envelope, &key, 1, 0).unwrap()
}

/// The signed envelope of list `version` and the root key that opens it.
fn signed_list(o: &Opts, version: u64) -> (String, String) {
    let root = Ed25519KeyPair::from_seed_unchecked(&[1; 32]).unwrap();
    let publisher = Ed25519KeyPair::from_seed_unchecked(&[2; 32]).unwrap();
    let mut dl = json!({"url": "https://agents.example.com/fake-agent", "sha256": o.sha});
    if let Some(c) = o.cmd {
        dl["cmd"] = json!(c);
    }
    if o.signature {
        dl["signature_url"] = json!("https://agents.example.com/fake-agent.sig");
        dl["signature_key"] = json!("vendor");
    }
    let payload = json!({
        "version": version, "expires_ms": 2_000_000_000_000_u64,
        "publishers": [{"id": "fake-official", "key": hex(publisher.public_key().as_ref()), "revoked": o.revoked}],
        "agents": [{
            "name": "fake-agent", "display": "Fake Agent",
            "changelog": "https://agents.example.com/changes", "homepage": "https://agents.example.com",
            "publisher": "fake-official", "version_policy": "track", "version": o.version,
            "provides": [o.provides],
            "verify": {"args": ["--version"], "regex": "fake-agent [0-9]+"},
            "installer": {"kind": "binary-download", "platforms": {o.platform: dl}},
        }],
    })
    .to_string();
    let envelope =
        json!({"payload": payload, "sig": hex(root.sign(payload.as_bytes()).as_ref())}).to_string();
    (envelope, hex(root.public_key().as_ref()))
}

#[derive(Default)]
struct Net {
    /// url -> body
    bodies: BTreeMap<String, Vec<u8>>,
    requests: RefCell<Vec<(String, IpAddr)>>,
    hosts: BTreeMap<String, Vec<IpAddr>>,
}
impl Net {
    fn serving(body: &str) -> Self {
        let mut n = Net::default();
        n.bodies
            .insert("https://agents.example.com/fake-agent".into(), body.into());
        n.bodies.insert(
            "https://agents.example.com/fake-agent.sig".into(),
            b"sig".to_vec(),
        );
        n.hosts.insert(
            "agents.example.com".into(),
            vec!["93.184.216.34".parse().unwrap()],
        );
        n
    }
    fn resolving(mut self, ips: &[&str]) -> Self {
        self.hosts.insert(
            "agents.example.com".into(),
            ips.iter().map(|i| i.parse().unwrap()).collect(),
        );
        self
    }
}
impl Resolver for Net {
    fn resolve(&self, host: &str, _port: u16) -> io::Result<Vec<IpAddr>> {
        self.hosts
            .get(host)
            .cloned()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }
}
impl Fetcher for Net {
    fn fetch(&self, r: &FetchRequest<'_>, sink: &mut dyn Write) -> Result<(), InstallError> {
        self.requests.borrow_mut().push((r.url.to_string(), r.addr));
        let body = self.bodies.get(r.url).ok_or(InstallError {
            code: Code::DownloadFailed,
            detail: "http_status".into(),
        })?;
        sink.write_all(body).map_err(|_| InstallError {
            code: Code::DownloadFailed,
            detail: "stopped".into(),
        })
    }
}
struct Sig(bool);
impl SignatureCheck for Sig {
    fn check(&self, _: &Path, _: &Path, _: &Path) -> Result<(), InstallError> {
        if self.0 {
            Ok(())
        } else {
            Err(InstallError {
                code: Code::SignatureInvalid,
                detail: "bad_signature".into(),
            })
        }
    }
}

fn tools<'a>(n: &'a Net, s: &'a Sig) -> Tools<'a> {
    Tools::new(n, n, s)
}
fn approval(p: &Plan) -> Approval {
    Approval::typed(p, "fake-agent", Some(&p.version)).unwrap()
}
fn run(
    home: &Home,
    o: &Opts,
    body: &str,
) -> Result<agora_adapters::agent_install::Outcome, InstallError> {
    let l = list(o);
    let p = plan(&l, "fake-agent", o.platform, &home.0)?;
    let net = Net::serving(body);
    install(&p, &approval(&p), &home.0, &tools(&net, &Sig(true)))
}
fn skip() -> bool {
    !agora_platform::SHELL_SCRIPTS_RUN
}
fn nothing_live(home: &Home, version: &str) {
    let a = home.agents();
    assert!(
        !a.join("fake-agent").join(version).exists(),
        "version folder left"
    );
    assert!(
        std::fs::read_dir(&a)
            .unwrap()
            .flatten()
            .all(|e| !e.file_name().to_string_lossy().starts_with(".tmp-")),
        "temporary folder left"
    );
}
/// The install lock is released: the file stays (it is never removed), empty, and free.
fn released(home: &Home) -> bool {
    std::fs::read_to_string(home.agents().join(".lock")).is_ok_and(|t| t.is_empty())
}
fn launcher(home: &Home) -> PathBuf {
    home.agents()
        .join("bin")
        .join(format!("fake-agent{}", agora_platform::EXECUTABLE_SUFFIX))
}

#[test]
fn a_verified_download_is_installed_and_linked() {
    if skip() {
        return;
    }
    let home = Home::new();
    let o = run(&home, &Opts::new("1.2.3", GOOD), GOOD).unwrap();
    assert_eq!(o.version, "1.2.3");
    assert!(!o.unchanged);
    assert!(o.dir.join("fake-agent").is_file());
    assert!(launcher(&home).exists());
    assert_eq!(
        agora_adapters::agent_install::installed_version(&home.0, "fake-agent").as_deref(),
        Some("1.2.3")
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode(&o.dir), 0o700);
        assert_eq!(mode(&o.dir.join("fake-agent")), 0o700);
        assert_eq!(mode(&home.agents()), 0o700);
        // The launcher is a relative link into the version folder.
        let target = std::fs::read_link(launcher(&home)).unwrap();
        assert!(target.is_relative() && target.ends_with("fake-agent"));
    }
    // The lock is released.
    assert!(released(&home));
    nothing_live(&home, "none");
}

#[test]
fn installing_the_same_version_again_changes_nothing() {
    if skip() {
        return;
    }
    let home = Home::new();
    run(&home, &Opts::new("1.2.3", GOOD), GOOD).unwrap();
    let l = list(&Opts::new("1.2.3", GOOD));
    let p = plan(&l, "fake-agent", PLATFORM, &home.0).unwrap();
    assert_eq!(p.change, Change::Same);
    let net = Net::default();
    let o = install(&p, &approval(&p), &home.0, &tools(&net, &Sig(true))).unwrap();
    assert!(o.unchanged);
    assert!(net.requests.borrow().is_empty());
}

#[test]
fn a_wrong_checksum_installs_nothing() {
    let home = Home::new();
    let mut o = Opts::new("1.2.3", GOOD);
    o.sha = sha("something else");
    let e = run(&home, &o, GOOD).unwrap_err();
    assert_eq!(e.code, Code::ChecksumMismatch);
    nothing_live(&home, "1.2.3");
    assert!(!launcher(&home).exists());
    assert_eq!(e.code.code(), "agent.install.checksum_mismatch");
}

#[test]
fn an_empty_download_is_not_a_match_even_for_the_hash_of_nothing() {
    let home = Home::new();
    let mut o = Opts::new("1.2.3", "");
    o.sha = sha("");
    assert_eq!(run(&home, &o, "").unwrap_err().code, Code::ChecksumMismatch);
}

#[test]
fn a_bad_signature_a_missing_key_and_a_good_signature() {
    let home = Home::new();
    let mut o = Opts::new("1.2.3", GOOD);
    o.signature = true;
    let l = list(&o);
    let p = plan(&l, "fake-agent", PLATFORM, &home.0).unwrap();
    assert_eq!(p.signature.as_ref().unwrap().1, "vendor");
    let net = Net::serving(GOOD);
    // No key file: refused before any download.
    let e = install(&p, &approval(&p), &home.0, &tools(&net, &Sig(true))).unwrap_err();
    assert_eq!(
        (e.code, e.detail.as_str()),
        (Code::SignatureInvalid, "key_missing")
    );
    assert!(net.requests.borrow().is_empty());
    std::fs::create_dir_all(home.0.join("agent-keys")).unwrap();
    std::fs::write(home.0.join("agent-keys").join("vendor.pub"), "x").unwrap();
    let e = install(&p, &approval(&p), &home.0, &tools(&net, &Sig(false))).unwrap_err();
    assert_eq!(e.code, Code::SignatureInvalid);
    nothing_live(&home, "1.2.3");
    assert!(!launcher(&home).exists());
    if skip() {
        return;
    }
    install(&p, &approval(&p), &home.0, &tools(&net, &Sig(true))).unwrap();
    assert!(launcher(&home).exists());
}

#[test]
fn the_real_ssh_signature_check_accepts_a_signature_and_refuses_a_changed_file() {
    // Needs ssh-keygen; a machine without it skips (the CLI reports `tool_missing` there).
    let dir = Home::new();
    let key = dir.0.join("key");
    let made = std::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", "t", "-f"])
        .arg(&key)
        .output();
    if !made.is_ok_and(|o| o.status.success()) {
        return;
    }
    let file = dir.0.join("data");
    std::fs::write(&file, "program").unwrap();
    let signed = std::process::Command::new("ssh-keygen")
        .args(["-Y", "sign", "-n", "file", "-f"])
        .arg(&key)
        .arg(&file)
        .output()
        .unwrap();
    if !signed.status.success() {
        return;
    }
    let public = std::fs::read_to_string(dir.0.join("key.pub")).unwrap();
    let allowed = dir.0.join("allowed");
    std::fs::write(&allowed, format!("agora-agent {public}")).unwrap();
    let sig = dir.0.join("data.sig");
    assert!(SshKeygen.check(&allowed, &file, &sig).is_ok());
    std::fs::write(&file, "program!").unwrap();
    assert_eq!(
        SshKeygen.check(&allowed, &file, &sig).unwrap_err().code,
        Code::SignatureInvalid
    );
}

#[test]
fn a_platform_the_entry_does_not_list_is_refused() {
    let home = Home::new();
    let l = list(&Opts::new("1.2.3", GOOD));
    let e = plan(&l, "fake-agent", "darwin-aarch64", &home.0).unwrap_err();
    assert_eq!(e.code, Code::UnsupportedPlatform);
    assert_eq!(e.code.code(), "agent.install.unsupported_platform");
    assert_eq!(
        plan(&l, "unknown", PLATFORM, &home.0).unwrap_err().code,
        Code::NotOffered
    );
}

#[test]
fn a_revoked_publisher_is_refused() {
    let home = Home::new();
    let mut o = Opts::new("1.2.3", GOOD);
    o.revoked = true;
    let e = run(&home, &o, GOOD).unwrap_err();
    assert_eq!(
        (e.code, e.detail.as_str()),
        (Code::Revoked, "fake-official")
    );
}

#[test]
fn an_archive_is_not_installable_yet() {
    let home = Home::new();
    let mut o = Opts::new("1.2.3", GOOD);
    o.cmd = Some("bin/fake-agent");
    let e = run(&home, &o, GOOD).unwrap_err();
    assert_eq!(
        (e.code, e.detail.as_str()),
        (Code::UnsupportedFormat, "archive")
    );
}

#[test]
fn private_and_special_addresses_are_never_downloaded_from() {
    let blocked = [
        "127.0.0.1",
        "127.8.9.1",
        "10.0.0.1",
        "172.16.5.5",
        "192.168.1.1",
        "169.254.169.254",
        "100.64.0.1",
        "100.127.255.255",
        "0.0.0.0",
        "224.0.0.1",
        "255.255.255.255",
        "192.0.0.8",
        "198.18.0.1",
        "240.0.0.1",
        "::1",
        "::",
        "fc00::1",
        "fd12:3456::1",
        "fe80::1",
        "::ffff:10.0.0.1",
        "::ffff:127.0.0.1",
        "::ffff:169.254.169.254",
        "64:ff9b::a00:1",
        "2002:a00:1::1",
        "2001:db8::1",
        "ff02::1",
        "fec0::1",
        "::10.0.0.1",
    ];
    for ip in blocked {
        assert!(
            blocked_address(ip.parse().unwrap()),
            "{ip} should be refused"
        );
    }
    for ip in [
        "93.184.216.34",
        "8.8.8.8",
        "1.1.1.1",
        "2606:4700:4700::1111",
        "::ffff:8.8.8.8",
        "100.128.0.1",
    ] {
        assert!(!blocked_address(ip.parse().unwrap()), "{ip} should pass");
    }
    // Through the whole install, with a name table that points a public-looking name at each.
    let home = Home::new();
    let l = list(&Opts::new("1.2.3", GOOD));
    let p = plan(&l, "fake-agent", PLATFORM, &home.0).unwrap();
    for ip in blocked {
        let net = Net::serving(GOOD).resolving(&[ip]);
        let e = install(&p, &approval(&p), &home.0, &tools(&net, &Sig(true))).unwrap_err();
        assert_eq!(
            (e.code, e.detail.as_str()),
            (Code::DownloadFailed, "blocked_address"),
            "{ip}"
        );
        assert!(net.requests.borrow().is_empty(), "{ip} was requested");
    }
    // One private answer among public ones refuses the name (the start of a rebinding trick).
    let net = Net::serving(GOOD).resolving(&["93.184.216.34", "10.0.0.1"]);
    let e = install(&p, &approval(&p), &home.0, &tools(&net, &Sig(true))).unwrap_err();
    assert_eq!(e.detail, "blocked_address");
    // A name that does not resolve.
    let mut net = Net::serving(GOOD);
    net.hosts.clear();
    assert_eq!(
        install(&p, &approval(&p), &home.0, &tools(&net, &Sig(true)))
            .unwrap_err()
            .detail,
        "resolve"
    );
    nothing_live(&home, "1.2.3");
}

#[test]
fn the_checked_address_is_the_one_requested() {
    if skip() {
        return;
    }
    let home = Home::new();
    let l = list(&Opts::new("1.2.3", GOOD));
    let p = plan(&l, "fake-agent", PLATFORM, &home.0).unwrap();
    let net = Net::serving(GOOD).resolving(&["93.184.216.34", "93.184.216.35"]);
    install(&p, &approval(&p), &home.0, &tools(&net, &Sig(true))).unwrap();
    assert_eq!(
        net.requests.borrow()[0].1,
        "93.184.216.34".parse::<IpAddr>().unwrap()
    );
}

#[test]
fn addresses_that_are_not_plain_https_names_are_refused() {
    let home = Home::new();
    let l = list(&Opts::new("1.2.3", GOOD));
    let base = plan(&l, "fake-agent", PLATFORM, &home.0).unwrap();
    for url in [
        "http://agents.example.com/x",
        "https://user@agents.example.com/x",
        "https://agents.example.com:0/x",
        "https://agents.example.com:99999/x",
        "https://93.184.216.34/x",
        "https://[::1]/x",
        "https://localhost/x",
        "ftp://agents.example.com/x",
    ] {
        let mut p = base.clone();
        p.url = url.into();
        let net = Net::serving(GOOD);
        let e = install(&p, &approval(&p), &home.0, &tools(&net, &Sig(true))).unwrap_err();
        assert_eq!(e.code, Code::DownloadFailed, "{url}");
        assert!(net.requests.borrow().is_empty(), "{url}");
    }
}

#[test]
fn curl_gets_the_pin_the_protocol_limits_and_no_redirects() {
    let r = FetchRequest {
        url: "https://agents.example.com:8443/a",
        host: "agents.example.com",
        port: 8443,
        addr: "93.184.216.34".parse().unwrap(),
        max_bytes: 1000,
        timeout_secs: 60,
    };
    let a = curl_args(&r);
    assert_eq!(a[0], "-q", "a .curlrc is ignored: -q must come first");
    let has = |pair: &[&str]| a.windows(pair.len()).any(|w| w == pair);
    assert!(has(&["--resolve", "agents.example.com:8443:93.184.216.34"]));
    assert!(has(&["--proto", "=https"]) && has(&["--proto-redir", "=https"]));
    assert!(a.contains(&"--globoff".to_string()));
    assert!(has(&["--max-redirs", "0"]) && !a.iter().any(|x| x == "--location" || x == "-L"));
    assert!(
        has(&["--noproxy", "*"]) && has(&["--max-filesize", "1000"]) && has(&["--max-time", "60"])
    );
    assert!(a.contains(&"--fail".to_string()));
    assert_eq!(a.last().unwrap(), "https://agents.example.com:8443/a");
    let v6 = FetchRequest {
        addr: "2606:4700::1111".parse().unwrap(),
        ..r
    };
    assert!(curl_args(&v6).contains(&"agents.example.com:8443:[2606:4700::1111]".to_string()));
}

#[test]
fn a_download_over_the_limit_is_stopped_and_leaves_nothing() {
    let home = Home::new();
    let l = list(&Opts::new("1.2.3", GOOD));
    let p = plan(&l, "fake-agent", PLATFORM, &home.0).unwrap();
    let net = Net::serving(GOOD);
    let mut t = tools(&net, &Sig(true));
    t.max_download = 10;
    let e = install(&p, &approval(&p), &home.0, &t).unwrap_err();
    assert_eq!(
        (e.code, e.detail.as_str()),
        (Code::DownloadFailed, "too_large")
    );
    nothing_live(&home, "1.2.3");
}

#[test]
fn a_failed_verify_keeps_the_old_version_linked() {
    if skip() {
        return;
    }
    let home = Home::new();
    run(&home, &Opts::new("1.0.0", GOOD), GOOD).unwrap();
    let before = std::fs::read_link(launcher(&home)).unwrap();
    for (body, detail) in [
        ("#!/bin/sh\necho nothing useful\n", "no_match"),
        (
            "#!/bin/sh\necho \"fake-agent 2.0.0\"\nexit 3\n",
            "exit_status",
        ),
    ] {
        let e = run(&home, &Opts::new("2.0.0", body), body).unwrap_err();
        assert_eq!((e.code, e.detail.as_str()), (Code::VerifyFailed, detail));
        assert_eq!(std::fs::read_link(launcher(&home)).unwrap(), before);
        assert_eq!(
            agora_adapters::agent_install::installed_version(&home.0, "fake-agent").as_deref(),
            Some("1.0.0")
        );
        nothing_live(&home, "2.0.0");
    }
    assert!(home
        .agents()
        .join("fake-agent")
        .join("1.0.0")
        .join("fake-agent")
        .is_file());
}

#[test]
fn verify_runs_with_a_scrubbed_environment_and_a_time_limit() {
    if skip() {
        return;
    }
    let home = Home::new();
    // The test process has a PATH with more in it; the program must not see it.
    let body = "#!/bin/sh\nif [ \"$PATH\" = \"/usr/bin:/bin\" ] && [ -z \"$CARGO_HOME\" ] && [ -z \"$USER\" ]; then echo \"fake-agent 1\"; else echo \"leaked $PATH\"; fi\n";
    run(&home, &Opts::new("1.0.0", body), body).unwrap();
    let slow = "#!/bin/sh\nexec sleep 30\n";
    let l = list(&Opts::new("1.0.1", slow));
    let p = plan(&l, "fake-agent", PLATFORM, &home.0).unwrap();
    let net = Net::serving(slow);
    let mut t = tools(&net, &Sig(true));
    t.verify_timeout = Duration::from_millis(300);
    let started = std::time::Instant::now();
    let e = install(&p, &approval(&p), &home.0, &t).unwrap_err();
    assert_eq!(e.code, Code::Timeout);
    assert!(started.elapsed() < Duration::from_secs(8));
    nothing_live(&home, "1.0.1");
}

#[test]
fn an_older_version_needs_a_typed_go_back() {
    if skip() {
        return;
    }
    let home = Home::new();
    run(&home, &Opts::new("2.0.0", GOOD), GOOD).unwrap();
    let l = list(&Opts::new("1.0.0", GOOD));
    let p = plan(&l, "fake-agent", PLATFORM, &home.0).unwrap();
    assert_eq!(
        (p.change, p.installed.as_deref()),
        (Change::Downgrade, Some("2.0.0"))
    );
    assert_eq!(
        Approval::typed(&p, "fake-agent", None).unwrap_err().code,
        Code::Downgrade
    );
    assert_eq!(
        Approval::typed(&p, "fake-agent", Some("2.0.0"))
            .unwrap_err()
            .code,
        Code::NotApproved
    );
    let a = Approval::typed(&p, "fake-agent\n", Some("1.0.0")).unwrap();
    let net = Net::serving(GOOD);
    let o = install(&p, &a, &home.0, &tools(&net, &Sig(true))).unwrap();
    assert_eq!(o.previous.as_deref(), Some("2.0.0"));
    assert_eq!(
        agora_adapters::agent_install::installed_version(&home.0, "fake-agent").as_deref(),
        Some("1.0.0")
    );
    // The version it came from is kept as the previous one.
    assert!(home.agents().join("fake-agent").join("2.0.0").is_dir());
}

#[test]
fn only_the_new_and_the_previous_version_are_kept() {
    if skip() {
        return;
    }
    let home = Home::new();
    for v in ["1.0.0", "1.1.0", "1.2.0"] {
        run(&home, &Opts::new(v, GOOD), GOOD).unwrap();
    }
    let kept: Vec<String> = {
        let mut k: Vec<String> = std::fs::read_dir(home.agents().join("fake-agent"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        k.sort();
        k
    };
    assert_eq!(kept, ["1.1.0", "1.2.0", "current", "exe"]);
}

#[test]
fn the_owner_must_type_the_exact_name() {
    let home = Home::new();
    let l = list(&Opts::new("1.2.3", GOOD));
    let p = plan(&l, "fake-agent", PLATFORM, &home.0).unwrap();
    for typed in [
        "",
        "y",
        "yes",
        "Fake-Agent",
        "fake-agent ",
        " fake-agent",
        "fake-agent2",
        "fake-agent\nx",
    ] {
        let e = Approval::typed(&p, typed, None).unwrap_err();
        assert_eq!(e.code, Code::NotApproved, "{typed:?}");
        assert_eq!(e.code.code(), "agent.install.not_approved");
    }
    assert!(Approval::typed(&p, "fake-agent", None).is_ok());
    assert!(Approval::typed(&p, "fake-agent\r\n", None).is_ok());
}

#[test]
fn a_second_install_waits_out_a_live_lock_and_takes_over_a_dead_one() {
    let home = Home::new();
    let l = list(&Opts::new("1.2.3", GOOD));
    let p = plan(&l, "fake-agent", PLATFORM, &home.0).unwrap();
    std::fs::create_dir_all(home.agents()).unwrap();
    let lock = home.agents().join(".lock");
    // Held by a process that is alive (this one).
    std::fs::write(&lock, std::process::id().to_string()).unwrap();
    let net = Net::serving(GOOD);
    let e = install(&p, &approval(&p), &home.0, &tools(&net, &Sig(true))).unwrap_err();
    assert_eq!(e.code, Code::InProgress);
    assert_eq!(e.code.code(), "agent.install.in_progress");
    assert!(net.requests.borrow().is_empty());
    assert!(lock.exists(), "a lock somebody else holds is not removed");
    // Held by a process that has ended.
    let mut child = if agora_platform::SHELL_SCRIPTS_RUN {
        std::process::Command::new("true").spawn().unwrap()
    } else {
        std::process::Command::new("cmd")
            .args(["/C", "exit"])
            .spawn()
            .unwrap()
    };
    let dead = child.id();
    child.wait().unwrap();
    std::fs::write(&lock, dead.to_string()).unwrap();
    if skip() {
        return;
    }
    install(&p, &approval(&p), &home.0, &tools(&net, &Sig(true))).unwrap();
    assert!(released(&home));
    // The same, in the current format (pid, start, boot): an identity that cannot be read falls back to the pid, which is gone.
    std::fs::write(&lock, format!("{dead} 12345 some-boot")).unwrap();
    let l1 = list(&Opts::new("1.2.4", GOOD));
    let p1 = plan(&l1, "fake-agent", PLATFORM, &home.0).unwrap();
    install(&p1, &approval(&p1), &home.0, &tools(&net, &Sig(true))).unwrap();
    assert!(released(&home));
    // Garbage names no live owner, and nobody holds the system lock: the lock is free (audit F100).
    std::fs::write(&lock, "not a pid").unwrap();
    let l2 = list(&Opts::new("1.2.5", GOOD));
    let p2 = plan(&l2, "fake-agent", PLATFORM, &home.0).unwrap();
    install(&p2, &approval(&p2), &home.0, &tools(&net, &Sig(true))).unwrap();
    assert!(released(&home));
}

#[test]
fn what_a_crash_left_is_removed_by_the_next_install() {
    if skip() {
        return;
    }
    let home = Home::new();
    let left = home.agents().join(".tmp-424242");
    std::fs::create_dir_all(left.join("stage")).unwrap();
    std::fs::write(left.join("stage").join("fake-agent"), "half").unwrap();
    std::fs::create_dir_all(home.agents().join("bin")).unwrap();
    std::fs::write(home.agents().join("bin").join("fake-agent.tmp-9"), "x").unwrap();
    // A lock an older version renamed aside before it crashed.
    let aside = home.agents().join(".lock.stale-424242-0");
    std::fs::write(&aside, "424242").unwrap();
    run(&home, &Opts::new("1.2.3", GOOD), GOOD).unwrap();
    assert!(!left.exists());
    assert!(!aside.exists());
    assert!(!home.agents().join("bin").join("fake-agent.tmp-9").exists());
}

#[test]
fn the_list_version_seen_only_goes_up() {
    let home = Home::new();
    assert_eq!(last_seen_version(&home.0).unwrap(), 0);
    remember_list_version(&home.0, 5).unwrap();
    assert_eq!(last_seen_version(&home.0).unwrap(), 5);
    // Recording an older list is refused as a rollback (Codex's review of #997); the same version is already recorded.
    let e = remember_list_version(&home.0, 3).unwrap_err();
    assert_eq!(
        (e.code, e.detail.as_str()),
        (Code::IndexInvalid, "rollback")
    );
    assert_eq!(last_seen_version(&home.0).unwrap(), 5);
    remember_list_version(&home.0, 5).unwrap();
    remember_list_version(&home.0, 6).unwrap();
    assert_eq!(last_seen_version(&home.0).unwrap(), 6);
}

/// Codex's review of #997: a list is checked against the record, then recorded. A newer list recorded by another install between
/// those two steps must not let the older one through: recording it is refused as a rollback and the newer record stays.
#[test]
fn a_newer_list_recorded_after_the_check_refuses_the_older_one() {
    let home = Home::new();
    let o = Opts::new("1.2.3", GOOD);
    remember_list_version(&home.0, 4).unwrap();
    let seen = last_seen_version(&home.0).unwrap();
    let (envelope, key) = signed_list(&o, 5);
    let older = verify_list(&envelope, &key, 1, seen).unwrap();
    // Another ordinary install records list 7 meanwhile.
    remember_list_version(&home.0, 7).unwrap();
    let e = remember_list_version(&home.0, older.version).unwrap_err();
    assert_eq!(
        (e.code, e.detail.as_str()),
        (Code::IndexInvalid, "rollback")
    );
    assert_eq!(last_seen_version(&home.0).unwrap(), 7);
}

/// Codex's review of #997: the record is opened without following a link at its name, and what was opened is checked on the handle.
/// A link there (to a readable record, or to nothing) is refused, never read and never taken as "no record yet".
#[test]
fn a_link_at_the_list_state_is_refused_not_followed() {
    let home = Home::new();
    std::fs::create_dir_all(home.agents()).unwrap();
    let target = home.0.join("elsewhere.json");
    std::fs::write(&target, "{\"version\":1}\n").unwrap();
    let state = home.agents().join("list-state.json");
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(&target, &state).is_ok();
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_file(&target, &state).is_ok();
    #[cfg(not(any(unix, windows)))]
    let linked = false;
    if !linked {
        // Windows without the right to make links: nothing to test.
        return;
    }
    let refused = || {
        let e = last_seen_version(&home.0).unwrap_err();
        assert_eq!(
            (e.code, e.detail.as_str()),
            (Code::IndexInvalid, "list_state_unreadable")
        );
        assert_eq!(
            remember_list_version(&home.0, 2).unwrap_err().detail,
            "list_state_unreadable"
        );
    };
    refused();
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "{\"version\":1}\n"
    );
    // A link to nothing is not "no record yet" either, and nothing is written through it.
    std::fs::remove_file(&target).unwrap();
    refused();
    assert!(!target.exists());
    assert!(std::fs::symlink_metadata(&state)
        .unwrap()
        .file_type()
        .is_symlink());
}

/// #923: a `list-state.json` that is there but cannot be read or understood is never taken as "no list seen yet" (version 0): that
/// let any older list through and then overwrote the state with its lower version. Every list is refused until the owner acts.
type MakeState = Box<dyn Fn(&Path)>;

fn unreadable_states() -> Vec<(&'static str, MakeState)> {
    let write =
        |text: String| -> MakeState { Box::new(move |p: &Path| std::fs::write(p, &text).unwrap()) };
    vec![
        ("garbage", write("not json at all".into())),
        ("truncated", write("{\"version\":".into())),
        ("empty", write(String::new())),
        ("wrong_shape", write("{\"v\":5}\n".into())),
        ("negative", write("{\"version\":-1}\n".into())),
        ("fraction", write("{\"version\":5.5}\n".into())),
        ("not_an_object", write("5\n".into())),
        // Other containers and values (Codex's review of #997): a derived reader took a positional array.
        ("array", write("[5]\n".into())),
        ("string", write("\"5\"\n".into())),
        ("null", write("null\n".into())),
        ("version_null", write("{\"version\":null}\n".into())),
        ("version_string", write("{\"version\":\"5\"}\n".into())),
        // A repeated `version` is not read as either value (Codex's review of #997).
        (
            "duplicate_version",
            write("{\"version\":1,\"version\":9}\n".into()),
        ),
        (
            "trailing",
            write("{\"version\":5}\n{\"version\":1}\n".into()),
        ),
        // Valid JSON, but far larger than the state ever is: not read into memory.
        (
            "oversized",
            write(format!("{{\"version\":5{}}}", " ".repeat(64 * 1024))),
        ),
        (
            "directory",
            Box::new(|p: &Path| std::fs::create_dir_all(p.join("inside")).unwrap()),
        ),
    ]
}

#[test]
fn an_unreadable_list_state_refuses_every_list_and_is_left_as_it_was() {
    for (case, make) in unreadable_states() {
        let home = Home::new();
        std::fs::create_dir_all(home.agents()).unwrap();
        let state = home.agents().join("list-state.json");
        make(&state);
        let before = std::fs::read(&state).ok();
        let e = last_seen_version(&home.0).unwrap_err();
        assert_eq!(
            (e.code, e.detail.as_str()),
            (Code::IndexInvalid, "list_state_unreadable"),
            "{case}"
        );
        for offered in [1, 3, 5, 9] {
            let e = remember_list_version(&home.0, offered).unwrap_err();
            assert_eq!(
                (e.code, e.detail.as_str()),
                (Code::IndexInvalid, "list_state_unreadable"),
                "{case}: list version {offered} was not refused"
            );
        }
        assert_eq!(
            std::fs::read(&state).ok(),
            before,
            "{case}: the state was changed"
        );
        assert_eq!(state.is_dir(), case == "directory", "{case}");
    }
}

#[cfg(unix)]
#[test]
fn a_list_state_this_user_cannot_read_refuses_every_list() {
    use std::os::unix::fs::PermissionsExt;
    let home = Home::new();
    remember_list_version(&home.0, 5).unwrap();
    let state = home.agents().join("list-state.json");
    std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&state).is_ok() {
        // Running as root: the permission does not stop a read, so there is nothing to test.
        return;
    }
    let e = last_seen_version(&home.0).unwrap_err();
    assert_eq!(
        (e.code, e.detail.as_str()),
        (Code::IndexInvalid, "list_state_unreadable")
    );
    let e = remember_list_version(&home.0, 3).unwrap_err();
    assert_eq!(
        (e.code, e.detail.as_str()),
        (Code::IndexInvalid, "list_state_unreadable")
    );
    assert_eq!(
        remember_list_version(&home.0, 9).unwrap_err().detail,
        "list_state_unreadable"
    );
    std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        std::fs::read_to_string(&state).unwrap(),
        "{\"version\":5}\n"
    );
}

/// #923, the way out: once the owner removes the file, the next list is the first one again (accepted and recorded); a newer list
/// is still accepted over a readable state and an older one is still refused.
#[test]
fn a_missing_list_state_accepts_and_records_the_first_list() {
    let home = Home::new();
    std::fs::create_dir_all(home.agents()).unwrap();
    let state = home.agents().join("list-state.json");
    std::fs::write(&state, "{\"version\":").unwrap();
    assert!(remember_list_version(&home.0, 4).is_err());
    std::fs::remove_file(&state).unwrap();
    remember_list_version(&home.0, 4).unwrap();
    assert_eq!(
        std::fs::read_to_string(&state).unwrap(),
        "{\"version\":4}\n"
    );
    remember_list_version(&home.0, 6).unwrap();
    assert_eq!(
        std::fs::read_to_string(&state).unwrap(),
        "{\"version\":6}\n"
    );
    // No temporary file is left beside it (the write is a rename over the old file).
    let names: Vec<String> = std::fs::read_dir(home.agents())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("list-state"))
        .collect();
    assert_eq!(names, ["list-state.json"]);
}

/// The whole check as `agora agent install` runs it (the state, then `verify_list`, then the record): over a readable state an
/// older signed list is a rollback, an equal or newer one is accepted; over a damaged state no list is opened at all.
#[test]
fn an_older_signed_list_is_refused_over_a_readable_state_and_every_list_over_a_damaged_one() {
    let home = Home::new();
    let o = Opts::new("1.2.3", GOOD);
    let open = |version: u64| -> Result<u64, String> {
        let seen = last_seen_version(&home.0).map_err(|e| e.detail)?;
        let (envelope, key) = signed_list(&o, version);
        let l = verify_list(&envelope, &key, 1, seen).map_err(|e| format!("{e:?}"))?;
        remember_list_version(&home.0, l.version).map_err(|e| e.detail)?;
        Ok(l.version)
    };
    assert_eq!(open(5), Ok(5));
    assert!(open(3).unwrap_err().contains("Rollback"));
    assert_eq!(open(5), Ok(5));
    assert_eq!(open(6), Ok(6));
    std::fs::write(home.agents().join("list-state.json"), "{\"vers").unwrap();
    for version in [3, 6, 7] {
        assert_eq!(open(version), Err("list_state_unreadable".into()));
    }
}

#[test]
fn versions_compare_as_semver_including_prerelease_identifiers() {
    use std::cmp::Ordering::{Equal, Greater, Less};
    let cases = [
        ("1.0.0-rc.2", "1.0.0-rc.10", Less),
        ("1.0.0-rc.10", "1.0.0-rc.2", Greater),
        ("1.0.0-1", "1.0.0-alpha", Less),
        ("1.0.0-alpha", "1.0.0-alpha.1", Less),
        ("1.0.0-alpha.1", "1.0.0-alpha.beta", Less),
        ("1.0.0-alpha.beta", "1.0.0-beta", Less),
        ("1.0.0-beta.11", "1.0.0-rc.1", Less),
        ("1.0.0-rc.1", "1.0.0", Less),
        ("1.0.0", "1.0.0-rc.1", Greater),
        ("1.0.0-rc.01", "1.0.0-rc.1", Equal),
        ("1.9.0", "1.10.0", Less),
        ("1.0.0+a", "1.0.0+b", Equal),
        ("2.0.0-rc.1", "1.99.99", Greater),
    ];
    for (a, b, want) in cases {
        assert_eq!(compare_versions(a, b), want, "{a} vs {b}");
    }
}

#[test]
fn a_prerelease_that_looks_newer_as_text_is_still_a_downgrade() {
    let home = Home::new();
    // Installed rc.10; the list names rc.2.
    std::fs::create_dir_all(home.agents().join("fake-agent")).unwrap();
    std::fs::write(
        home.agents().join("fake-agent").join("current"),
        "1.0.0-rc.10",
    )
    .unwrap();
    let l = list(&Opts::new("1.0.0-rc.2", GOOD));
    let p = plan(&l, "fake-agent", PLATFORM, &home.0).unwrap();
    assert_eq!(p.change, Change::Downgrade);
}

/// Writes `.lock` and runs an install against it; returns the install's result.
fn with_lock(home: &Home, text: &str, age_secs: u64) -> Result<(), InstallError> {
    std::fs::create_dir_all(home.agents()).unwrap();
    let lock = home.agents().join(".lock");
    std::fs::write(&lock, text).unwrap();
    if age_secs > 0 {
        let when = std::time::SystemTime::now() - Duration::from_secs(age_secs);
        std::fs::File::options()
            .write(true)
            .open(&lock)
            .unwrap()
            .set_modified(when)
            .unwrap();
    }
    let l = list(&Opts::new("1.2.3", GOOD));
    let p = plan(&l, "fake-agent", PLATFORM, &home.0).unwrap();
    let net = Net::serving(GOOD);
    install(&p, &approval(&p), &home.0, &tools(&net, &Sig(true))).map(|_| ())
}

#[test]
fn an_empty_or_garbage_lock_nobody_holds_is_free_whatever_its_age() {
    if skip() {
        return;
    }
    // Audit F100: the system lock tells who holds it; text that names no live owner (a released lock is empty) never blocks.
    for text in ["", "garbage", "0x12"] {
        for age in [0, 120] {
            let home = Home::new();
            with_lock(&home, text, age).unwrap();
            assert!(released(&home), "{text:?} {age}");
        }
    }
}

#[test]
fn a_lock_of_a_reused_pid_is_stale() {
    if skip() {
        return;
    }
    // This very pid, but a start time that is not this process's: a different process that got the pid later.
    let home = Home::new();
    with_lock(&home, &format!("{} 1 other-boot", std::process::id()), 0).unwrap();
    // The real identity of this process is alive.
    let home = Home::new();
    let i = agora_platform::process_identity(std::process::id()).unwrap();
    let alive = format!("{} {} {}", std::process::id(), i.started, i.boot);
    assert_eq!(
        with_lock(&home, &alive, 0).unwrap_err().code,
        Code::InProgress
    );
}

/// What reading the lock file gave while it was held, and what `during` returned.
type Seen = (io::Result<String>, Result<(), InstallError>);

/// A fetcher that, while its install holds the lock, runs `during` and records what reading the lock file gave.
struct During<'a> {
    net: &'a Net,
    lock: PathBuf,
    during: &'a dyn Fn() -> Result<(), InstallError>,
    seen: RefCell<Option<Seen>>,
}
impl Fetcher for During<'_> {
    fn fetch(&self, r: &FetchRequest<'_>, sink: &mut dyn Write) -> Result<(), InstallError> {
        let text = std::fs::read_to_string(&self.lock);
        *self.seen.borrow_mut() = Some((text, (self.during)()));
        self.net.fetch(r, sink)
    }
}

/// The text a lock held by process `pid` names.
fn owner_text(pid: u32) -> String {
    let i = agora_platform::process_identity(pid).unwrap();
    format!("{pid} {} {}", i.started, i.boot)
}

/// Runs on every system (review of audit F100: the Windows lock had no test). Where the fake agent cannot run, the install fails after
/// its download, which is still inside the lock.
#[test]
fn a_held_lock_names_its_owner_and_refuses_another_install_until_released() {
    // Audit F100: the holder keeps the system lock; another install (here another thread of this process, which writes the same
    // owner text) is refused while it is held, and the file is emptied, not removed, on release.
    let home = Home::new();
    let l = list(&Opts::new("1.2.3", GOOD));
    let p = plan(&l, "fake-agent", PLATFORM, &home.0).unwrap();
    let l2 = list(&Opts::new("1.2.4", GOOD));
    let p2 = plan(&l2, "fake-agent", PLATFORM, &home.0).unwrap();
    let second = || {
        std::thread::scope(|s| {
            s.spawn(|| {
                let net = Net::serving(GOOD);
                install(&p2, &approval(&p2), &home.0, &tools(&net, &Sig(true))).map(|_| ())
            })
            .join()
            .unwrap()
        })
    };
    let net = Net::serving(GOOD);
    let during = During {
        net: &net,
        lock: home.agents().join(".lock"),
        during: &second,
        seen: RefCell::new(None),
    };
    let sig = Sig(true);
    let first = install(&p, &approval(&p), &home.0, &Tools::new(&net, &during, &sig));
    if !skip() {
        first.unwrap();
    }
    let (text, other) = during.seen.into_inner().unwrap();
    if agora_platform::LOCK_FILES_MANDATORY {
        // Nobody else can read a held lock there (an older version judges it by its age instead).
        assert!(text.is_err(), "{text:?}");
    } else {
        assert_eq!(text.unwrap(), owner_text(std::process::id()));
    }
    assert_eq!(other.unwrap_err().code, Code::InProgress);
    assert!(released(&home));
    // Released: the next holder takes it (the list version is remembered under the same lock, on every system).
    remember_list_version(&home.0, 3).unwrap();
    assert_eq!(last_seen_version(&home.0).unwrap(), 3);
    if !skip() {
        let net = Net::serving(GOOD);
        install(&p2, &approval(&p2), &home.0, &tools(&net, &Sig(true))).unwrap();
    }
    assert!(released(&home));
}

#[test]
fn racing_installs_over_a_stale_lock_end_with_one_consistent_install() {
    if skip() {
        return;
    }
    let home = Home::new();
    std::fs::create_dir_all(home.agents()).unwrap();
    std::fs::write(home.agents().join(".lock"), "").unwrap();
    std::fs::File::options()
        .write(true)
        .open(home.agents().join(".lock"))
        .unwrap()
        .set_modified(std::time::SystemTime::now() - Duration::from_secs(300))
        .unwrap();
    let results: Vec<Result<(), InstallError>> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..6)
            .map(|_| {
                s.spawn(|| {
                    let l = list(&Opts::new("1.2.3", GOOD));
                    let p = plan(&l, "fake-agent", PLATFORM, &home.0).unwrap();
                    let net = Net::serving(GOOD);
                    install(&p, &approval(&p), &home.0, &tools(&net, &Sig(true))).map(|_| ())
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert!(results.iter().any(Result::is_ok), "{results:?}");
    for r in &results {
        if let Err(e) = r {
            assert_eq!(e.code, Code::InProgress, "{e:?}");
        }
    }
    assert!(launcher(&home).exists());
    assert!(released(&home));
    assert_eq!(
        agora_adapters::agent_install::installed_version(&home.0, "fake-agent").as_deref(),
        Some("1.2.3")
    );
}

/// Set for [`lock_holder_child`]: the Agora home it installs into.
const HOLDER_HOME: &str = "AGORA_TEST_INSTALL_LOCK_HOME";

/// A fetcher that, once its install holds the lock, writes `<home>/held-<pid>` and waits until its stdin closes.
struct HoldUntilStdinCloses {
    net: Net,
    home: PathBuf,
}
impl Fetcher for HoldUntilStdinCloses {
    fn fetch(&self, r: &FetchRequest<'_>, sink: &mut dyn Write) -> Result<(), InstallError> {
        std::fs::write(self.home.join(format!("held-{}", std::process::id())), "").unwrap();
        let _ = io::copy(&mut io::stdin().lock(), &mut io::sink());
        self.net.fetch(r, sink)
    }
}

/// Not a test by itself (it returns at once unless [`HOLDER_HOME`] is set): the test below runs it as other processes, each one
/// install contending for the lock. It writes `<home>/done-<pid>` with its outcome (`ok` or the error code) on the first line and the
/// error's detail on the second, so a failure says which error it was; one that gets the lock holds it until its stdin closes or it
/// is killed.
#[test]
fn lock_holder_child() {
    let Some(home) = std::env::var_os(HOLDER_HOME).map(PathBuf::from) else {
        return;
    };
    let l = list(&Opts::new("1.2.3", GOOD));
    let p = plan(&l, "fake-agent", PLATFORM, &home).unwrap();
    let net = Net::serving(GOOD);
    let hold = HoldUntilStdinCloses {
        net: Net::serving(GOOD),
        home: home.clone(),
    };
    let out = match install(
        &p,
        &approval(&p),
        &home,
        &Tools::new(&net, &hold, &Sig(true)),
    ) {
        Ok(_) => "ok\n".to_string(),
        Err(e) => format!("{}\n{}", e.code.code(), e.detail),
    };
    // Written aside and renamed, so `done-<pid>` is never seen half written.
    let aside = home.join(format!("out-{}", std::process::id()));
    std::fs::write(&aside, out).unwrap();
    std::fs::rename(&aside, home.join(format!("done-{}", std::process::id()))).unwrap();
}

/// The processes a test started; killed and reaped when it ends, also when it fails.
struct Children(Vec<std::process::Child>);
impl Drop for Children {
    fn drop(&mut self) {
        for c in &mut self.0 {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// The outcome a [`lock_holder_child`] wrote to `done` (its first line), and the whole file for a failure message.
fn outcome(done: &Path) -> (String, String) {
    let all = std::fs::read_to_string(done).unwrap();
    (all.lines().next().unwrap_or("").to_string(), all)
}

/// The pids in the names of `<dir>/<prefix><pid>` files.
fn pids(dir: &Path, prefix: &str) -> Vec<u32> {
    std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .filter_map(|e| {
            e.file_name()
                .to_string_lossy()
                .strip_prefix(prefix)
                .and_then(|p| p.parse().ok())
        })
        .collect()
}

/// Waits until `done`, at most a minute.
fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let end = std::time::Instant::now() + Duration::from_secs(60);
    while !done() {
        assert!(std::time::Instant::now() < end, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Audit F100, with processes (review: the threads of the earlier test mostly took turns, and its FIFO never reached the system
/// lock). Two installs started together in two processes contend for the lock: exactly one holds it and the other is
/// `in_progress`. Then only the system lock is left to refuse (Unix: the owner text is wiped; Windows: nobody can read it): this
/// process and a third process are refused, and on Windows nobody can read, rename or remove the held file (an older version judges
/// a lock it cannot read by its age, then renames it). The holder is then killed while holding: the system frees the lock.
/// Runs on every system: the holders wait inside their download, before the fake agent would run.
#[test]
fn installs_in_two_processes_never_hold_the_lock_at_once_and_a_killed_holder_frees_it() {
    let home = Home::new();
    std::fs::create_dir_all(home.agents()).unwrap();
    let spawn = || {
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "lock_holder_child",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(HOLDER_HOME, &home.0)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap()
    };
    let mut kids = Children(vec![spawn(), spawn()]);
    wait_until("both installs got the lock or were refused", || {
        pids(&home.0, "held-").len() + pids(&home.0, "done-").len() >= 2
    });
    let (held, done) = (pids(&home.0, "held-"), pids(&home.0, "done-"));
    assert_eq!(held.len(), 1, "two processes held the lock at once");
    assert_eq!(done.len(), 1);
    assert_ne!(held[0], done[0]);
    let (code, all) = outcome(&home.0.join(format!("done-{}", done[0])));
    assert_eq!(
        code, "agent.install.in_progress",
        "the refused install: {all:?}"
    );
    let holder = held[0];
    let lock = home.agents().join(".lock");
    if agora_platform::LOCK_FILES_MANDATORY {
        assert!(std::fs::read(&lock).is_err(), "a held lock was read");
        assert!(
            std::fs::rename(&lock, home.agents().join(".lock.moved")).is_err(),
            "a held lock was renamed"
        );
        assert!(
            std::fs::remove_file(&lock).is_err(),
            "a held lock was removed"
        );
    } else {
        assert_eq!(std::fs::read_to_string(&lock).unwrap(), owner_text(holder));
        // The lock is advisory here, so the text can be wiped: from now on only the system lock refuses.
        std::fs::write(&lock, "").unwrap();
    }
    // This process is refused while the other holds it, and so is a third process.
    assert_eq!(
        remember_list_version(&home.0, 7).unwrap_err().code,
        Code::InProgress
    );
    kids.0.push(spawn());
    let third = kids.0.last().unwrap().id();
    let third_done = home.0.join(format!("done-{third}"));
    wait_until("the third install was refused", || {
        third_done.exists() || pids(&home.0, "held-").len() > 1
    });
    assert_eq!(
        pids(&home.0, "held-"),
        [holder],
        "two processes held the lock at once"
    );
    let (code, all) = outcome(&third_done);
    assert_eq!(
        code, "agent.install.in_progress",
        "the third install: {all:?}"
    );
    // The holder ends without letting go.
    let c = kids.0.iter_mut().find(|c| c.id() == holder).unwrap();
    c.kill().unwrap();
    c.wait().unwrap();
    // Windows may take a moment to drop the lock of a process that was ended. The ended holder's handle stays open in `kids`, which
    // keeps it queryable there: it must still read as gone (Codex's review of #934).
    let end = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        match remember_list_version(&home.0, 7) {
            Ok(()) => break,
            Err(e) => assert!(
                std::time::Instant::now() < end,
                "timed out: the lock is not free once its holder was killed; last: {e:?}"
            ),
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(last_seen_version(&home.0).unwrap(), 7);
    assert!(released(&home));
}

/// Review of audit F100: an older version that ended between putting a lock back (`hard_link`) and removing the name it had moved it
/// to left `.lock` with a second name, which the open refuses; that left every later install failing. The `.lock.stale-*` name goes
/// and the lock is taken. Runs on every system.
#[test]
fn a_lock_an_older_version_left_with_a_second_name_is_taken() {
    let home = Home::new();
    std::fs::create_dir_all(home.agents()).unwrap();
    let aside = home.agents().join(".lock.stale-424242-0");
    std::fs::write(&aside, "").unwrap();
    std::fs::hard_link(&aside, home.agents().join(".lock")).unwrap();
    remember_list_version(&home.0, 4).unwrap();
    assert_eq!(last_seen_version(&home.0).unwrap(), 4);
    assert!(!aside.exists());
    assert!(released(&home));
    // A second name of any other kind is still refused.
    let other = home.agents().join("other");
    std::fs::hard_link(home.agents().join(".lock"), &other).unwrap();
    assert_eq!(
        remember_list_version(&home.0, 5).unwrap_err().code,
        Code::IoFailed
    );
    assert!(other.exists());
}

#[test]
fn a_failure_after_the_launcher_moved_puts_the_old_launcher_back() {
    if skip() {
        return;
    }
    let home = Home::new();
    run(&home, &Opts::new("1.0.0", GOOD), GOOD).unwrap();
    let before = std::fs::read_link(launcher(&home)).unwrap();
    let l = list(&Opts::new("2.0.0", GOOD));
    let p = plan(&l, "fake-agent", PLATFORM, &home.0).unwrap();
    let net = Net::serving(GOOD);
    let mut t = tools(&net, &Sig(true));
    t.fault = Some(Fault::BeforeCurrent);
    let e = install(&p, &approval(&p), &home.0, &t).unwrap_err();
    assert_eq!(e.detail, "injected");
    assert_eq!(std::fs::read_link(launcher(&home)).unwrap(), before);
    assert!(launcher(&home).exists(), "the launcher must still resolve");
    nothing_live(&home, "2.0.0");
    // A first install that fails after the move leaves no launcher at all.
    let home = Home::new();
    let l = list(&Opts::new("1.0.0", GOOD));
    let p = plan(&l, "fake-agent", PLATFORM, &home.0).unwrap();
    let net = Net::serving(GOOD);
    let mut t = tools(&net, &Sig(true));
    t.fault = Some(Fault::BeforeCurrent);
    install(&p, &approval(&p), &home.0, &t).unwrap_err();
    assert!(std::fs::symlink_metadata(launcher(&home)).is_err());
    nothing_live(&home, "1.0.0");
}

#[test]
fn a_failed_write_of_current_is_rolled_back_for_real() {
    if skip() {
        return;
    }
    let home = Home::new();
    run(&home, &Opts::new("1.0.0", GOOD), GOOD).unwrap();
    let before = std::fs::read_link(launcher(&home)).unwrap();
    let l = list(&Opts::new("2.0.0", GOOD));
    let p = plan(&l, "fake-agent", PLATFORM, &home.0).unwrap();
    // A folder where `current` must go makes the real write fail.
    let current = home.agents().join("fake-agent").join("current");
    std::fs::remove_file(&current).unwrap();
    std::fs::create_dir(&current).unwrap();
    let net = Net::serving(GOOD);
    install(&p, &approval(&p), &home.0, &tools(&net, &Sig(true))).unwrap_err();
    assert_eq!(std::fs::read_link(launcher(&home)).unwrap(), before);
    assert!(home
        .agents()
        .join("fake-agent")
        .join("1.0.0")
        .join("fake-agent")
        .is_file());
    assert!(!home.agents().join("fake-agent").join("2.0.0").exists());
}

/// Codex review of #1027: an update that renames the program and then fails puts the `exe` record (which agent owns which launcher
/// name) back with `current`, so the old launcher keeps its owner and the new name is not claimed by a version that was rolled back.
#[test]
fn a_failed_update_that_renames_the_program_puts_the_exe_record_back() {
    if skip() {
        return;
    }
    let home = Home::new();
    run(&home, &Opts::new("1.0.0", GOOD), GOOD).unwrap();
    let before = std::fs::read_link(launcher(&home)).unwrap();
    let exe_file = home.agents().join("fake-agent").join("exe");
    let old_exe = std::fs::read_to_string(&exe_file).unwrap();
    let mut o = Opts::new("2.0.0", GOOD);
    o.provides = "fake-agent-two";
    let l = list(&o);
    let p = plan(&l, "fake-agent", PLATFORM, &home.0).unwrap();
    assert_ne!(p.exe, old_exe.trim());
    // A folder where `current` must go makes the real write fail, after `exe` was written.
    let current = home.agents().join("fake-agent").join("current");
    std::fs::remove_file(&current).unwrap();
    std::fs::create_dir(&current).unwrap();
    let net = Net::serving(GOOD);
    let e = install(&p, &approval(&p), &home.0, &tools(&net, &Sig(true))).unwrap_err();
    assert_eq!(e.code, Code::IoFailed);
    assert_eq!(std::fs::read_link(launcher(&home)).unwrap(), before);
    assert_eq!(std::fs::read_to_string(&exe_file).unwrap(), old_exe);
    assert!(
        !home.agents().join("bin").join(&p.exe).exists(),
        "the new launcher name is not left behind"
    );
    assert!(!home.agents().join("fake-agent").join("2.0.0").exists());
}

/// A rollback that cannot put the records back says so: the error is not the plain write failure, so nobody reads it as a clean
/// rollback (Codex review of #1027).
#[test]
fn a_rollback_that_cannot_restore_the_records_says_so() {
    if skip() {
        return;
    }
    let home = Home::new();
    run(&home, &Opts::new("1.0.0", GOOD), GOOD).unwrap();
    let l = list(&Opts::new("2.0.0", GOOD));
    let p = plan(&l, "fake-agent", PLATFORM, &home.0).unwrap();
    let net = Net::serving(GOOD);
    let mut t = tools(&net, &Sig(true));
    t.fault = Some(Fault::RecordsUnrestorable);
    let e = install(&p, &approval(&p), &home.0, &t).unwrap_err();
    assert_eq!(
        (e.code, e.detail.as_str()),
        (Code::IoFailed, "rollback_incomplete")
    );
}

#[test]
fn a_failed_write_of_exe_keeps_current_on_the_previous_version() {
    if skip() {
        return;
    }
    let home = Home::new();
    run(&home, &Opts::new("1.0.0", GOOD), GOOD).unwrap();
    let before = std::fs::read_link(launcher(&home)).unwrap();
    let l = list(&Opts::new("2.0.0", GOOD));
    let p = plan(&l, "fake-agent", PLATFORM, &home.0).unwrap();
    let exe_meta = home.agents().join("fake-agent").join("exe");
    std::fs::remove_file(&exe_meta).unwrap();
    std::fs::create_dir(&exe_meta).unwrap();
    let net = Net::serving(GOOD);
    install(&p, &approval(&p), &home.0, &tools(&net, &Sig(true))).unwrap_err();
    assert_eq!(std::fs::read_link(launcher(&home)).unwrap(), before);
    assert_eq!(
        std::fs::read_to_string(home.agents().join("fake-agent").join("current")).unwrap(),
        "1.0.0"
    );
    assert!(!home.agents().join("fake-agent").join("2.0.0").exists());
}

#[test]
fn dot_and_dotdot_in_current_are_rejected_by_installed_version() {
    let home = Home::new();
    let dir = home.agents().join("fake-agent");
    std::fs::create_dir_all(&dir).unwrap();
    for bad in [".", "..", ""] {
        std::fs::write(dir.join("current"), bad).unwrap();
        assert_eq!(
            agora_adapters::agent_install::installed_version(&home.0, "fake-agent"),
            None,
            "must reject {bad:?}"
        );
    }
    std::fs::write(dir.join("current"), "1.2.3").unwrap();
    assert_eq!(
        agora_adapters::agent_install::installed_version(&home.0, "fake-agent").as_deref(),
        Some("1.2.3")
    );
}

#[test]
fn a_folder_the_launcher_uses_is_never_removed() {
    if skip() {
        return;
    }
    let home = Home::new();
    run(&home, &Opts::new("1.0.0", GOOD), GOOD).unwrap();
    // `current` is lost, so the plan sees a fresh install of the very version the launcher resolves into.
    std::fs::remove_file(home.agents().join("fake-agent").join("current")).unwrap();
    let e = run(&home, &Opts::new("1.0.0", GOOD), GOOD).unwrap_err();
    assert_eq!(e.detail, "in_use");
    assert!(launcher(&home).exists());
    // And a failed verify of the same version does not remove it either.
    let bad = "#!/bin/sh\necho nope\n";
    let e = run(&home, &Opts::new("1.0.0", bad), bad).unwrap_err();
    assert_eq!(e.detail, "in_use");
    assert!(launcher(&home).exists());
}

#[test]
fn a_launcher_name_belongs_to_one_agent_and_bin_is_not_an_agent() {
    if skip() {
        return;
    }
    let home = Home::new();
    run(&home, &Opts::new("1.0.0", GOOD), GOOD).unwrap();
    // Another agent folder claims the same program name.
    let l = list(&Opts::new("1.0.0", GOOD));
    let mut p = plan(&l, "fake-agent", PLATFORM, &home.0).unwrap();
    let a = approval(&p);
    p.name = "other-agent".into();
    p.change = Change::Fresh;
    let net = Net::serving(GOOD);
    let e = install(&p, &a, &home.0, &tools(&net, &Sig(true))).unwrap_err();
    assert_eq!(
        (e.code, e.detail.as_str()),
        (Code::Conflict, "executable_owned")
    );
    p.name = "bin".into();
    let e = install(&p, &a, &home.0, &tools(&net, &Sig(true))).unwrap_err();
    assert_eq!(
        (e.code, e.detail.as_str()),
        (Code::Conflict, "reserved_name")
    );
    assert_eq!(e.code.code(), "agent.install.conflict");
}

#[test]
fn a_linked_agents_folder_is_refused() {
    let home = Home::new();
    let real = home.0.join("elsewhere");
    std::fs::create_dir_all(&real).unwrap();
    if agora_platform::link_dir(&real, &home.agents()).is_err() {
        return;
    }
    let l = list(&Opts::new("1.2.3", GOOD));
    let p = plan(&l, "fake-agent", PLATFORM, &home.0).unwrap();
    let net = Net::serving(GOOD);
    let e = install(&p, &approval(&p), &home.0, &tools(&net, &Sig(true))).unwrap_err();
    assert_eq!(
        (e.code, e.detail.as_str()),
        (Code::IoFailed, "not_confined")
    );
    assert!(std::fs::read_dir(&real).unwrap().next().is_none());
    assert_eq!(
        remember_list_version(&home.0, 9).unwrap_err().detail,
        "not_confined"
    );
}

#[test]
fn the_list_version_is_remembered_under_the_lock() {
    let home = Home::new();
    std::fs::create_dir_all(home.agents()).unwrap();
    std::fs::write(home.agents().join(".lock"), std::process::id().to_string()).unwrap();
    assert_eq!(
        remember_list_version(&home.0, 4).unwrap_err().code,
        Code::InProgress
    );
    assert_eq!(last_seen_version(&home.0).unwrap(), 0);
}

// ---- finding what was installed (ADR-0087 item 3) ----------------------------------------------------------------------------

/// The layout the installer leaves, made by hand (no program runs): the version folder, `current`, `exe` and the launcher.
fn layout(home: &Path, name: &str, version: &str, exe: &str) -> PathBuf {
    let agents = home.join("agents");
    let vdir = agents.join(name).join(version);
    std::fs::create_dir_all(&vdir).unwrap();
    std::fs::create_dir_all(agents.join("bin")).unwrap();
    std::fs::write(vdir.join(exe), "x").unwrap();
    // As the installer leaves them, whatever this machine's umask: only the owner can write.
    for d in [&agents, &agents.join("bin"), &agents.join(name), &vdir] {
        agora_platform::restrict_to_owner(d).unwrap();
    }
    agora_platform::set_executable(&vdir.join(exe), true).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(vdir.join(exe), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    std::fs::write(agents.join(name).join("current"), version).unwrap();
    std::fs::write(agents.join(name).join("exe"), exe).unwrap();
    let launcher = agents.join("bin").join(exe);
    agora_platform::install_launcher(
        &vdir.join(exe),
        &Path::new("..").join(name).join(version).join(exe),
        &launcher,
    )
    .unwrap();
    launcher
}

#[test]
fn the_program_the_installer_made_is_found_by_its_plain_name() {
    let home = Home::new();
    layout(&home.0, "fake", "1.2.3", "fake-agent");
    let found = agora_adapters::agent_install::installed_program(&home.0, "fake-agent").unwrap();
    assert_eq!(
        (found.name.as_str(), found.version.as_str()),
        ("fake", "1.2.3")
    );
    assert!(found.path.is_file());
    assert!(found.path.ends_with("fake-agent"));
    // Nothing else: a name no agent owns, and nothing at all in an empty home.
    assert!(agora_adapters::agent_install::installed_program(&home.0, "other").is_none());
    let empty = Home::new();
    assert!(agora_adapters::agent_install::installed_program(&empty.0, "fake-agent").is_none());
}

#[test]
fn only_a_plain_file_name_is_looked_up_in_the_install_folder() {
    let home = Home::new();
    layout(&home.0, "fake", "1.2.3", "fake-agent");
    let agents = home.agents();
    let sep = std::path::MAIN_SEPARATOR;
    for bad in [
        String::new(),
        ".".into(),
        "..".into(),
        format!("bin{sep}fake-agent"),
        format!("..{sep}bin{sep}fake-agent"),
        format!("{sep}fake-agent"),
        agents.join("bin").join("fake-agent").display().to_string(),
        "fake-agent/".into(),
        "./fake-agent".into(),
    ] {
        assert!(
            agora_adapters::agent_install::installed_program(&home.0, &bad).is_none(),
            "{bad:?}"
        );
    }
}

#[test]
fn a_launcher_nobody_recorded_is_not_used() {
    let home = Home::new();
    layout(&home.0, "fake", "1.2.3", "fake-agent");
    // A file someone dropped into `bin` that no agent folder owns.
    std::fs::write(home.agents().join("bin").join("planted"), "x").unwrap();
    assert!(agora_adapters::agent_install::installed_program(&home.0, "planted").is_none());
    // The record gone: the launcher alone is not enough.
    std::fs::remove_file(home.agents().join("fake").join("exe")).unwrap();
    assert!(agora_adapters::agent_install::installed_program(&home.0, "fake-agent").is_none());
}

#[cfg(unix)]
#[test]
fn a_launcher_that_leaves_the_install_folder_or_the_current_version_is_not_used() {
    let home = Home::new();
    let launcher = layout(&home.0, "fake", "1.2.3", "fake-agent");
    let outside = Home::new();
    std::fs::write(outside.0.join("fake-agent"), "x").unwrap();
    // Points outside `agents`.
    std::fs::remove_file(&launcher).unwrap();
    std::os::unix::fs::symlink(outside.0.join("fake-agent"), &launcher).unwrap();
    assert!(agora_adapters::agent_install::installed_program(&home.0, "fake-agent").is_none());
    // Points at another agent's file inside `agents`.
    layout(&home.0, "evil", "9", "evil-agent");
    std::fs::remove_file(&launcher).unwrap();
    std::os::unix::fs::symlink("../evil/9/evil-agent", &launcher).unwrap();
    assert!(agora_adapters::agent_install::installed_program(&home.0, "fake-agent").is_none());
    // Points at a version that `current` does not name.
    let old = home.agents().join("fake").join("1.0.0");
    std::fs::create_dir_all(&old).unwrap();
    std::fs::write(old.join("fake-agent"), "x").unwrap();
    std::fs::remove_file(&launcher).unwrap();
    std::os::unix::fs::symlink("../fake/1.0.0/fake-agent", &launcher).unwrap();
    assert!(agora_adapters::agent_install::installed_program(&home.0, "fake-agent").is_none());
}

#[cfg(unix)]
#[test]
fn links_for_the_agents_or_bin_folder_and_files_others_can_write_are_not_used() {
    use std::os::unix::fs::PermissionsExt;
    let home = Home::new();
    layout(&home.0, "fake", "1.2.3", "fake-agent");
    let exe = home.agents().join("fake").join("1.2.3").join("fake-agent");
    // Writable by the group or everyone.
    for mode in [0o775, 0o757] {
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(mode)).unwrap();
        assert!(agora_adapters::agent_install::installed_program(&home.0, "fake-agent").is_none());
    }
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(agora_adapters::agent_install::installed_program(&home.0, "fake-agent").is_some());
    // `bin` and `agents` as links to a real folder.
    let bin = home.agents().join("bin");
    let moved = home.0.join("elsewhere");
    std::fs::rename(&bin, &moved).unwrap();
    std::os::unix::fs::symlink(&moved, &bin).unwrap();
    assert!(agora_adapters::agent_install::installed_program(&home.0, "fake-agent").is_none());
    let real = Home::new();
    std::fs::rename(home.agents(), real.0.join("agents")).unwrap();
    std::os::unix::fs::symlink(real.0.join("agents"), home.agents()).unwrap();
    assert!(agora_adapters::agent_install::installed_program(&home.0, "fake-agent").is_none());
}

#[test]
fn a_manifest_name_maps_to_the_exe_the_installer_wrote() {
    use agora_adapters::agent_install::launcher_names;
    assert_eq!(launcher_names("claude", true), ["claude", "claude.exe"]);
    assert_eq!(launcher_names("claude", false), ["claude"]);
    assert_eq!(launcher_names("claude.exe", true), ["claude.exe"]);
    assert_eq!(launcher_names("tool.cmd", true), ["tool.cmd"]);
}

#[cfg(unix)]
#[test]
fn a_record_that_fails_a_check_is_unsafe_not_absent() {
    use agora_adapters::agent_install::{lookup_installed, Lookup};
    use std::os::unix::fs::PermissionsExt;
    let home = Home::new();
    layout(&home.0, "fake", "1.2.3", "fake-agent");
    assert!(matches!(
        lookup_installed(&home.0, "fake-agent"),
        Lookup::Installed(_)
    ));
    assert_eq!(lookup_installed(&home.0, "nobody"), Lookup::NotInstalled);
    // Each folder on the way, group-writable, makes it unsafe.
    for dir in [
        home.agents(),
        home.agents().join("bin"),
        home.agents().join("fake").join("1.2.3"),
    ] {
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o775)).unwrap();
        assert_eq!(
            lookup_installed(&home.0, "fake-agent"),
            Lookup::Unsafe,
            "{dir:?}"
        );
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(matches!(
            lookup_installed(&home.0, "fake-agent"),
            Lookup::Installed(_)
        ));
    }
}
