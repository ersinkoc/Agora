//! ADR-0131 Amendment A: `BoardCall.from_run_id` / `request_seq` and `TaskSnapshot.pickup_eligible` / `picked_up` are additive. The
//! published fixtures (written before the fields existed, never edited) decode with nothing about pickup, and the new fields round-trip.
#![allow(clippy::unwrap_used)]

use agora_protocol::v1;
use prost::Message;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../schema/fixtures/v1/agora.v1.{name}.bin")),
    )
    .unwrap()
}

#[test]
fn older_board_calls_and_tasks_say_nothing_about_pickup_and_the_new_fields_round_trip() {
    let call = v1::BoardCall::decode(fixture("BoardCall").as_slice()).unwrap();
    assert!(call.from_run_id.is_empty());
    assert_eq!(call.request_seq, 0);
    let task = v1::TaskSnapshot::decode(fixture("TaskSnapshot").as_slice()).unwrap();
    assert!(!task.pickup_eligible);
    assert!(task.picked_up.is_none());

    let call = v1::BoardCall {
        project_id: "p".into(),
        tool: "task_take_next".into(),
        arguments_json: "{}".into(),
        from_agent: "a".into(),
        from_run_id: vec![7; 16],
        request_seq: 3,
        ..Default::default()
    };
    assert_eq!(
        v1::BoardCall::decode(call.encode_to_vec().as_slice()).unwrap(),
        call
    );
    let picked = v1::TaskPickedUp {
        project_id: "p".into(),
        task_id: "t1".into(),
        node_id: "n".into(),
        agent_id: "a".into(),
        run_id: vec![7; 16],
        at_ms: 5,
        rule: "take_next".into(),
    };
    let task = v1::TaskSnapshot {
        task_id: "t1".into(),
        project_id: "p".into(),
        pickup_eligible: true,
        picked_up: Some(picked.clone()),
        ..Default::default()
    };
    assert_eq!(
        v1::TaskSnapshot::decode(task.encode_to_vec().as_slice()).unwrap(),
        task
    );
    for body in [
        v1::event::Body::TaskPickupEligible(v1::TaskPickupEligible {
            project_id: "p".into(),
            task_id: "t1".into(),
            eligible: true,
        }),
        v1::event::Body::TaskPickedUp(picked),
    ] {
        let event = v1::Event {
            producer_id: "hub".into(),
            seq: 1,
            at_ms: 2,
            body: Some(body),
        };
        assert_eq!(
            v1::Event::decode(event.encode_to_vec().as_slice()).unwrap(),
            event
        );
    }
}
