// Review-only inclusion in the actual server integration test.
#[tokio::test]
async fn review629_fixed_checkpoints_heal_and_safe_tail_replays() {
    let store = SqliteStore::open_in_memory().unwrap();
    seed_legacy(&store, true);
    // A legacy client may already have consumed clean events AFTER the forgeries.
    store
        .append("n1", 3, 7, &event("n1", 3, working("a")))
        .unwrap();
    store
        .append(
            "n2",
            5,
            8,
            &event(
                "n2",
                5,
                Body::NodeTrustedDevices(v1::NodeTrustedDevices::default()),
            ),
        )
        .unwrap();
    let rows = store.events_after(0, 100).unwrap();
    let mut prefix = agora_hub::projection::View::default();
    for row in rows.iter().take(3) {
        prefix.apply(&v1::Event::decode(row.payload.as_slice()).unwrap());
    }
    let hub = Arc::new(Hub::open_at(store, 100).unwrap());
    let (view, end) = hub.view();
    assert_eq!(
        view.distrusted_until, 8,
        "barrier is log end, not last bad record6"
    );
    assert_eq!(end, 12);
    let st = Arc::new(ServerState {
        hub,
        support: support(),
        ca: None,
        lockout: Arc::new(Lockout::new()),
        auth: Box::new(Tokens),
        assets: None,
        sessions: None,
    });
    let addr = start(Arc::clone(&st)).await;
    let (_, fresh) = subscribe_after(addr, None).await;
    assert_eq!(fresh.kind, kinds::SNAPSHOT);
    let snap = v1::Snapshot::decode(fresh.payload.as_slice()).unwrap();
    assert!(snap
        .agents
        .iter()
        .any(|a| a.agent_id == "n1/a" && a.running));
    assert_eq!(
        snap.nodes
            .iter()
            .find(|n| n.node_id == "n1")
            .unwrap()
            .agents_running,
        0
    );
    assert_eq!(
        snap.checkouts
            .iter()
            .find(|c| c.checkout_id == "n2/forged")
            .unwrap()
            .node_id,
        "n2"
    );
    let output = std::env::var_os("REVIEW_629_FRAMES").map(std::path::PathBuf::from);
    if let Some(dir) = &output {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("prefix.bin"),
            server::snapshot_of(&prefix, 3).encode_to_vec(),
        )
        .unwrap();
        std::fs::write(dir.join("fresh.bin"), &fresh.payload).unwrap();
        for (i, row) in rows.iter().skip(3).enumerate() {
            let p = v1::PositionedEvent {
                pos: row.pos,
                event: Some(v1::Event::decode(row.payload.as_slice()).unwrap()),
            };
            std::fs::write(dir.join(format!("legacy-{i}.bin")), p.encode_to_vec()).unwrap();
        }
    }
    for after in [0, 3, 4, 6, 7, 8, end + 1] {
        let (_, resumed) = subscribe_after(addr, Some(after)).await;
        assert_eq!(resumed.kind, kinds::SNAPSHOT, "checkpoint{after}");
        assert_eq!(resumed.payload, fresh.payload, "checkpoint{after}");
        if after == 8 {
            if let Some(dir) = &output {
                std::fs::write(dir.join("resumed.bin"), &resumed.payload).unwrap();
            }
        }
    }
    let (_, tail) = subscribe_after(addr, Some(9)).await;
    assert_eq!(tail.kind, kinds::POSITIONED_EVENT);
    let tail = v1::PositionedEvent::decode(tail.payload.as_slice()).unwrap();
    assert_eq!(tail.pos, 10);
    assert_eq!(tail.event.unwrap().producer_id, "hub");
    let empty = st.hub.subscribe(Some(end)).unwrap();
    assert!(empty.snapshot.is_none() && empty.backlog.is_empty());
    // Published stored bytes have not been edited to make the projection safe.
    assert_eq!(st.hub.store().events_after(0, 8).unwrap(), rows);
    println!("7 unsafe/beyond-end checkpoints equal fresh snapshot; clean tail replays; at-end resume empty; 8 stored records byte-identical");
}

#[test]
fn review629_fixed_second_open_reestablishes_barrier_and_clean_history_has_none() {
    let dir = std::env::temp_dir().join(format!(
        "0629-reopen-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    for forged in [true, false] {
        let db = dir.join(if forged { "legacy.db" } else { "clean.db" });
        let store = SqliteStore::open(&db).unwrap();
        seed_legacy(&store, forged);
        let first = Hub::open_at(store, 100).unwrap();
        let first_end = first.view().1;
        assert_eq!(first.view().0.distrusted_until, if forged { 6 } else { 0 });
        let bytes = first.store().events_after(0, 100).unwrap();
        drop(first);
        let second = Hub::open_at(SqliteStore::open(&db).unwrap(), 200).unwrap();
        assert_eq!(second.view().1, first_end);
        assert_eq!(second.store().events_after(0, 100).unwrap(), bytes);
        assert_eq!(
            second.view().0.distrusted_until,
            if forged { first_end } else { 0 }
        );
        let resumed = second.subscribe(Some(first_end)).unwrap();
        assert_eq!(
            resumed.snapshot.is_some(),
            forged,
            "second open forged={forged}"
        );
        if !forged {
            assert!(second.subscribe(Some(0)).unwrap().snapshot.is_none());
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
    println!("Second disk-backed open refreshes legacy barrier; clean history replays; stored bytes unchanged");
}
