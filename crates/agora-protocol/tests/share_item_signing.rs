//! ADR-0122: the vectors of the `agora-share-item-v1` signing domain (and of an item's content hash) match the layout in
//! docs/specs/protocol.md "Shared skills and MCP servers".
#![allow(clippy::expect_used)] // expect is allowed in tests

use std::path::PathBuf;

use serde_json::Value;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 (FIPS 180-4), here so the test needs no dependency of its own.
fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in msg.chunks(64) {
        let mut w = [0u32; 64];
        for (i, c) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7]
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [
                t1.wrapping_add(t2),
                v[0],
                v[1],
                v[2],
                v[3].wrapping_add(t1),
                v[4],
                v[5],
                v[6],
            ];
        }
        for (a, b) in h.iter_mut().zip(v) {
            *a = a.wrapping_add(b);
        }
    }
    let mut out = [0u8; 32];
    for (i, x) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}

/// F(x): the decimal byte length, `:`, then x.
fn field(out: &mut Vec<u8>, x: &[u8]) {
    out.extend_from_slice(x.len().to_string().as_bytes());
    out.push(b':');
    out.extend_from_slice(x);
}

fn list(items: &[String]) -> String {
    let mut s = items.len().to_string();
    for i in items {
        s.push_str(&format!(":{}:{i}", i.len()));
    }
    s
}

fn pairs(items: &[(String, String)]) -> String {
    let mut s = items.len().to_string();
    for (a, b) in items {
        s.push_str(&format!(":{}:{a}:{}:{b}", a.len(), b.len()));
    }
    s
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .map(|x| x.as_str().expect("text").to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn sorted_pairs(v: &Value) -> Vec<(String, String)> {
    let mut p: Vec<(String, String)> = v
        .as_array()
        .map(|a| {
            a.iter()
                .map(|x| {
                    (
                        x[0].as_str().expect("name").to_string(),
                        x[1].as_str().expect("value").to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    p.sort();
    p
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

fn content(item: &Value) -> Vec<u8> {
    let mut out = b"agora-share-item-content-v1\0".to_vec();
    let kind = s(item, "kind");
    field(&mut out, kind.as_bytes());
    field(&mut out, s(item, "name").as_bytes());
    if kind == "mcp_server" {
        let m = &item["mcp"];
        field(&mut out, s(m, "command").as_bytes());
        field(&mut out, list(&strs(&m["args"])).as_bytes());
        field(&mut out, pairs(&sorted_pairs(&m["env"])).as_bytes());
        field(&mut out, s(m, "url").as_bytes());
        field(&mut out, pairs(&sorted_pairs(&m["headers"])).as_bytes());
    } else {
        let k = &item["skill"];
        field(&mut out, s(k, "description").as_bytes());
        let mut files: Vec<(String, String)> = k["files"]
            .as_array()
            .expect("files")
            .iter()
            .map(|f| {
                (
                    s(f, "path").to_string(),
                    hex(&sha256(s(f, "content").as_bytes())),
                )
            })
            .collect();
        files.sort();
        field(&mut out, pairs(&files).as_bytes());
    }
    out
}

#[test]
fn the_share_item_vectors_match_the_layout() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../schema/fixtures/signing/share-item-v1.json");
    let doc: Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("vectors")).expect("json");
    let cases = doc["cases"].as_array().expect("cases");
    assert!(cases.len() >= 4);
    for c in cases {
        let name = s(c, "name");
        let hash = if c["item"].is_null() {
            String::new()
        } else {
            let bytes = content(&c["item"]);
            assert_eq!(hex(&bytes), s(c, "content_hex"), "{name}: content");
            hex(&sha256(&bytes))
        };
        assert_eq!(hash, s(c, "item_hash"), "{name}: hash");
        let mut out = b"agora-share-item-v1\0".to_vec();
        field(&mut out, s(c, "action").as_bytes());
        field(&mut out, s(c, "item_name").as_bytes());
        field(&mut out, hash.as_bytes());
        field(&mut out, s(c, "project_id").as_bytes());
        field(&mut out, list(&strs(&c["kinds"])).as_bytes());
        field(
            &mut out,
            c["issued_ms"]
                .as_u64()
                .expect("issued_ms")
                .to_string()
                .as_bytes(),
        );
        assert_eq!(hex(&out), s(c, "hex"), "{name}: signed bytes");
    }
}

/// The item a vector names, as the protocol carries it.
fn item_of(v: &Value) -> agora_protocol::v1::ShareItem {
    use agora_protocol::v1::{
        McpServerDef, ShareItem, ShareItemKind, ShareValue, SkillDef, SkillFile,
    };
    let values = |x: &Value| -> Vec<ShareValue> {
        x.as_array()
            .map(|a| {
                a.iter()
                    .map(|p| ShareValue {
                        name: p[0].as_str().expect("name").into(),
                        value: p[1].as_str().expect("value").into(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    if s(v, "kind") == "mcp_server" {
        let m = &v["mcp"];
        ShareItem {
            name: s(v, "name").into(),
            kind: ShareItemKind::McpServer as i32,
            mcp: Some(McpServerDef {
                command: s(m, "command").into(),
                args: strs(&m["args"]),
                env: values(&m["env"]),
                url: s(m, "url").into(),
                headers: values(&m["headers"]),
            }),
            skill: None,
        }
    } else {
        let k = &v["skill"];
        ShareItem {
            name: s(v, "name").into(),
            kind: ShareItemKind::Skill as i32,
            mcp: None,
            skill: Some(SkillDef {
                description: s(k, "description").into(),
                files: k["files"]
                    .as_array()
                    .expect("files")
                    .iter()
                    .map(|f| SkillFile {
                        path: s(f, "path").into(),
                        content: s(f, "content").as_bytes().to_vec(),
                    })
                    .collect(),
            }),
        }
    }
}

/// `agora_protocol::share`, which the hub and the node use, builds the same bytes as the vectors, and every vector's item keeps the
/// rules.
#[test]
fn the_protocols_share_module_builds_the_vectors() {
    use agora_protocol::share::{check_item, content_bytes, signed_message, Action};
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../schema/fixtures/signing/share-item-v1.json");
    let doc: Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("vectors")).expect("json");
    for c in doc["cases"].as_array().expect("cases") {
        let name = s(c, "name");
        let action = if s(c, "action") == "remove" {
            Action::Remove
        } else {
            Action::Publish
        };
        if !c["item"].is_null() {
            let item = item_of(&c["item"]);
            assert_eq!(check_item(&item), Ok(()), "{name}");
            let bytes = content_bytes(&item, |b| hex(&sha256(b)));
            assert_eq!(hex(&bytes), s(c, "content_hex"), "{name}: content");
        }
        let scope = agora_protocol::v1::ShareScope {
            project_id: s(c, "project_id").into(),
            kinds: strs(&c["kinds"]),
        };
        let msg = signed_message(
            action,
            s(c, "item_name"),
            s(c, "item_hash"),
            &scope,
            c["issued_ms"].as_u64().expect("issued_ms"),
        );
        assert_eq!(hex(&msg), s(c, "hex"), "{name}: signed bytes");
    }
}
