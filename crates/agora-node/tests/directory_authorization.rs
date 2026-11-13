#![allow(clippy::unwrap_used, clippy::expect_used)]
use agora_node::{
    approvals::{NodeApprovals, P256Verifier},
    directory::signed_message,
    grants::{Grants, SignedCommands},
};
use agora_protocol::v1::{self, command_request::Command as Cmd};
use ring::{
    rand::SystemRandom,
    signature::{self, KeyPair},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
fn key() -> signature::EcdsaKeyPair {
    let rng = SystemRandom::new();
    let pk =
        signature::EcdsaKeyPair::generate_pkcs8(&signature::ECDSA_P256_SHA256_FIXED_SIGNING, &rng)
            .unwrap();
    signature::EcdsaKeyPair::from_pkcs8(
        &signature::ECDSA_P256_SHA256_FIXED_SIGNING,
        pk.as_ref(),
        &rng,
    )
    .unwrap()
}
fn grants(key: &signature::EcdsaKeyPair) -> Grants {
    let mut v = P256Verifier::new();
    assert!(v.trust("browser", key.public_key().as_ref()));
    Grants::new(
        "node-a",
        Arc::new(NodeApprovals::new("node-a", Box::new(v))),
        SignedCommands::default(),
    )
}
fn launch(key: &signature::EcdsaKeyPair, nonce: &str) -> v1::AgentLaunch {
    let expires = 101_000;
    let message = signed_message(
        "node-a",
        "launch",
        "/selected",
        "probe",
        "id:project",
        false,
        false,
        nonce,
        expires,
    );
    let signature = key
        .sign(&SystemRandom::new(), &message)
        .unwrap()
        .as_ref()
        .to_vec();
    v1::AgentLaunch {
        path: "/selected".into(),
        kind: "probe".into(),
        project_id: "id:project".into(),
        authorization: Some(v1::DirectoryAuthorization {
            device_id: "browser".into(),
            nonce: nonce.into(),
            expires_at_ms: expires,
            signature,
        }),
        ..Default::default()
    }
}
#[test]
fn signatures_bind_every_launch_field_and_refuse_replay_including_after_restart() {
    let key = key();
    let nonce = "00112233445566778899aabbccddeeff";
    let a = launch(&key, nonce);
    let file = std::env::temp_dir().join(format!("directory-nonce-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&file);
    let g = grants(&key).with_directory_nonce_file(file.clone());
    for f in 0..6 {
        let mut b = a.clone();
        match f {
            0 => b.path = "/other".into(),
            1 => b.kind = "other".into(),
            2 => b.project_id = "other".into(),
            3 => b.separate_worktree = true,
            4 => b.accept_identity_change = true,
            _ => b.authorization.as_mut().unwrap().expires_at_ms += 1,
        };
        assert!(g.directory_action(&Cmd::AgentLaunch(b), 100_000).is_err());
    }
    assert!(g
        .directory_action(&Cmd::AgentLaunch(a.clone()), 100_000)
        .is_ok());
    assert!(g
        .directory_action(&Cmd::AgentLaunch(a.clone()), 100_000)
        .is_err());
    assert!(grants(&key)
        .with_directory_nonce_file(file.clone())
        .directory_action(&Cmd::AgentLaunch(a), 100_000)
        .is_err());
    let _ = std::fs::remove_file(file);
}
#[test]
fn unsigned_expired_cross_node_and_project_limited_requests_cannot_register_arbitrary_folders() {
    let key = key();
    let a = launch(&key, "00112233445566778899aabbccddeeff");
    let g = grants(&key);
    let mut missing = a.clone();
    missing.authorization = None;
    assert!(g
        .directory_action(&Cmd::AgentLaunch(missing), 100_000)
        .is_err());
    assert!(g
        .directory_action(&Cmd::AgentLaunch(a.clone()), 102_000)
        .is_err());
    let mut verifier = P256Verifier::new();
    verifier.trust("browser", key.public_key().as_ref());
    let other = Grants::new(
        "node-b",
        Arc::new(NodeApprovals::new("node-b", Box::new(verifier))),
        SignedCommands::default(),
    );
    assert!(other
        .directory_action(&Cmd::AgentLaunch(a.clone()), 100_000)
        .is_err());
    let limited = grants(&key).with_limits(Arc::new(agora_node::limits::DeviceLimits::new(
        BTreeMap::from([("browser".into(), BTreeSet::from(["id:project".into()]))]),
    )));
    assert!(limited
        .directory_action(&Cmd::AgentLaunch(a), 100_000)
        .is_err());
}
fn list(key: &signature::EcdsaKeyPair, op: &str, path: &str, nonce: &str) -> v1::DirectoryList {
    let expires = 101_000;
    let message = signed_message("node-a", op, path, "", "", false, false, nonce, expires);
    let signature = key
        .sign(&SystemRandom::new(), &message)
        .unwrap()
        .as_ref()
        .to_vec();
    v1::DirectoryList {
        path: path.into(),
        authorization: Some(v1::DirectoryAuthorization {
            device_id: "browser".into(),
            nonce: nonce.into(),
            expires_at_ms: expires,
            signature,
        }),
    }
}
#[test]
fn a_folder_listing_needs_the_owners_signature_for_that_folder() {
    let key = key();
    let g = grants(&key);
    let n = |i: u8| format!("{i:032x}");
    // Signed for listing this folder: allowed once.
    let ok = list(&key, "list", "/Users/me", &n(1));
    assert!(g
        .directory_action(&Cmd::DirectoryList(ok.clone()), 100_000)
        .is_ok());
    assert!(
        g.directory_action(&Cmd::DirectoryList(ok), 100_000)
            .is_err(),
        "replay"
    );
    // Unsigned, for another folder, or a signature made for inspecting: refused.
    let mut unsigned = list(&key, "list", "/Users/me", &n(2));
    unsigned.authorization = None;
    assert!(g
        .directory_action(&Cmd::DirectoryList(unsigned), 100_000)
        .is_err());
    let mut moved = list(&key, "list", "/Users/me", &n(3));
    moved.path = "/".into();
    assert!(g
        .directory_action(&Cmd::DirectoryList(moved), 100_000)
        .is_err());
    let inspect = list(&key, "inspect", "/Users/me", &n(4));
    assert!(g
        .directory_action(&Cmd::DirectoryList(inspect), 100_000)
        .is_err());
    // An empty path (the home folder) is signed like any other.
    assert!(g
        .directory_action(&Cmd::DirectoryList(list(&key, "list", "", &n(5))), 100_000)
        .is_ok());
    // A device limited to some projects never lists a machine's folders.
    let limited = grants(&key).with_limits(Arc::new(agora_node::limits::DeviceLimits::new(
        BTreeMap::from([("browser".into(), BTreeSet::from(["id:project".into()]))]),
    )));
    assert!(limited
        .directory_action(
            &Cmd::DirectoryList(list(&key, "list", "/Users/me", &n(6))),
            100_000
        )
        .is_err());
}

/// ADR-0060: `agent.resume` is signed like a launch, over the agent id and the Start again flag.
#[test]
fn a_resume_is_signed_for_that_agent_and_that_choice_only() {
    let key = key();
    let nonce = "ffeeddccbbaa99887766554433221100";
    let expires = 101_000;
    let message = signed_message(
        "node-a", "resume", "agent-3", "", "", false, false, nonce, expires,
    );
    let signature = key
        .sign(&SystemRandom::new(), &message)
        .unwrap()
        .as_ref()
        .to_vec();
    let a = v1::AgentResume {
        agent_id: "agent-3".into(),
        new_session: false,
        authorization: Some(v1::DirectoryAuthorization {
            device_id: "browser".into(),
            nonce: nonce.into(),
            expires_at_ms: expires,
            signature,
        }),
    };
    let g = grants(&key);
    let mut other = a.clone();
    other.agent_id = "agent-4".into();
    assert!(g
        .directory_action(&Cmd::AgentResume(other), 100_000)
        .is_err());
    let mut fresh = a.clone();
    fresh.new_session = true;
    assert!(g
        .directory_action(&Cmd::AgentResume(fresh), 100_000)
        .is_err());
    let mut unsigned = a.clone();
    unsigned.authorization = None;
    assert!(g
        .directory_action(&Cmd::AgentResume(unsigned), 100_000)
        .is_err());
    assert!(g
        .directory_action(&Cmd::AgentResume(a.clone()), 100_000)
        .is_ok());
    assert!(g.directory_action(&Cmd::AgentResume(a), 100_000).is_err());
}

/// ADR-0059: a launch that chose options is signed over them and over which of them the user confirmed (`directory-action-v3`,
/// two length-prefixed lists); a hub can neither add, remove or swap an option nor add or strip a confirmation.
#[test]
fn launch_options_are_signed_so_a_hub_can_neither_add_nor_remove_one() {
    let key = key();
    let strs = |v: &[&str]| v.iter().map(|o| (*o).to_string()).collect::<Vec<String>>();
    let sign = |options: &[&str], confirmed: &[&str], nonce: &str| {
        let (options, confirmed) = (strs(options), strs(confirmed));
        let message = agora_node::directory::launch_signed_message(
            "node-a",
            "/selected",
            "probe",
            "id:project",
            false,
            false,
            &options,
            &confirmed,
            nonce,
            101_000,
        );
        let mut a = launch(&key, nonce);
        a.launch_options = options;
        a.confirmed_launch_options = confirmed;
        a.authorization.as_mut().unwrap().signature = key
            .sign(&SystemRandom::new(), &message)
            .unwrap()
            .as_ref()
            .to_vec();
        a
    };
    let g = grants(&key);
    // Signed without options: the hub adds one, or a confirmation.
    let mut added = launch(&key, "00000000000000000000000000000001");
    added.launch_options = strs(&["skip-permissions"]);
    assert!(g
        .directory_action(&Cmd::AgentLaunch(added.clone()), 100_000)
        .is_err());
    added.confirmed_launch_options = strs(&["skip-permissions"]);
    assert!(g
        .directory_action(&Cmd::AgentLaunch(added), 100_000)
        .is_err());
    let signed = sign(
        &["skip-permissions"],
        &["skip-permissions"],
        "00000000000000000000000000000002",
    );
    for (options, confirmed) in [
        (vec![], vec![]),
        (strs(&["skip-permissions"]), vec![]),
        (strs(&["resume-last"]), strs(&["skip-permissions"])),
        (
            strs(&["skip-permissions", "resume-last"]),
            strs(&["skip-permissions"]),
        ),
        (strs(&["skip-permissions!"]), vec![]),
    ] {
        let mut b = signed.clone();
        b.launch_options = options;
        b.confirmed_launch_options = confirmed;
        assert!(g.directory_action(&Cmd::AgentLaunch(b), 100_000).is_err());
    }
    // A signature over the unconfirmed option does not confirm it: the hub cannot add the confirmation afterwards.
    let mut unconfirmed = sign(
        &["skip-permissions"],
        &[],
        "00000000000000000000000000000003",
    );
    unconfirmed.confirmed_launch_options = strs(&["skip-permissions"]);
    assert!(g
        .directory_action(&Cmd::AgentLaunch(unconfirmed), 100_000)
        .is_err());
    // One id `a,b` is not the two ids `a` and `b`.
    let mut comma = sign(&["a,b"], &[], "00000000000000000000000000000004");
    comma.launch_options = strs(&["a", "b"]);
    assert!(g
        .directory_action(&Cmd::AgentLaunch(comma), 100_000)
        .is_err());
    assert!(g
        .directory_action(&Cmd::AgentLaunch(signed), 100_000)
        .is_ok());
}

/// A device limited to some projects (ADR-0049) takes directory actions only inside its own projects, as the node itself knows
/// them (folders launches confirmed, registered checkouts, the ledger), and keeps exactly that after the node restarts.
mod limited_device {
    use super::*;
    use agora_node::directory::{known_project, Bindings};
    use agora_node::grants::Place;
    use agora_protocol::ErrorCode;
    use std::path::{Path, PathBuf};

    const NOW: u64 = 100_000;
    const EXPIRES: u64 = 101_000;

    struct Machine {
        dir: PathBuf,
        bindings: PathBuf,
        nonces: PathBuf,
        own: String,
        sub: String,
        other: String,
        outside: String,
    }

    fn canon(p: &Path) -> String {
        agora_platform::canonicalize_plain(p)
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    fn machine(tag: &str) -> Machine {
        let dir = std::env::temp_dir().join(format!("agora-limited-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for d in ["own/src", "other", "outside"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        let bindings = dir.join("bindings.json");
        // What earlier launches confirmed (the owner's, say): `own` is project `id:own`, `other` is `id:other`.
        let mut b = Bindings::load(bindings.clone());
        for (folder, project) in [("own", "id:own"), ("other", "id:other")] {
            b.confirm(&v1::DirectoryInfo {
                path: canon(&dir.join(folder)),
                project_id: project.into(),
                ..Default::default()
            })
            .unwrap();
        }
        Machine {
            own: canon(&dir.join("own")),
            sub: canon(&dir.join("own/src")),
            other: canon(&dir.join("other")),
            outside: canon(&dir.join("outside")),
            nonces: dir.join("nonces.json"),
            bindings,
            dir,
        }
    }

    /// The node as the daemon builds it: limits from the policy, places from its own files. Built again for a restart.
    fn node(key: &signature::EcdsaKeyPair, m: &Machine) -> Grants {
        let limits = Arc::new(agora_node::limits::DeviceLimits::new(BTreeMap::from([(
            "browser".into(),
            BTreeSet::from(["id:own".into()]),
        )])));
        let g = grants(key)
            .with_limits(limits)
            .with_directory_nonce_file(m.nonces.clone());
        let bindings = m.bindings.clone();
        g.set_place_resolver(Box::new(move |place| match place {
            Place::Folder(path) => known_project(&bindings, &[], path),
            Place::Agent("agent-1") => Some("id:own".into()),
            Place::Agent("agent-2") => Some("id:other".into()),
            Place::Agent(_) => None,
        }));
        g
    }

    fn signature(key: &signature::EcdsaKeyPair, message: &[u8]) -> Vec<u8> {
        key.sign(&SystemRandom::new(), message)
            .unwrap()
            .as_ref()
            .to_vec()
    }

    fn auth(
        key: &signature::EcdsaKeyPair,
        nonce: u32,
        message: &[u8],
    ) -> v1::DirectoryAuthorization {
        v1::DirectoryAuthorization {
            device_id: "browser".into(),
            nonce: format!("{nonce:032x}"),
            expires_at_ms: EXPIRES,
            signature: signature(key, message),
        }
    }

    fn launch_in(
        key: &signature::EcdsaKeyPair,
        nonce: u32,
        path: &str,
        project: &str,
        identity_change: bool,
    ) -> Cmd {
        let msg = signed_message(
            "node-a",
            "launch",
            path,
            "probe",
            project,
            false,
            identity_change,
            &format!("{nonce:032x}"),
            EXPIRES,
        );
        Cmd::AgentLaunch(v1::AgentLaunch {
            path: path.into(),
            kind: "probe".into(),
            project_id: project.into(),
            accept_identity_change: identity_change,
            authorization: Some(auth(key, nonce, &msg)),
            ..Default::default()
        })
    }

    fn inspect(key: &signature::EcdsaKeyPair, nonce: u32, path: &str, create: bool) -> Cmd {
        let msg = signed_message(
            "node-a",
            "inspect",
            path,
            "",
            "",
            create,
            false,
            &format!("{nonce:032x}"),
            EXPIRES,
        );
        Cmd::DirectoryInspect(v1::DirectoryInspect {
            path: path.into(),
            create_project_id: create,
            authorization: Some(auth(key, nonce, &msg)),
        })
    }

    fn list(key: &signature::EcdsaKeyPair, nonce: u32, path: &str) -> Cmd {
        let msg = signed_message(
            "node-a",
            "list",
            path,
            "",
            "",
            false,
            false,
            &format!("{nonce:032x}"),
            EXPIRES,
        );
        Cmd::DirectoryList(v1::DirectoryList {
            path: path.into(),
            authorization: Some(auth(key, nonce, &msg)),
        })
    }

    fn resume(key: &signature::EcdsaKeyPair, nonce: u32, agent: &str) -> Cmd {
        let msg = signed_message(
            "node-a",
            "resume",
            agent,
            "",
            "",
            false,
            false,
            &format!("{nonce:032x}"),
            EXPIRES,
        );
        Cmd::AgentResume(v1::AgentResume {
            agent_id: agent.into(),
            new_session: false,
            authorization: Some(auth(key, nonce, &msg)),
        })
    }

    fn forbidden(r: Result<(), agora_protocol::Error>) -> bool {
        matches!(r, Err(e) if e.code == ErrorCode::AuthForbidden)
    }

    /// Every check, from nonce `base` on (nonces are single-use across restarts, so each run uses its own).
    fn the_rule_holds(g: &Grants, key: &signature::EcdsaKeyPair, m: &Machine, base: u32) {
        let n = |i: u32| base + i;
        // Inside its own project: the folder, a folder inside it, inspecting and listing them, resuming its agent.
        assert!(g
            .directory_action(&launch_in(key, n(1), &m.own, "id:own", false), NOW)
            .is_ok());
        assert!(g
            .directory_action(&launch_in(key, n(2), &m.sub, "id:own", false), NOW)
            .is_ok());
        assert!(g
            .directory_action(&inspect(key, n(3), &m.own, false), NOW)
            .is_ok());
        assert!(g.directory_action(&list(key, n(4), &m.own), NOW).is_ok());
        assert!(g.directory_action(&list(key, n(5), &m.sub), NOW).is_ok());
        assert!(g
            .directory_action(&resume(key, n(6), "agent-1"), NOW)
            .is_ok());
        // Another project's folder, whatever project the request names.
        for (i, project) in [(7, "id:other"), (8, "id:own")] {
            assert!(forbidden(g.directory_action(
                &launch_in(key, n(i), &m.other, project, false),
                NOW
            )));
        }
        assert!(forbidden(
            g.directory_action(&inspect(key, n(9), &m.other, false), NOW)
        ));
        assert!(forbidden(
            g.directory_action(&list(key, n(10), &m.other), NOW)
        ));
        assert!(forbidden(
            g.directory_action(&resume(key, n(11), "agent-2"), NOW)
        ));
        // Its own folder signed for another project: the launch would not land where the request says.
        assert!(forbidden(g.directory_action(
            &launch_in(key, n(12), &m.own, "id:other", false),
            NOW
        )));
        // What is the machine's and not a project's: a folder no project has, the home folder, the root, a parent of its
        // project, a new identity, a changed identity, an agent the node cannot place.
        assert!(forbidden(g.directory_action(
            &launch_in(key, n(13), &m.outside, "id:own", false),
            NOW
        )));
        assert!(forbidden(
            g.directory_action(&inspect(key, n(14), &m.outside, false), NOW)
        ));
        assert!(forbidden(g.directory_action(&list(key, n(15), ""), NOW)));
        assert!(forbidden(g.directory_action(&list(key, n(16), "/"), NOW)));
        assert!(forbidden(
            g.directory_action(&list(key, n(17), &canon(&m.dir)), NOW)
        ));
        assert!(forbidden(
            g.directory_action(&inspect(key, n(18), &m.own, true), NOW)
        ));
        assert!(forbidden(g.directory_action(
            &launch_in(key, n(19), &m.own, "id:own", true),
            NOW
        )));
        assert!(forbidden(
            g.directory_action(&resume(key, n(20), "agent-9"), NOW)
        ));
        // `..` cannot walk out of the project: the path is judged as the node resolves it.
        let escaped = format!("{}/../other", m.own);
        assert!(forbidden(g.directory_action(
            &launch_in(key, n(21), &escaped, "id:own", false),
            NOW
        )));
        // A forged signature is refused as before, before the node says anything about the folder.
        let mut forged = launch_in(key, n(22), &m.own, "id:own", false);
        if let Cmd::AgentLaunch(a) = &mut forged {
            a.authorization.as_mut().unwrap().signature[10] ^= 1;
        }
        assert!(matches!(
            g.directory_action(&forged, NOW),
            Err(e) if e.code == ErrorCode::ApprovalSignatureInvalid
        ));
    }

    #[test]
    fn a_limited_device_starts_in_its_project_only_before_and_after_a_restart() {
        let key = key();
        let m = machine("restart");
        the_rule_holds(&node(&key, &m), &key, &m, 0x100);
        // The node restarts: limits, bindings and used nonces are read again from disk, and the rule is the same.
        let again = node(&key, &m);
        the_rule_holds(&again, &key, &m, 0x200);
        // A nonce used before the restart stays used.
        assert!(again
            .directory_action(&launch_in(&key, 0x101, &m.own, "id:own", false), NOW)
            .is_err());
        let _ = std::fs::remove_dir_all(&m.dir);
    }

    #[test]
    fn a_limited_device_still_cannot_sign_for_the_whole_machine() {
        let key = key();
        let m = machine("node-scope");
        let g = node(&key, &m);
        let commands = vec!["agent.start".to_string()];
        let msg = agora_node::grants::signed_message("node-a", "g1", "node", &commands, NOW + 1000);
        let grant = v1::GrantCreate {
            grant_id: "g1".into(),
            scope: "node".into(),
            commands,
            expires_at_ms: NOW + 1000,
            signer_device_id: "browser".into(),
            signature: signature(&key, &msg),
        };
        assert!(matches!(g.create(&grant, NOW), Err(e) if e.code == ErrorCode::AuthForbidden));
        let _ = std::fs::remove_dir_all(&m.dir);
    }

    #[test]
    fn without_places_a_limited_device_takes_no_directory_action_and_others_are_unchanged() {
        let key = key();
        let m = machine("unset");
        let limited = grants(&key).with_limits(Arc::new(agora_node::limits::DeviceLimits::new(
            BTreeMap::from([("browser".into(), BTreeSet::from(["id:own".into()]))]),
        )));
        assert!(forbidden(limited.directory_action(
            &launch_in(&key, 1, &m.own, "id:own", false),
            NOW
        )));
        // A device the policy does not limit keeps acting anywhere the owner signs for.
        let owner = grants(&key);
        assert!(owner
            .directory_action(&launch_in(&key, 2, &m.outside, "", false), NOW)
            .is_ok());
        let _ = std::fs::remove_dir_all(&m.dir);
    }

    #[test]
    fn the_node_places_folders_by_its_own_records_only() {
        let m = machine("places");
        // A registered checkout by path, id or name; the nearest known folder wins; nothing for an unknown folder.
        let projects = [(
            "id:reg".to_string(),
            "reg".to_string(),
            PathBuf::from(&m.outside),
        )];
        assert_eq!(
            known_project(&m.bindings, &projects, &m.outside).as_deref(),
            Some("id:reg")
        );
        assert_eq!(
            known_project(&m.bindings, &projects, "reg").as_deref(),
            Some("id:reg")
        );
        assert_eq!(
            known_project(&m.bindings, &projects, "id:reg").as_deref(),
            Some("id:reg")
        );
        assert_eq!(
            known_project(&m.bindings, &[], &m.sub).as_deref(),
            Some("id:own")
        );
        assert_eq!(known_project(&m.bindings, &[], &m.outside), None);
        assert_eq!(known_project(&m.bindings, &[], ""), None);
        // The folder's own metadata claiming a project changes nothing.
        std::fs::write(
            Path::new(&m.outside).join(agora_node::directory::METADATA),
            br#"{"project_id":"id:own"}"#,
        )
        .unwrap();
        assert_eq!(known_project(&m.bindings, &[], &m.outside), None);
        // The same folder known as two projects is placed in neither.
        let both = [(
            "id:third".to_string(),
            "x".to_string(),
            PathBuf::from(&m.own),
        )];
        assert_eq!(known_project(&m.bindings, &both, &m.own), None);
        // A damaged bindings file places nothing.
        std::fs::write(&m.bindings, b"not json").unwrap();
        assert_eq!(known_project(&m.bindings, &[], &m.own), None);
        let _ = std::fs::remove_dir_all(&m.dir);
    }
}
