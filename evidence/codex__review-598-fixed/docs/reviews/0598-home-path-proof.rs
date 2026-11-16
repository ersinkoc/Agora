// Review-only: include at the end of crates/agora-hub/tests/server.rs.
// Reuses its real TCP/WebSocket hub/node helpers, not a copy of the sanitizer.
struct PrivacyProbeAuth {
    role: agora_hub::users::Role,
    scoped: bool,
}

impl ClientAuth for PrivacyProbeAuth {
    fn check(&self, token: &str) -> bool {
        token == "good-token"
    }
    fn access(&self, token: &str) -> Option<agora_hub::users::Access> {
        self.check(token).then(|| agora_hub::users::Access {
            role: self.role,
            user: "reader".into(),
            projects: self.scoped.then(|| ["n1/mine".to_string()].into()),
            session: String::new(),
        })
    }
}

async fn probe_online_binary(client: &mut Conn<TcpStream>) -> String {
    loop {
        let fr = recv(client).await;
        assert_eq!(fr.kind, kinds::POSITIONED_EVENT);
        let ev = v1::PositionedEvent::decode(fr.payload.as_slice()).unwrap();
        if let Some(Body::NodeOnline(n)) = ev.event.and_then(|e| e.body) {
            return n.binary;
        }
    }
}

#[tokio::test]
async fn review_598_home_username_does_not_reach_nonowners() {
    use agora_hub::users::Role;
    let mut leaks = Vec::new();
    let mut observations = 0;
    for path in [
        "/home/alice/agora",
        "/Users/alice/agora",
        r"C:\Users\alice\agora.exe",
    ] {
        for (role, scoped) in [
            (Role::Owner, false),
            (Role::Member, false),
            (Role::Member, true),
            (Role::Viewer, false),
            (Role::Viewer, true),
        ] {
            let st = Arc::new(ServerState {
                hub: Arc::new(Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap()),
                support: support(),
                ca: None,
                lockout: Arc::new(Lockout::new()),
                auth: Box::new(PrivacyProbeAuth { role, scoped }),
                assets: None,
                sessions: None,
            });
            let addr = start(Arc::clone(&st)).await;
            let (mut node, task) = node_conn(&st, "n1").await;
            handshake(&mut node).await;
            let online = || {
                Body::NodeOnline(v1::NodeOnline {
                    node_id: "n1".into(),
                    binary: path.into(),
                    ..Default::default()
                })
            };
            node.send(&f(STREAM_EVENTS, kinds::EVENT, event("n1", 1, online())))
                .await
                .unwrap();
            next_ack(&mut node).await;
            let mut client = dial(addr, Some("good-token")).await;
            client
                .send(&f(
                    STREAM_EVENTS,
                    kinds::SUBSCRIBE,
                    v1::Subscribe::default().encode_to_vec(),
                ))
                .await
                .unwrap();
            let fr = recv(&mut client).await;
            assert_eq!(fr.kind, kinds::SNAPSHOT);
            let snap = v1::Snapshot::decode(fr.payload.as_slice()).unwrap();
            let mut seen = vec![("snapshot", snap.nodes[0].binary.clone())];
            node.send(&f(STREAM_EVENTS, kinds::EVENT, event("n1", 2, online())))
                .await
                .unwrap();
            next_ack(&mut node).await;
            seen.push(("live", probe_online_binary(&mut client).await));
            if !scoped {
                let mut replay = dial(addr, Some("good-token")).await;
                replay
                    .send(&f(
                        STREAM_EVENTS,
                        kinds::SUBSCRIBE,
                        v1::Subscribe { after_pos: Some(0) }.encode_to_vec(),
                    ))
                    .await
                    .unwrap();
                seen.push(("backlog", probe_online_binary(&mut replay).await));
            }
            for (route, shown) in seen {
                observations += 1;
                println!("{role:?} scoped={scoped} {route} {path} -> {shown}");
                if role == Role::Owner {
                    assert_eq!(shown, path, "owner full-path control");
                } else if shown.contains("alice") {
                    leaks.push(format!("{role:?} scoped={scoped} {route}: {shown}"));
                }
            }
            task.abort();
        }
    }
    assert_eq!(observations, 39, "all routes and owner controls exercised");
    assert!(
        leaks.is_empty(),
        "{} of 30 nonowner deliveries expose the fixture username: {leaks:?}",
        leaks.len()
    );
}
