//! ADR-0114 decision 6: the hub checks a plugin package exactly as the window does. Every package in `fixtures/plugin-packages/` is
//! checked here and by `apps/web/src/plugins/package.test.ts` against the same `expected.json`: both accept the same packages, and
//! refuse the others with the same code and detail. A case is added by adding a package and its line; none is edited.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::PathBuf;

use agora_hub::plugin_package::{check, MAX_PACKAGE_BYTES};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plugin-packages")
}

#[test]
fn the_hub_and_the_window_agree_on_every_shared_package() {
    let expected: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir().join("expected.json")).unwrap())
            .unwrap();
    let version = expected["agoraVersion"].as_str().unwrap();
    let cases = expected["cases"].as_array().unwrap();
    assert!(cases.len() >= 30, "the shared set is all there");
    let mut files: Vec<String> = std::fs::read_dir(dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".bundle"))
        .collect();
    files.sort();
    let mut listed: Vec<String> = cases
        .iter()
        .map(|c| c["file"].as_str().unwrap().to_string())
        .collect();
    listed.sort();
    assert_eq!(files, listed, "every package has one line in expected.json");
    for c in cases {
        let file = c["file"].as_str().unwrap();
        let bytes = std::fs::read(dir().join(file)).unwrap();
        let got = check(&bytes, version);
        if c["ok"].as_bool().unwrap() {
            let m = got
                .unwrap_or_else(|e| panic!("{file}: refused {e:?}"))
                .manifest;
            assert_eq!(m.id, c["id"].as_str().unwrap(), "{file}");
            assert_eq!(m.version, c["version"].as_str().unwrap(), "{file}");
            let caps: Vec<&str> = c["capabilities"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_str().unwrap())
                .collect();
            assert_eq!(m.capabilities, caps, "{file}");
        } else {
            let e = got.err().unwrap_or_else(|| panic!("{file}: accepted"));
            assert_eq!(e.code.as_str(), c["code"].as_str().unwrap(), "{file}");
            assert_eq!(e.detail.as_deref(), c["detail"].as_str(), "{file}");
        }
    }
}

/// Over 2 MiB is refused before anything is read (not a shared file: it would put 2 MiB in the repository).
#[test]
fn a_package_over_two_mib_is_refused_unread() {
    let big = vec![b' '; MAX_PACKAGE_BYTES + 1];
    assert_eq!(check(&big, "0.3.0").unwrap_err().code.as_str(), "too-large");
}
