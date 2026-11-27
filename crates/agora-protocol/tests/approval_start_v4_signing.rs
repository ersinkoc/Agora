//! ADR-0119 decision 4: the vectors of the `agora-approval-start-v4` signing domain match the layout in docs/specs/protocol.md.
#![allow(clippy::expect_used)] // expect is allowed in tests

use std::path::PathBuf;

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

/// `F(x)`: the decimal byte length of `x`, `:`, then `x`.
fn f(x: &str) -> String {
    format!("{}:{x}", x.len())
}

/// A length-prefixed list, as v3 writes `launch_options`: the count, then `:len:item` for each.
fn list(items: &[String]) -> String {
    let mut out = items.len().to_string();
    for i in items {
        out.push_str(&format!(":{}:{i}", i.len()));
    }
    out
}

/// A length-prefixed pair list, as v2 and v3 write `settings`.
fn pairs(items: &[(String, String)]) -> String {
    let mut out = items.len().to_string();
    for (id, value) in items {
        for p in [id, value] {
            out.push_str(&format!(":{}:{p}", p.len()));
        }
    }
    out
}

#[test]
fn approval_start_v4_vectors_follow_the_documented_layout() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../schema/fixtures/signing/approval-start-v4.json");
    let text = std::fs::read_to_string(path).expect("fixture is committed");
    let doc: serde_json::Value = serde_json::from_str(&text).expect("fixture is JSON");
    let cases = doc["cases"].as_array().expect("cases");
    assert!(cases.len() >= 5);
    let mut seen_grant = false;
    for c in cases {
        let s = |k: &str| c[k].as_str().expect(k).to_string();
        let flag = |v: &serde_json::Value| if v.as_bool().expect("flag") { "1" } else { "0" };
        let settings: Vec<(String, String)> = c["settings"]
            .as_array()
            .expect("settings")
            .iter()
            .map(|p| {
                (
                    p[0].as_str().expect("id").into(),
                    p[1].as_str().expect("value").into(),
                )
            })
            .collect();
        let options: Vec<String> = c["options"]
            .as_array()
            .expect("options")
            .iter()
            .map(|o| o.as_str().expect("option").into())
            .collect();
        let g = &c["grant"];
        let max = g["max_running"].as_u64().expect("max_running");
        seen_grant |= max > 0;
        if !options.is_empty() {
            assert_eq!(max, 0, "a start that skips permissions is never granted");
        }
        let detail = s("detail");
        let mut out = b"agora-approval-start-v4\0".to_vec();
        for part in [
            s("node"),
            s("origin_node"),
            s("origin_agent"),
            s("approval_id"),
            flag(&c["approved"]).into(),
            pairs(&settings),
            list(&options),
            max.to_string(),
            flag(&g["any_kind"]).into(),
            flag(&g["below_folder"]).into(),
            flag(&g["chain"]).into(),
            g["until_ms"].as_u64().expect("until_ms").to_string(),
            hex(&sha256(detail.as_bytes())),
        ] {
            out.extend_from_slice(f(&part).as_bytes());
        }
        assert_eq!(hex(&out), s("hex"), "{}", s("name"));
    }
    assert!(seen_grant, "a case with a standing grant");
}
