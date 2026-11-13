# An already-ended survivor cannot clear Stop it (#437/#439)

Reviewed heads: node `fbe0db06026f3da6a7945e0393f55bc1cb6e8100`, client
`03f2f017d368b1a7373b39c8910821b4cb929252` (identical node code).

After a node restart announces `still_running=true`, its old process may finish before the person
presses **Stop it**. The node only enters `stop_survivor` while `Entry::still_running()` remains true.
Once the process has ended the command instead returns `agent.not_found`, emits no updated interruption
event, and leaves the client showing Stop it with Resume/Start again hidden. The node does not periodically
announce the death of these old processes. An explicit reconnect/resync or node restart is required to
refresh that stale state.

Append this proof to `crates/agora-node/tests/session_resume.rs` at the reviewed head, run
`cargo test -p agora-node --test session_resume review_a_survivor_that_ended_after_announcement`, then
remove the appended proof. It uses that file's existing fake-agent harness and only ends the matching
fake process it created. No production node, real agent, token or model call is involved.

```rust
#[test]
fn review_a_survivor_that_ended_after_announcement_can_be_cleared_by_stop() {
    if !supported() || agora_platform::process_identity(std::process::id()).is_err() { return; }
    let mut e = Env::new("review-dead-survivor");
    let probe = e.launch("probe");
    e.wait("the owned probe process in the ledger", |e| e.node().ledger.get(&probe).is_some_and(|x| x.process().is_some()));
    let process = e.node().ledger.get(&probe).unwrap().process().unwrap();
    e.node.take().unwrap().crash();
    e.start();
    assert!(e.interrupted().iter().any(|i| i.agent_id == probe && i.still_running));
    // End only this test's fake process after the hub would have seen still_running=true.
    agora_platform::kill_process_if_same(&process).unwrap();
    e.wait("the owned probe to end", |_| !agora_platform::process_still_running(&process));
    let result=stop(&e, &probe, false);
    assert!(result.is_ok(), "Stop it cannot clear an already-ended survivor: {result:?}");
}
```

Observed result: the last assertion fails with
`Stop it cannot clear an already-ended survivor: Err("agent.not_found")`.
The unchanged six restart/resume integration tests pass; session-file tests (3), ledger tests (8),
process-identity guard (1), web resume regressions (4), and TUI guards/refusals (5) also pass.

The node fallback should accept a known interrupted entry whose old process is already gone, retain
the existing identity checks, clear its process record and emit `still_running=false`. Unknown and
non-interrupted agents should remain refused. The finding and smallest suggested fix are on #437;
#439's integration approval waits for that event fix. No product source was changed for this review.
