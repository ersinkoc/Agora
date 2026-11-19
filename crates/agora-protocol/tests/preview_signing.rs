//! ADR-0088: the vectors of the `preview-action-v1` signing domain match the layout in docs/specs/protocol.md.

#![allow(clippy::expect_used)] // expect is allowed in tests

use std::path::PathBuf;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn preview_action_v1_vectors_follow_the_documented_layout() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../schema/fixtures/signing/preview-action-v1.json");
    let text = std::fs::read_to_string(path).expect("fixture is committed");
    let doc: serde_json::Value = serde_json::from_str(&text).expect("fixture is JSON");
    let cases = doc["cases"].as_array().expect("cases");
    assert!(!cases.is_empty());
    for c in cases {
        let field = |k: &str| c[k].as_str().unwrap_or_default().to_string();
        let num = |k: &str| c[k].as_u64().expect("number").to_string();
        let parts = [
            "preview-action-v1".to_string(),
            field("node"),
            "open".to_string(),
            num("port"),
            num("expires_seconds"),
            field("nonce"),
            num("expires"),
        ];
        let bytes = parts.join("\0");
        assert_eq!(hex(bytes.as_bytes()), field("hex"), "{}", field("name"));
    }
}
