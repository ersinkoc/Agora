// Appended by the review runner to a temporary copy of session_resume.rs.
// Uses the existing real-node/fake-agent harness; no production file changes.

#[test]
fn codex_review635_unknown_done_and_invalid_policy_do_not_receive_a_prompt() {
    assert!(
        supported(),
        "native PTY/local socket evidence requires platform support"
    );
    for (name, state, policy) in [
        ("unknown", v1::AgentState::Unspecified, None),
        ("done", v1::AgentState::Done, None),
        (
            "typo",
            v1::AgentState::Working,
            Some("resume.continue = ON\n"),
        ),
    ] {
        let mut e = Env::new(&format!("codex-review635-{name}"));
        let id = e.launch("carry");
        e.wait("the session", |e| !e.session(&id).is_empty());
        // The original fixture helper asserts every unlisted state is unknown;
        // Done is actually persisted as "done", so observe the real event here.
        e.node().ledger.observe(
            &v1::event::Body::AgentStateChanged(v1::AgentStateChanged {
                agent_id: id.clone(),
                state: state as i32,
                reported: false,
            }),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
        );
        assert_eq!(
            e.node().ledger.get(&id).unwrap().last_state,
            match state {
                v1::AgentState::Done => "done",
                v1::AgentState::Working => "working",
                _ => "unknown",
            }
        );
        if let Some(policy) = policy {
            std::fs::write(e.root.join("data/policy"), policy).unwrap();
        }
        e.resume_all = true;
        e.start();
        e.wait("plain resume probe", |e| !e.probes("resume-").is_empty());
        assert!(e.probes("carry-").is_empty(), "{name}: prompt was sent");
        assert_eq!(e.node().ledger.get(&id).unwrap().continue_sent_ms, 0);
    }
}

#[test]
fn codex_review635_sentence_metacharacters_stay_one_literal_argument() {
    assert!(supported());
    let mut e = Env::new("codex-review635-literal");
    let sentinel = e.root.join("must-not-exist");
    let sentence = format!(
        "Check first; $(touch '{}') `touch '{}'` && echo quoted text",
        sentinel.display(),
        sentinel.display()
    );
    let manifest = e.root.join("manifests/carry.toml");
    let original = std::fs::read_to_string(&manifest).unwrap();
    let modified = original.replace(
        &format!("continue = \"{SENTENCE}\""),
        &format!("continue = {}", serde_json::to_string(&sentence).unwrap()),
    );
    assert_ne!(original, modified);
    std::fs::write(&manifest, modified).unwrap();
    let id = e.launch("carry");
    e.wait("session", |e| !e.session(&id).is_empty());
    let old_session = e.session(&id);
    e.set_state(&id, v1::AgentState::Working);
    e.resume_all = true;
    e.start();
    e.wait("carry probe", |e| !e.probes("carry-").is_empty());
    let probes = e.probes("carry-");
    assert_eq!(probes.len(), 1);
    assert_eq!(
        &probes[0][probes[0].len() - 3..],
        ["--resume", old_session.as_str(), sentence.as_str()]
    );
    assert_eq!(e.session(&id), old_session);
    assert!(
        !sentinel.exists(),
        "shell syntax executed instead of literal argv"
    );
    assert!(e.node().ledger.get(&id).unwrap().continue_sent_ms > 0);
}

#[test]
fn codex_review635_manual_start_again_never_carries_the_prompt() {
    assert!(supported());
    let mut e = Env::new("codex-review635-start-again");
    let id = e.launch("carry");
    e.wait("session", |e| !e.session(&id).is_empty());
    let old_session = e.session(&id);
    e.set_state(&id, v1::AgentState::Working);
    e.start();
    e.resume(&id, true).unwrap();
    e.wait("new session", |e| {
        !e.session(&id).is_empty() && e.session(&id) != old_session
    });
    e.wait("fresh launch probe", |e| e.probes("start-").len() == 2);
    assert!(e.probes("carry-").is_empty());
    assert!(e.probes("resume-").is_empty());
    assert_eq!(e.node().ledger.get(&id).unwrap().continue_sent_ms, 0);
}
