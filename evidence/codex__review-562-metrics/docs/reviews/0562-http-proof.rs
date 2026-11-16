// Append to tests/server.rs in an isolated review worktree.
#[tokio::test]
async fn review_562_disabled_metrics_with_app_assets_and_head_when_enabled() {
    use agora_hub::signin::{HubSessions, SignIn};
    use agora_hub::web::EmbeddedAssets;
    let hub = Arc::new(Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap());
    let sessions = Arc::new(HubSessions::new(Arc::clone(&hub)));
    let st = Arc::new(ServerState {
        hub: Arc::clone(&hub),
        support: support(),
        ca: None,
        lockout: Arc::new(Lockout::new()),
        auth: Box::new(HubSessionsAuth(Arc::clone(&sessions))),
        assets: Some(Arc::new(EmbeddedAssets(&[(
            "index.html",
            b"<html>review-shell</html>",
        )]))),
        sessions: Some(sessions),
    });
    let addr = start(st).await;
    let now = server::now_ms();
    let owner = SignIn::new(hub.store(), Arc::new(Lockout::new()))
        .start_session(now)
        .unwrap();
    for method in ["GET", "HEAD"] {
        for token in [None, Some(owner.as_str())] {
            let unknown = raw(addr, method, "/api/nothing", token).await;
            assert!(unknown.starts_with("HTTP/1.1 200"));
            for path in ["/api/metrics", "/api/metrics?x=1"] {
                assert_eq!(
                    raw(addr, method, path, token).await,
                    unknown,
                    "{method} {path}"
                );
            }
        }
    }
    hub.enable_metrics();
    for (token, status) in [(None, "401"), (Some(owner.as_str()), "200")] {
        let get = raw(addr, "GET", "/api/metrics", token).await;
        let head = raw(addr, "HEAD", "/api/metrics", token).await;
        let (get_headers, _) = get.split_once("\r\n\r\n").unwrap();
        let (head_headers, body) = head.split_once("\r\n\r\n").unwrap();
        assert!(get_headers.starts_with(&format!("HTTP/1.1 {status}")));
        assert_eq!(head_headers, get_headers);
        assert!(body.is_empty());
    }
}
