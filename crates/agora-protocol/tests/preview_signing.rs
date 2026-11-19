//! ADR-0088: the vectors of the `preview-action-v1` signing domain match the layout in docs/specs/protocol.md.
#![allow(clippy::expect_used)] // expect is allowed in tests

use std::path::PathBuf;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `F(x)`: the decimal byte length of `x`, `:`, then `x`.
fn f(x: &str) -> String {
    format!("{}:{x}", x.len())
}

/// The signed bytes: the domain, one NUL, then every field length-prefixed in the order sent.
fn signed(node: &str, port: u32, seconds: u32, nonce: &str, expires: u64) -> Vec<u8> {
    let mut out = String::from("preview-action-v1\0");
    for part in [
        node,
        "open",
        &port.to_string(),
        &seconds.to_string(),
        nonce,
        &expires.to_string(),
    ] {
        out.push_str(&f(part));
    }
    out.into_bytes()
}

#[test]
fn preview_action_v1_vectors_follow_the_documented_layout() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../schema/fixtures/signing/preview-action-v1.json");
    let text = std::fs::read_to_string(path).expect("fixture is committed");
    let doc: serde_json::Value = serde_json::from_str(&text).expect("fixture is JSON");
    let cases = doc["cases"].as_array().expect("cases");
    assert!(cases.len() >= 3);
    for c in cases {
        let field = |k: &str| c[k].as_str().unwrap_or_default().to_string();
        let num = |k: &str| c[k].as_u64().expect("number");
        let bytes = signed(
            &field("node"),
            num("port") as u32,
            num("expires_seconds") as u32,
            &field("nonce"),
            num("expires"),
        );
        assert_eq!(hex(&bytes), field("hex"), "{}", field("name"));
    }
}

/// A node or a nonce may hold any byte, NUL included. Where a NUL would end one field and start the next under plain joining, the
/// length prefixes keep different tuples apart.
#[test]
fn fields_holding_nul_bytes_sign_different_bytes() {
    // Under NUL-joining both tuples read `a NUL b NUL open ...` up to where the nonce starts.
    let a = signed("a\0b", 3000, 0, "n", 1);
    let b = signed("a", 3000, 0, "b\0n", 1);
    assert_ne!(a, b);
    let c = signed("laptop", 3000, 0, "x\0y", 1);
    let d = signed("laptop\0x", 3000, 0, "y", 1);
    assert_ne!(c, d);
    // Moving a byte between the node and the nonce changes the prefixes, so the bytes.
    assert_ne!(signed("ab", 1, 0, "c", 1), signed("a", 1, 0, "bc", 1));
}
