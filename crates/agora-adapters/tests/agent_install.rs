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
        }
    }
}

/// A list signed by a fake key made from a public seed, opened by the real `verify_list`.
fn list(o: &Opts) -> AgentList {
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
        "version": 3, "expires_ms": 2_000_000_000_000_u64,
        "publishers": [{"id": "fake-official", "key": hex(publisher.public_key().as_ref()), "revoked": o.revoked}],
        "agents": [{
            "name": "fake-agent", "display": "Fake Agent",
            "changelog": "https://agents.example.com/changes", "homepage": "https://agents.example.com",
            "publisher": "fake-official", "version_policy": "track", "version": o.version,
            "provides": ["fake-agent"],
            "verify": {"args": ["--version"], "regex": "fake-agent [0-9]+"},
            "installer": {"kind": "binary-download", "platforms": {o.platform: dl}},
        }],
    })
    .to_string();
    let envelope =
        json!({"payload": payload, "sig": hex(root.sign(payload.as_bytes()).as_ref())}).to_string();
    verify_list(&envelope, &hex(root.public_key().as_ref()), 1, 0).unwrap()
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
    assert!(!home.agents().join(".lock").exists());
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
    assert!(!lock.exists());
    // The same, in the current format (pid, start, boot): an identity that cannot be read falls back to the pid, which is gone.
    std::fs::write(&lock, format!("{dead} 12345 some-boot")).unwrap();
    let l1 = list(&Opts::new("1.2.4", GOOD));
    let p1 = plan(&l1, "fake-agent", PLATFORM, &home.0).unwrap();
    install(&p1, &approval(&p1), &home.0, &tools(&net, &Sig(true))).unwrap();
    assert!(!lock.exists());
    // Garbage in the lock is not ours to break.
    std::fs::write(&lock, "not a pid").unwrap();
    let l2 = list(&Opts::new("1.2.5", GOOD));
    let p2 = plan(&l2, "fake-agent", PLATFORM, &home.0).unwrap();
    assert_eq!(
        install(&p2, &approval(&p2), &home.0, &tools(&net, &Sig(true)))
            .unwrap_err()
            .code,
        Code::InProgress
    );
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
    run(&home, &Opts::new("1.2.3", GOOD), GOOD).unwrap();
    assert!(!left.exists());
    assert!(!home.agents().join("bin").join("fake-agent.tmp-9").exists());
}

#[test]
fn the_list_version_seen_only_goes_up() {
    let home = Home::new();
    assert_eq!(last_seen_version(&home.0), 0);
    remember_list_version(&home.0, 5).unwrap();
    assert_eq!(last_seen_version(&home.0), 5);
    remember_list_version(&home.0, 3).unwrap();
    assert_eq!(last_seen_version(&home.0), 5);
    remember_list_version(&home.0, 6).unwrap();
    assert_eq!(last_seen_version(&home.0), 6);
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
fn an_empty_or_garbage_lock_blocks_only_while_it_is_young() {
    if skip() {
        return;
    }
    for text in ["", "garbage", "0x12"] {
        let home = Home::new();
        assert_eq!(
            with_lock(&home, text, 0).unwrap_err().code,
            Code::InProgress,
            "{text:?}"
        );
        let home = Home::new();
        with_lock(&home, text, 120).unwrap();
        assert!(!home.agents().join(".lock").exists());
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

/// A fetcher that, while the install holds the lock, replaces the lock with somebody else's.
struct Thief<'a>(&'a Net, PathBuf);
impl Fetcher for Thief<'_> {
    fn fetch(&self, r: &FetchRequest<'_>, sink: &mut dyn Write) -> Result<(), InstallError> {
        std::fs::write(&self.1, format!("{} 1 someone-else", std::process::id())).unwrap();
        self.0.fetch(r, sink)
    }
}

#[test]
fn the_lock_is_removed_only_while_it_is_still_ours() {
    if skip() {
        return;
    }
    let home = Home::new();
    let l = list(&Opts::new("1.2.3", GOOD));
    let p = plan(&l, "fake-agent", PLATFORM, &home.0).unwrap();
    let net = Net::serving(GOOD);
    let thief = Thief(&net, home.agents().join(".lock"));
    let sig = Sig(true);
    let t = Tools::new(&net, &thief, &sig);
    install(&p, &approval(&p), &home.0, &t).unwrap();
    let left = std::fs::read_to_string(home.agents().join(".lock")).unwrap();
    assert!(
        left.ends_with("someone-else"),
        "another process's lock was removed"
    );
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
    assert!(!home.agents().join(".lock").exists());
    assert_eq!(
        agora_adapters::agent_install::installed_version(&home.0, "fake-agent").as_deref(),
        Some("1.2.3")
    );
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
    assert_eq!(last_seen_version(&home.0), 0);
}
