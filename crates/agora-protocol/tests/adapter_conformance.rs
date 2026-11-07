//! Adapter conformance: a minimal adapter written in Python (adapters/examples/minimal-python) must play the
//! transcript in schema/fixtures/adapter/ exactly, and every payload in the transcript must match the schema.
//! Skipped (with a message) if no Python interpreter is installed.
#![allow(clippy::expect_used)] // expect is allowed in tests

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use agora_protocol::jsonrpc::{method, parse_line, Message};
use prost_reflect::{DescriptorPool, DynamicMessage};
use serde_json::Value;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn transcript() -> Vec<(Value, Vec<Value>)> {
    let text =
        std::fs::read_to_string(root().join("schema/fixtures/adapter/minimal.transcript.json"))
            .expect("transcript");
    let steps: Vec<Value> = serde_json::from_str(&text).expect("json");
    steps
        .into_iter()
        .map(|s| {
            (
                s["send"].clone(),
                s["recv"].as_array().cloned().unwrap_or_default(),
            )
        })
        .collect()
}

fn validate(pool: &DescriptorPool, name: &str, value: &Value) {
    let desc = pool
        .get_message_by_name(&format!("agora.v1.{name}"))
        .expect("message in schema");
    DynamicMessage::deserialize(desc, value.clone())
        .unwrap_or_else(|e| panic!("{name} does not match the schema: {e} ({value})"));
}

#[test]
fn transcript_payloads_match_the_schema() {
    let pool = DescriptorPool::decode(agora_protocol::FILE_DESCRIPTOR_SET).expect("pool");
    let mut result_types: HashMap<String, &'static str> = HashMap::new();
    for (send, recv) in transcript() {
        for msg in std::iter::once(&send).chain(recv.iter()) {
            match parse_line(&msg.to_string()).expect("valid JSON-RPC") {
                Message::Request {
                    id,
                    method: name,
                    params,
                } => {
                    if let Some(info) = method(&name) {
                        validate(&pool, info.params, &params);
                        if let Some(r) = info.result {
                            result_types.insert(id.to_string(), r);
                        }
                    }
                }
                Message::Notification {
                    method: name,
                    params,
                } => {
                    if let Some(info) = method(&name) {
                        validate(&pool, info.params, &params);
                    }
                }
                Message::Response { id, result } => {
                    if let Some(t) = result_types.get(&id.to_string()) {
                        validate(&pool, t, &result);
                    }
                }
                Message::ErrorResponse { .. } => {}
            }
        }
    }
}

fn python() -> Option<&'static str> {
    ["python3", "python"].into_iter().find(|p| {
        Command::new(p)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
}

#[test]
fn minimal_python_adapter_plays_the_transcript() {
    let Some(py) = python() else {
        eprintln!("skipped: no python interpreter found");
        return;
    };
    let script = root().join("adapters/examples/minimal-python/adapter.py");
    let mut child = Command::new(py)
        .arg("-I")
        .arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn adapter");
    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    for (send, expected) in transcript() {
        writeln!(stdin, "{send}").expect("write");
        stdin.flush().expect("flush");
        for want in expected {
            let line = rx
                .recv_timeout(Duration::from_secs(10))
                .unwrap_or_else(|_| {
                    let _ = child.kill();
                    panic!("adapter did not answer {send}")
                });
            let got: Value = serde_json::from_str(&line).expect("adapter output is JSON");
            assert_eq!(got, want, "after sending {send}");
        }
    }
    drop(stdin);
    assert!(
        child.wait().expect("wait").success(),
        "adapter should exit cleanly after shutdown"
    );
    assert!(
        rx.recv_timeout(Duration::from_millis(200)).is_err(),
        "adapter sent unexpected extra output"
    );
}
