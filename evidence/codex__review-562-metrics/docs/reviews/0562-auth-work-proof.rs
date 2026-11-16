// Append to hub.rs in an isolated review worktree; cfg(test) only.
#[cfg(test)]
mod review_562_auth_before_work {
    use super::Hub;
    use crate::enroll::Lockout;
    use crate::signin::{HubSessions, SignIn};
    use crate::store::SqliteStore;
    use crate::users::{Role, Users};
    use crate::web::SessionApi;
    use std::sync::{mpsc, Arc};
    use std::time::Duration;

    #[test]
    fn rejected_metrics_requests_do_not_wait_for_the_hub_lock() {
        let hub = Arc::new(Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap());
        hub.enable_metrics();
        let api = Arc::new(HubSessions::new(Arc::clone(&hub)));
        let now = crate::server::now_ms();
        let signin = SignIn::new(hub.store(), Arc::new(Lockout::new()));
        let revoked = signin.start_session(now).unwrap();
        let owner = signin.start_session(now).unwrap();
        signin.revoke_others(&owner).unwrap();
        let users = Users::new(hub.store());
        let viewer = users.add("review-viewer", Role::Viewer, now).unwrap();
        let member = users.add("review-member", Role::Member, now).unwrap();
        let viewer = signin.start_session_for(Some(&viewer.id), now).unwrap();
        let member = signin.start_session_for(Some(&member.id), now).unwrap();
        let expired = signin.start_session(1).unwrap();

        // Auth may query its own session store, but rejected scrapes must not
        // reach the metrics work protected by this deliberately held mutex.
        let held = hub.lock();
        let (send, receive) = mpsc::channel();
        let api2 = Arc::clone(&api);
        let worker = std::thread::spawn(move || {
            for (label, token, status) in [
                ("missing", None, 401),
                ("invalid", Some("not-a-token".to_string()), 401),
                ("revoked", Some(revoked), 401),
                ("expired", Some(expired), 401),
                ("viewer", Some(viewer), 403),
                ("member", Some(member), 403),
            ] {
                let got = api2.metrics(token.as_deref());
                assert_eq!(got, (status, String::new()), "{label}");
                send.send(label).unwrap();
            }
        });
        for label in [
            "missing", "invalid", "revoked", "expired", "viewer", "member",
        ] {
            assert_eq!(
                receive
                    .recv_timeout(Duration::from_secs(1))
                    .expect("rejected scrape reached blocked metrics work"),
                label
            );
        }
        worker.join().unwrap();

        // Positive control: an owner reaches that same locked work and waits.
        let (send, receive) = mpsc::channel();
        let worker = std::thread::spawn(move || send.send(api.metrics(Some(&owner))).unwrap());
        assert!(receive.recv_timeout(Duration::from_millis(100)).is_err());
        drop(held);
        let (status, body) = receive.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(status, 200);
        assert!(body.contains("agora_hub_events_ingested_total 0\n"));
        worker.join().unwrap();
    }
}
