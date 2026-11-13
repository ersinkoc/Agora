//! ADR-0061 end to end, with a real hub and node in one process and the `agora` binary for what the owner types on the machine:
//! the hub's device list never becomes trust without the owner's yes; `--yes` covers only the fingerprints given; the old
//! `agora node trust ID KEY` works as before; an endorsement the hub carries is verified on the node (a forged one and an expired one
//! are refused, a real one is trusted); a revocation stops the device at once; `agora node untrust` does the same on the machine.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use agora_hub::ca::{new_device_request, Ca};
use agora_hub::enroll::{Enrollment, Lockout};
use agora_hub::hub::Hub;
use agora_hub::server::{self, ClientAuth, ServerState};
use agora_hub::store::SqliteStore;
use agora_hub::tls;
use agora_node::device_trust::{endorse_message, fingerprint, revoke_message};
use agora_node::uplink_client::{spawn, HubLink};
use agora_protocol::frame::{kinds, STREAM_COMMANDS, STREAM_EVENTS};
use agora_protocol::handshake::Support;
use agora_protocol::v1::{self, command_request as cr, command_response};
use agora_protocol::Frame;
use prost::Message;
use ring::rand::SystemRandom;
use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};

const AGORA: &str = env!("CARGO_BIN_EXE_agora");

struct Tokens;

impl ClientAuth for Tokens {
    fn check(&self, t: &str) -> bool {
        t == "client-token"
    }
}

struct Key(EcdsaKeyPair);

impl Key {
    fn new() -> Self {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        Key(
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng)
                .unwrap(),
        )
    }
    fn public(&self) -> Vec<u8> {
        self.0.public_key().as_ref().to_vec()
    }
    fn hex(&self) -> String {
        self.public().iter().map(|b| format!("{b:02x}")).collect()
    }
    /// This device registering itself, with its proof of possession.
    fn registration(&self, id: &str, name: &str) -> v1::DeviceRegister {
        let at = server::now_ms();
        v1::DeviceRegister {
            device_id: id.into(),
            name: name.into(),
            public_key: self.public(),
            signed_at_ms: at,
            signature: self.sign(&agora_node::device_trust::register_message(
                id,
                name,
                &self.public(),
                at,
            )),
        }
    }
    fn sign(&self, m: &[u8]) -> Vec<u8> {
        self.0
            .sign(&SystemRandom::new(), m)
            .unwrap()
            .as_ref()
            .to_vec()
    }
}

fn wait(what: &str, mut ok: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(20);
    while !ok() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn agora(home: &Path, args: &[&str]) -> (bool, String, String) {
    let out = Command::new(AGORA)
        .args(args)
        .env("AGORA_HOME", home)
        .env("AGORA_LANG", "en")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn trusted(file: &Path) -> Vec<String> {
    let mut ids: Vec<String> = agora_node::device_trust::read_entries(file)
        .0
        .into_iter()
        .map(|e| e.id)
        .collect();
    ids.sort();
    ids
}

#[test]
fn the_hub_offers_devices_the_owner_decides_and_endorsements_and_revocations_are_verified_on_the_machine(
) {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    // ---- a hub, and this machine enrolled with it ----
    let ca = Ca::generate("hub ca").unwrap();
    let (server_cert, server_key) = ca.issue_server(&["localhost".to_string()]).unwrap();
    let acceptor = tls::acceptor_with(&server_cert, &server_key, ca.cert_pem(), true).unwrap();
    let store = SqliteStore::open_in_memory().unwrap();
    let mut enrollment = Enrollment::new(&store);
    let code = enrollment.create_code(server::now_ms()).unwrap();
    let (key_pem, csr) = new_device_request().unwrap();
    let issued = enrollment
        .redeem(&ca, &code, "laptop", &csr, server::now_ms())
        .unwrap();
    let ca_pem = ca.cert_pem().to_string();
    let state = Arc::new(ServerState {
        hub: Arc::new(Hub::open(store).unwrap()),
        support: Support {
            generations: vec![1],
            capabilities: vec!["device.trust".into(), "device.list".into()],
            required: vec![],
        },
        ca: Some(ca),
        lockout: Arc::new(Lockout::new()),
        auth: Box::new(Tokens),
        assets: None,
        sessions: None,
    });
    let rt = tokio::runtime::Runtime::new().unwrap();
    let listener = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    rt.spawn(server::run(listener, Arc::clone(&state), Some(acceptor)));
    let url = format!("wss://localhost:{}", addr.port());

    let home: PathBuf = std::env::temp_dir().join(format!("agora-devtrust-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let link_dir = home.join("link");
    std::fs::create_dir_all(&link_dir).unwrap();
    for (n, v) in [
        ("url", url.as_str()),
        ("node-id", "laptop"),
        ("ca.pem", ca_pem.as_str()),
        ("cert.pem", issued.cert_pem.as_str()),
        ("key.pem", key_pem.as_str()),
    ] {
        std::fs::write(link_dir.join(n), v).unwrap();
    }
    let data = home.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let file = data.join("trusted-devices");
    let data_arg = data.to_str().unwrap();

    // ---- the owner's devices, and one the hub made up ----
    let (old, new, third, fake) = (Key::new(), Key::new(), Key::new(), Key::new());
    let devices = agora_hub::devices::Devices::new(state.hub.store());
    for (id, k) in [
        ("web-old", &old),
        ("web-new", &new),
        ("web-third", &third),
        ("web-fake", &fake),
    ] {
        devices
            .register(
                "owner",
                &k.registration(id, &format!("Browser {id}")),
                server::now_ms(),
            )
            .unwrap();
    }

    // The old way still works, unchanged: the owner trusts a device by its key on the machine.
    let (ok, out, _) = agora(
        &home,
        &["node", "trust", "web-old", &old.hex(), "--data", data_arg],
    );
    assert!(ok, "{out}");
    assert_eq!(trusted(&file), ["web-old"]);

    // The hub's list is shown with fingerprints, but on a terminal that cannot answer nothing is trusted.
    let (ok, out, err) = agora(&home, &["node", "trust", "--from-hub", "--data", data_arg]);
    assert!(ok, "{out}{err}");
    assert!(out.contains(&fingerprint(&fake.public())), "{out}");
    assert!(out.contains(&fingerprint(&new.public())), "{out}");
    assert!(
        !out.contains(&fingerprint(&old.public())),
        "already trusted: not offered again\n{out}"
    );
    assert!(out.contains("Nothing was trusted"), "{out}");
    assert_eq!(
        trusted(&file),
        ["web-old"],
        "a listed device is not a trusted one"
    );

    // `--yes` covers exactly the fingerprints given: the fake device stays out.
    let fp = fingerprint(&new.public()).to_uppercase().replace('-', " ");
    let (ok, out, err) = agora(
        &home,
        &[
            "node",
            "trust",
            "--from-hub",
            "--yes",
            "--fingerprint",
            &fp,
            "--data",
            data_arg,
        ],
    );
    assert!(ok, "{out}{err}");
    assert_eq!(trusted(&file), ["web-new", "web-old"]);
    // A fingerprint the hub does not list trusts nothing and says so.
    let (ok, _, err) = agora(
        &home,
        &[
            "node",
            "trust",
            "--from-hub",
            "--yes",
            "--fingerprint",
            "0000-0000-0000-0000",
            "--data",
            data_arg,
        ],
    );
    assert!(!ok && err.contains("0000-0000-0000-0000"), "{err}");
    assert_eq!(trusted(&file), ["web-new", "web-old"]);
    // `--yes` alone covers nothing and is refused.
    let (ok, _, _) = agora(
        &home,
        &["node", "trust", "--from-hub", "--yes", "--data", data_arg],
    );
    assert!(!ok);

    // Someone else's device on the same hub: never trusted by the owner's fingerprints alone, only with --include-others.
    let eda = Key::new();
    devices
        .register(
            "eda",
            &eda.registration("web-eda", "Eda's laptop"),
            server::now_ms(),
        )
        .unwrap();
    let fp_eda = fingerprint(&eda.public());
    let (ok, out, err) = agora(
        &home,
        &[
            "node",
            "trust",
            "--from-hub",
            "--yes",
            "--fingerprint",
            &fp_eda,
            "--data",
            data_arg,
        ],
    );
    assert!(!ok && err.contains(&fp_eda), "{out}{err}");
    assert!(out.contains("other people"), "listed apart: {out}");
    assert!(!trusted(&file).contains(&"web-eda".to_string()));
    let (ok, out, err) = agora(
        &home,
        &[
            "node",
            "trust",
            "--from-hub",
            "--yes",
            "--include-others",
            "--fingerprint",
            &fp_eda,
            "--data",
            data_arg,
        ],
    );
    assert!(ok, "{out}{err}");
    assert!(trusted(&file).contains(&"web-eda".to_string()));
    agora(&home, &["node", "untrust", "web-eda", "--data", data_arg]);

    // An id the machine trusts with another key: the hub's key replaces it only with --replace.
    let (mine_before, swapped) = (Key::new(), Key::new());
    agora(
        &home,
        &[
            "node",
            "trust",
            "web-swap",
            &mine_before.hex(),
            "--data",
            data_arg,
        ],
    );
    devices
        .register(
            "owner",
            &swapped.registration("web-swap", "Swapped"),
            server::now_ms(),
        )
        .unwrap();
    let fp_swap = fingerprint(&swapped.public());
    let (ok, out, err) = agora(
        &home,
        &[
            "node",
            "trust",
            "--from-hub",
            "--yes",
            "--fingerprint",
            &fp_swap,
            "--data",
            data_arg,
        ],
    );
    assert!(!ok && out.contains("replaces"), "{out}{err}");
    let kept = agora_node::device_trust::read_entries(&file)
        .0
        .into_iter()
        .find(|e| e.id == "web-swap")
        .unwrap();
    assert_eq!(kept.key, mine_before.public(), "not replaced silently");
    let (ok, out, err) = agora(
        &home,
        &[
            "node",
            "trust",
            "--from-hub",
            "--yes",
            "--replace",
            "--fingerprint",
            &fp_swap,
            "--data",
            data_arg,
        ],
    );
    assert!(ok, "{out}{err}");
    let now_key = agora_node::device_trust::read_entries(&file)
        .0
        .into_iter()
        .find(|e| e.id == "web-swap")
        .unwrap();
    assert_eq!(now_key.key, swapped.public());
    agora(&home, &["node", "untrust", "web-swap", "--data", data_arg]);

    // The owner removes one on the machine.
    let (ok, out, _) = agora(&home, &["node", "untrust", "web-new", "--data", data_arg]);
    assert!(ok && out.contains("web-new"), "{out}");
    assert_eq!(trusted(&file), ["web-old"]);

    // ---- the node runs and links to the hub ----
    let cfg = agora_node::daemon::NodeConfig {
        node_id: "laptop".into(),
        socket: home.join("c.sock"),
        data_dir: data.clone(),
        worktree_root: home.join("wt"),
        manifests_dir: home.join("manifests"),
        projects: vec![],
        limits: Default::default(),
    };
    std::fs::write(data.join("policy"), "signed_commands = agent.prompt\n").unwrap();
    let node = agora_node::daemon::Node::start(&cfg).unwrap();
    let mut link = HubLink {
        url: url.clone(),
        ca_pem: ca_pem.clone(),
        cert_pem: issued.cert_pem.clone(),
        key_pem: key_pem.clone(),
        commands: Some(cfg.socket.clone()),
        mesh_out: None,
        grants: Some(Arc::clone(&node.grants)),
        mesh: None,
        devices: None,
    };
    link.devices = Some(Arc::clone(&node.devices));
    let _uplink = spawn(Arc::clone(&node.events), link).unwrap();
    let hub_trusts = |id: &str| {
        state
            .hub
            .view()
            .0
            .trusted_devices
            .get("laptop")
            .is_some_and(|ids| ids.iter().any(|d| d == id))
    };
    wait("the node to say what it trusts", || hub_trusts("web-old"));

    // A signed-in window of the owner.
    let mut client = rt.block_on(async {
        use rustls::pki_types::pem::PemObject;
        use rustls::pki_types::{CertificateDer, ServerName};
        let mut roots = rustls::RootCertStore::empty();
        for c in CertificateDer::pem_slice_iter(ca_pem.as_bytes()) {
            roots.add(c.unwrap()).unwrap();
        }
        let cfg = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
        let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
        let tls = tokio_rustls::TlsConnector::from(Arc::new(cfg))
            .connect(ServerName::try_from("localhost").unwrap(), tcp)
            .await
            .unwrap();
        let mut c = agora_protocol::ws::connect_with_subprotocol_token(
            tls,
            &format!("wss://localhost:{}/", addr.port()),
            "client-token",
        )
        .await
        .unwrap();
        c.send(&Frame {
            stream_id: STREAM_EVENTS,
            kind: kinds::SUBSCRIBE,
            seq: None,
            payload: v1::Subscribe::default().encode_to_vec(),
        })
        .await
        .unwrap();
        c
    });
    let mut n = 0;
    let mut call = |target: &str, cmd: cr::Command| -> String {
        n += 1;
        let id = format!("r{n}");
        let req = v1::CommandRequest {
            request_id: id.clone(),
            idempotency_key: String::new(),
            target_node: target.into(),
            command: Some(cmd),
        };
        rt.block_on(async {
            client
                .send(&Frame {
                    stream_id: STREAM_COMMANDS,
                    kind: 0,
                    seq: None,
                    payload: req.encode_to_vec(),
                })
                .await
                .unwrap();
            loop {
                let f = tokio::time::timeout(Duration::from_secs(20), client.recv())
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
                if (f.stream_id, f.kind) == (STREAM_COMMANDS, 1) {
                    let r = v1::CommandResponse::decode(f.payload.as_slice()).unwrap();
                    if r.request_id == id {
                        return match r.result {
                            Some(command_response::Result::Err(e)) => e.code,
                            _ => "ok".to_string(),
                        };
                    }
                }
            }
        })
    };
    let now = agora_node::grants::now_ms();
    let endorsement = |signer: &Key, by: &str, issued: u64, expires: u64| {
        let mut e = v1::DeviceEndorsement {
            device_id: "web-third".into(),
            public_key: third.public(),
            name: "Browser web-third".into(),
            nodes: vec![],
            issued_at_ms: issued,
            expires_at_ms: expires,
            endorser_device_id: by.into(),
            signature: Vec::new(),
        };
        e.signature = signer.sign(&endorse_message(&e));
        cr::Command::DeviceEndorse(v1::DeviceEndorse {
            endorsement: Some(e),
        })
    };
    // A grant signed by web-third: what the machine accepts only from a device it trusts.
    let mut g = 0;
    let mut grant = |signer: &Key| {
        g += 1;
        let cmds = vec!["agent.prompt".to_string()];
        let id = format!("g-{g}");
        let expires = now + 3_600_000;
        let msg = agora_node::grants::signed_message("laptop", &id, "node", &cmds, expires);
        cr::Command::GrantCreate(v1::GrantCreate {
            grant_id: id,
            scope: "node".into(),
            commands: cmds,
            expires_at_ms: expires,
            signer_device_id: "web-third".into(),
            signature: signer.sign(&msg),
        })
    };
    assert_eq!(
        call("laptop", grant(&third)),
        "approval.signature_invalid",
        "not trusted yet"
    );

    // Anyone without web-old's key makes up an endorsement: the hub does not even keep it...
    assert_eq!(
        call("", endorsement(&fake, "web-old", now, now + 86_400_000)),
        "device.signature_invalid"
    );
    // ...and a hub that carries one anyway is refused on the machine.
    let cr::Command::DeviceEndorse(v1::DeviceEndorse {
        endorsement: Some(forged),
    }) = endorsement(&fake, "web-old", now, now + 86_400_000)
    else {
        panic!("not an endorsement")
    };
    assert_eq!(
        node.devices.endorse(&forged, now).unwrap_err().code,
        agora_protocol::ErrorCode::DeviceSignatureInvalid
    );
    // An expired one is refused before it travels (and the machine would refuse it too).
    assert_eq!(
        call(
            "",
            endorsement(&old, "web-old", now - 3 * 86_400_000, now - 86_400_000)
        ),
        "device.endorsement_expired"
    );
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(trusted(&file), ["web-old"]);
    assert_eq!(call("laptop", grant(&third)), "approval.signature_invalid");
    let log = std::fs::read_to_string(data.join("device-trust.log")).unwrap_or_default();
    assert!(
        log.contains("endorse web-third by=web-old device.signature_invalid"),
        "{log}"
    );

    // The owner's old device approves the new one: carried by the hub, verified and trusted on the machine.
    assert_eq!(
        call("", endorsement(&old, "web-old", now, now + 30 * 86_400_000)),
        "ok"
    );
    wait("the machine to trust web-third", || hub_trusts("web-third"));
    assert_eq!(call("laptop", grant(&third)), "ok");

    // web-old revokes it: the machine stops trusting it at once.
    let mut r = v1::DeviceRevocation {
        device_id: "web-third".into(),
        issued_at_ms: agora_node::grants::now_ms(),
        revoker_device_id: "web-old".into(),
        signature: Vec::new(),
    };
    r.signature = old.sign(&revoke_message(&r));
    assert_eq!(
        call(
            "",
            cr::Command::DeviceRevoke(v1::DeviceRevoke {
                revocation: Some(r)
            })
        ),
        "ok"
    );
    wait("the machine to drop web-third", || !hub_trusts("web-third"));
    assert_eq!(call("laptop", grant(&third)), "approval.signature_invalid");
    assert_eq!(trusted(&file), ["web-old"]);

    drop(node);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn enrolling_shows_the_registered_devices_and_trusts_only_the_ones_the_owner_names() {
    let ca = Ca::generate("hub ca").unwrap();
    let (server_cert, server_key) = ca.issue_server(&["localhost".to_string()]).unwrap();
    let acceptor = tls::acceptor_with(&server_cert, &server_key, ca.cert_pem(), true).unwrap();
    let store = SqliteStore::open_in_memory().unwrap();
    let ca_pem = ca.cert_pem().to_string();
    let (mine, other) = (Key::new(), Key::new());
    let devices = agora_hub::devices::Devices::new(&store);
    for (id, k) in [("web-mine", &mine), ("web-other", &other)] {
        devices
            .register(
                "owner",
                &k.registration(id, &format!("Browser {id}")),
                server::now_ms(),
            )
            .unwrap();
    }
    let codes: Vec<String> = (0..3)
        .map(|_| {
            Enrollment::new(&store)
                .create_code(server::now_ms())
                .unwrap()
        })
        .collect();
    let state = Arc::new(ServerState {
        hub: Arc::new(Hub::open(store).unwrap()),
        support: Support {
            generations: vec![1],
            capabilities: vec![],
            required: vec![],
        },
        ca: Some(ca),
        lockout: Arc::new(Lockout::new()),
        auth: Box::new(Tokens),
        assets: None,
        sessions: None,
    });
    let rt = tokio::runtime::Runtime::new().unwrap();
    let listener = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    rt.spawn(server::run(listener, Arc::clone(&state), Some(acceptor)));
    let url = format!("wss://localhost:{}", addr.port());
    let home: PathBuf =
        std::env::temp_dir().join(format!("agora-devenroll-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let ca_file = home.join("hub-ca.pem");
    std::fs::write(&ca_file, &ca_pem).unwrap();
    let data = home.join("data");
    let file = data.join("trusted-devices");
    let base = |code: &str| {
        vec![
            "node".to_string(),
            "enroll".into(),
            "--hub".into(),
            url.clone(),
            "--ca".into(),
            ca_file.display().to_string(),
            "--code".into(),
            code.to_string(),
            "--id".into(),
            "box".into(),
            "--data".into(),
            data.display().to_string(),
        ]
    };
    let run = |args: Vec<String>| {
        let a: Vec<&str> = args.iter().map(String::as_str).collect();
        agora(&home, &a)
    };

    // `--yes` without a fingerprint names nothing: refused before anything is enrolled (the code is not spent).
    let mut args = base(&codes[0]);
    args.push("--yes".into());
    let (ok, _, _) = run(args);
    assert!(!ok);
    assert!(!home.join("link").exists());

    // Enrolled from a script (no terminal to answer): the devices are listed with fingerprints, none is trusted.
    let (ok, out, err) = run(base(&codes[0]));
    assert!(ok, "{out}{err}");
    assert!(
        out.contains(&fingerprint(&mine.public())) && out.contains(&fingerprint(&other.public())),
        "{out}"
    );
    assert!(out.contains("Browser web-mine"), "{out}");
    assert!(trusted(&file).is_empty());

    // Enrolled again, naming one fingerprint: that device only.
    let mut args = base(&codes[1]);
    args.extend([
        "--yes".into(),
        "--fingerprint".into(),
        fingerprint(&mine.public()),
    ]);
    let (ok, out, err) = run(args);
    assert!(ok, "{out}{err}");
    assert_eq!(trusted(&file), ["web-mine"]);
    let _ = std::fs::remove_dir_all(&home);
}
