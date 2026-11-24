#![allow(clippy::unwrap_used)]
//! ADR-0092 golden fixtures: `<id>.agent.json` in, `<id>.manifest.toml` and `<id>.package.json` out, or `<id>.skip` holding the
//! reason code. Fixtures are only ever added, never edited.

use std::fs;
use std::path::Path;

use agora_adapters::registry_convert::{
    catalog_entry, check_icon, convert, quarantine_list, Reason, MAX_ICON_BYTES,
};
use agora_adapters::Manifest;

#[test]
fn golden_fixtures() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/registry");
    let mut n = 0;
    for e in fs::read_dir(&dir).unwrap() {
        let path = e.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap().to_string();
        let Some(id) = name.strip_suffix(".agent.json") else {
            continue;
        };
        n += 1;
        let got = convert(&fs::read_to_string(&path).unwrap());
        let skip = dir.join(format!("{id}.skip"));
        if skip.exists() {
            let want = fs::read_to_string(&skip).unwrap();
            assert_eq!(got.unwrap_err().code(), want.trim(), "{id}");
            continue;
        }
        let c = got.unwrap_or_else(|r| panic!("{id}: {}", r.code()));
        assert_eq!(
            c.manifest,
            // A checkout on Windows may turn line ends into CRLF; the converter always writes LF.
            fs::read_to_string(dir.join(format!("{id}.manifest.toml")))
                .unwrap()
                .replace("\r\n", "\n"),
            "{id}"
        );
        let want: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(dir.join(format!("{id}.package.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(c.package, want, "{id}");
        // The output is a valid manifest, structured, and has no way to skip permissions.
        let m = Manifest::parse(&c.manifest).unwrap_or_else(|e| panic!("{id}: {e:?}"));
        assert!(m.structured.is_some() && m.launch.option.is_empty(), "{id}");
        assert!(c.manifest.starts_with("# Generated"), "{id}");
        assert!(
            !c.manifest.contains("FAST_AGENT_MODEL")
                && !c.package.to_string().contains("codexplan"),
            "env values are never copied"
        );
    }
    assert!(n >= 9);
}

#[test]
fn not_json_and_bad_id() {
    assert_eq!(convert("nope").unwrap_err().code(), "registry.not_json");
    let bad = r#"{"id":"../x","name":"X","version":"1.0.0","distribution":{}}"#;
    assert_eq!(convert(bad).unwrap_err().code(), "registry.bad_id");
}

fn doc(id: &str, name: &str, extra: &str) -> String {
    format!(
        r#"{{"id":"{id}","name":"{name}","version":"1.0.0"{extra},"distribution":{{"npx":{{"package":"ok@1.0.0","env":{{"TOKEN":"s3cr3t-value"}}}}}}}}"#
    )
}

#[test]
fn hostile_ids_and_text_are_refused() {
    for id in ["../x", "a/b", "A B", "x\\u202ey", "", "-x"] {
        assert_eq!(
            convert(&doc(id, "X", "")).unwrap_err().code(),
            "registry.bad_id",
            "{id}"
        );
    }
    // Bidi override, zero width space, line separator, control character.
    for name in ["Evil\\u202etxt", "a\\u200bb", "a\\u2028b", "a\\u0007b"] {
        assert_eq!(
            convert(&doc("fine", name, "")).unwrap_err().code(),
            "registry.bad_field",
            "{name}"
        );
    }
    for k in ["repository", "license"] {
        let extra = format!(r#","{k}":"x\u202ey""#);
        assert_eq!(
            convert(&doc("fine", "X", &extra)).unwrap_err().code(),
            "registry.bad_field",
            "{k}"
        );
    }
}

#[test]
fn env_values_are_never_present() {
    let c = convert(&doc("fine", "X", "")).unwrap();
    assert!(
        !c.manifest.contains("s3cr3t-value") && !c.package.to_string().contains("s3cr3t-value")
    );
    assert!(c.package.to_string().contains("TOKEN"));
}

#[test]
fn a_clean_entry_still_converts() {
    let c = convert(&doc("fine", "Fine", r#","license":"MIT""#)).unwrap();
    assert_eq!(c.kind, "registry-fine");
}

#[test]
fn a_quarantined_entry_is_named_with_its_reason() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/registry");
    let q = quarantine_list(&fs::read_to_string(dir.join("quarantine.json")).unwrap()).unwrap();
    assert_eq!(
        q.keys().map(String::as_str).collect::<Vec<_>>(),
        ["odd-held-back", "odd-other"]
    );
    assert_eq!(Reason::Quarantined.code(), "registry.quarantined");
    // Not an object of names to text: refused whole, never guessed at.
    for bad in ["[]", "nope", r#"{"../x":"y"}"#, r#"{"a":1}"#] {
        assert!(quarantine_list(bad).is_err(), "{bad}");
    }
    assert!(quarantine_list("{}").unwrap().is_empty());
}

#[test]
fn icons_are_plain_drawings_or_refused() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/registry/icons");
    let mut seen = 0;
    for e in fs::read_dir(&dir).unwrap() {
        let path = e.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap().to_string();
        let bytes = fs::read(&path).unwrap();
        let got = check_icon(&bytes);
        seen += 1;
        if name.starts_with("good-") {
            let svg = got.unwrap_or_else(|r| panic!("{name}: {}", r.code()));
            assert!(
                svg.starts_with("<svg"),
                "{name}: the prolog, doctype and comments are dropped"
            );
            assert!(!svg.contains("<!") && !svg.contains("<?"), "{name}");
            // What is kept is checked again the same way: the output is a fixed point.
            assert_eq!(check_icon(svg.as_bytes()).unwrap(), svg, "{name}");
        } else {
            assert_eq!(
                got.map(|_| ()).unwrap_err().code(),
                "registry.bad_icon",
                "{name}"
            );
        }
    }
    assert!(seen >= 20);
    assert!(check_icon(&[0xff, 0xfe, b'<']).is_err(), "not UTF-8");
    assert!(check_icon(b"").is_err());
    assert_eq!(MAX_ICON_BYTES, 16 * 1024);
}

#[test]
fn a_catalog_line_carries_what_the_window_shows() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/registry");
    let c = convert(&fs::read_to_string(dir.join("odd-percmd.agent.json")).unwrap()).unwrap();
    let line = catalog_entry(&c, true);
    assert_eq!(
        line,
        serde_json::json!({
            "kind": "registry-odd-percmd", "id": "odd-percmd", "name": "Odd Per Command", "version": "0.4.0",
            "license": "Apache-2.0", "repository": "https://example.com/odd/percmd", "runs": "binary", "icon": true, "tested": false,
        })
    );
    let c = convert(&fs::read_to_string(dir.join("odd-uvx-at.agent.json")).unwrap()).unwrap();
    assert_eq!(catalog_entry(&c, false)["runs"], "uvx");
    assert_eq!(catalog_entry(&c, false)["icon"], false);
}

/// The committed conversion (`manifests/agents/registry/`) is whole: a manifest, a package draft and a catalog line per agent, an icon
/// only where the catalog says so and only one that passes the check, and counts that agree with `SOURCE` and `SKIPPED`.
#[test]
fn the_committed_registry_folder_is_consistent() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../manifests/agents/registry");
    let mut ids: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| {
            let n = e.unwrap().file_name().to_string_lossy().into_owned();
            n.strip_suffix(".toml").map(str::to_string)
        })
        .collect();
    ids.sort();
    assert!(ids.len() >= 40, "{}", ids.len());
    let catalog: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("catalog.json")).unwrap()).unwrap();
    let lines = catalog["agents"].as_array().unwrap();
    let listed: Vec<&str> = lines.iter().map(|l| l["id"].as_str().unwrap()).collect();
    assert_eq!(listed, ids, "one catalog line per manifest, in order");
    for (id, line) in ids.iter().zip(lines) {
        let m = Manifest::parse(&fs::read_to_string(dir.join(format!("{id}.toml"))).unwrap())
            .unwrap_or_else(|e| panic!("{id}: {e:?}"));
        assert_eq!(m.kind, format!("registry-{id}"));
        assert_eq!(line["kind"], m.kind.as_str());
        assert_eq!(line["version"], m.version.as_str());
        assert_eq!(line["tested"], false);
        let pkg: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(dir.join(format!("{id}.package.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(pkg["name"], m.kind.as_str(), "{id}");
        assert_eq!(pkg["tested"], false, "{id}");
        let icon = dir.join("icons").join(format!("{id}.svg"));
        assert_eq!(line["icon"].as_bool().unwrap(), icon.exists(), "{id}");
        if icon.exists() {
            let svg = fs::read_to_string(&icon).unwrap();
            assert_eq!(check_icon(svg.as_bytes()).unwrap() + "\n", svg, "{id}");
        }
    }
    let source = fs::read_to_string(dir.join("SOURCE")).unwrap();
    let field = |k: &str| {
        source
            .lines()
            .find_map(|l| l.strip_prefix(&format!("{k} = ")))
            .unwrap()
            .to_string()
    };
    assert_eq!(field("converted"), ids.len().to_string());
    let skipped = fs::read_to_string(dir.join("SKIPPED")).unwrap();
    let skipped: Vec<&str> = skipped.lines().filter(|l| !l.starts_with('#')).collect();
    assert_eq!(field("skipped"), skipped.len().to_string());
    assert_eq!(
        field("entries").parse::<usize>().unwrap(),
        ids.len() + skipped.len()
    );
    for s in skipped {
        let (id, code) = s.split_once(' ').unwrap();
        assert!(!ids.iter().any(|i| i == id), "{id}");
        assert!(code.starts_with("registry."), "{s}");
    }
    let icons = fs::read_dir(dir.join("icons")).unwrap().count();
    assert_eq!(icons, lines.iter().filter(|l| l["icon"] == true).count());
}
