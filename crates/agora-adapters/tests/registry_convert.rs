#![allow(clippy::unwrap_used)]
//! ADR-0092 golden fixtures: `<id>.agent.json` in, `<id>.manifest.toml` and `<id>.package.json` out, or `<id>.skip` holding the
//! reason code. Fixtures are only ever added, never edited.

use std::fs;
use std::path::Path;

use agora_adapters::registry_convert::convert;
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
