//! ADR-0123: `AgentKind.provider`, `AgentStarted.launch_settings` and `AgentSnapshot.model`, `effort`, `config` and `launch_settings`
//! are additive, and `LaunchSetting` moved from commands.proto to common.proto without changing its name or wire. The published
//! fixtures (written before the fields existed, never edited) still decode with the new fields empty, and the new fields round-trip.
#![allow(clippy::unwrap_used)]

use agora_protocol::v1;
use prost::Message;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../schema/fixtures/v1")
            .join(format!("agora.v1.{name}.bin")),
    )
    .unwrap()
}

#[test]
fn old_bytes_decode_with_the_new_fields_empty() {
    let k = v1::AgentKind::decode(fixture("AgentKind").as_slice()).unwrap();
    assert!(k.provider.is_empty());
    let s = v1::AgentStarted::decode(fixture("AgentStarted").as_slice()).unwrap();
    assert!(s.launch_settings.is_empty());
    let a = v1::AgentSnapshot::decode(fixture("AgentSnapshot").as_slice()).unwrap();
    assert!(
        a.model.is_empty()
            && a.effort.is_empty()
            && a.config.is_empty()
            && a.launch_settings.is_empty()
    );
    // The moved message reads its own published bytes as before.
    let l = v1::LaunchSetting::decode(fixture("LaunchSetting").as_slice()).unwrap();
    assert_eq!(l.id, "x");
}

#[test]
fn the_new_fields_round_trip() {
    let setting = |id: &str, value: &str| v1::LaunchSetting {
        id: id.into(),
        value: value.into(),
    };
    let k = v1::AgentKind {
        kind: "claude-code".into(),
        provider: "anthropic".into(),
        ..Default::default()
    };
    assert_eq!(
        v1::AgentKind::decode(k.encode_to_vec().as_slice())
            .unwrap()
            .provider,
        "anthropic"
    );
    let s = v1::AgentStarted {
        agent_id: "agent-1".into(),
        launch_settings: vec![setting("model", "opus"), setting("effort", "high")],
        ..Default::default()
    };
    assert_eq!(
        v1::AgentStarted::decode(s.encode_to_vec().as_slice()).unwrap(),
        s
    );
    let a = v1::AgentSnapshot {
        agent_id: "n/agent-1".into(),
        model: "anthropic/claude-opus".into(),
        effort: "high".into(),
        config: vec![v1::SessionConfigOption {
            id: "effort".into(),
            current: "high".into(),
            ..Default::default()
        }],
        launch_settings: vec![setting("model", "opus")],
        ..Default::default()
    };
    assert_eq!(
        v1::AgentSnapshot::decode(a.encode_to_vec().as_slice()).unwrap(),
        a
    );
}
