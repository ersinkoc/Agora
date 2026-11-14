//! Turns what happens in the hub into notifications on the owner's delivery targets (`deliver`): an approval is waiting, an
//! agent is blocked or done, a machine went offline. It watches the hub's live events from the moment it starts (history is
//! never replayed as news), routes each through the rules in `notify` (dedupe), renders the text from `locales/` and sends it
//! from a short-lived thread so a slow target never holds up anything else.

use std::sync::Arc;

use agora_i18n::{Arg, Catalog};
use agora_protocol::v1::{event::Body, AgentState, Event};

use crate::deliver::{self, Target};
use crate::hub::Hub;
use crate::notify::{Channel, Context, Notice, Outcome, Prefs, Router, Trigger};

/// The notice an event is worth, with the name to show in its text.
pub fn notice_for(e: &Event) -> Option<(Notice, String)> {
    let (trigger, subject, name) = match e.body.as_ref()? {
        Body::ApprovalRequested(a) => (
            Trigger::ApprovalRequested,
            format!("{}/{}", e.producer_id, a.approval_id),
            format!("{}/{}", e.producer_id, a.agent_id),
        ),
        Body::AgentStateChanged(s) => {
            let trigger = match AgentState::try_from(s.state) {
                Ok(AgentState::Blocked) => Trigger::AgentBlocked,
                Ok(AgentState::Done) => Trigger::AgentDone,
                _ => return None,
            };
            let who = format!("{}/{}", e.producer_id, s.agent_id);
            (trigger, who.clone(), who)
        }
        Body::NodeOffline(n) => (Trigger::NodeOffline, n.node_id.clone(), n.node_id.clone()),
        _ => return None,
    };
    Some((
        Notice {
            trigger,
            subject,
            at_ms: e.at_ms,
        },
        name,
    ))
}

/// The text of a notice in `catalog`'s language.
pub fn render(catalog: &Catalog, n: &Notice, name: &str) -> String {
    let arg = match n.trigger {
        Trigger::NodeOffline => "machine",
        _ => "agent",
    };
    catalog
        .render(n.trigger.key(), &[(arg, Arg::from(name))])
        .unwrap_or_else(|| n.trigger.key().to_string())
}

/// Where the owner opens a waiting approval (ADR-0046): the hub's public address and the approval's place in the page. A
/// notification only points at it; allowing still takes the device key.
pub fn approval_link(public_url: &str, subject: &str) -> String {
    page_link(public_url, "approvals", subject)
}

/// Where the owner opens an agent that waits or finished (ADR-0067 rung 2): its own view in the page, on a phone too.
pub fn agent_link(public_url: &str, subject: &str) -> String {
    page_link(public_url, "agent", subject)
}

/// `<public_url>/#/<route>/<subject>`, the subject percent-encoded as one path part.
fn page_link(public_url: &str, route: &str, subject: &str) -> String {
    let encoded: String = subject
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => c.to_string(),
            other => other
                .to_string()
                .bytes()
                .map(|b| format!("%{b:02X}"))
                .collect(),
        })
        .collect();
    format!("{}/#/{route}/{encoded}", public_url.trim_end_matches('/'))
}

/// Watches `hub` and delivers to `targets` until the hub is gone. Returns at once when there is nothing to deliver to.
pub fn spawn(hub: Arc<Hub>, targets: Vec<Target>, catalog: Catalog, public_url: Option<String>) {
    if targets.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        let (_, pos) = hub.view();
        let Ok(sub) = hub.subscribe(Some(pos)) else {
            return;
        };
        let (prefs, ctx) = (Prefs::webhooks_only(), Context::default());
        let mut router = Router::new();
        let title = catalog.render("notify-title", &[]).unwrap_or_default();
        while let Ok(d) = sub.live.recv() {
            let Some((notice, name)) = notice_for(&d.event) else {
                continue;
            };
            router.prune(notice.at_ms);
            let Outcome::Deliver(channels) = router.route(&prefs, &ctx, &notice) else {
                continue;
            };
            if !channels.contains(&Channel::Webhook) {
                continue;
            }
            let mut text = render(&catalog, &notice, &name);
            match (notice.trigger, public_url.as_deref()) {
                (Trigger::ApprovalRequested, Some(base)) => {
                    text = format!("{text}\n{}", approval_link(base, &notice.subject));
                }
                (Trigger::AgentBlocked | Trigger::AgentDone, Some(base)) => {
                    text = format!("{text}\n{}", agent_link(base, &notice.subject));
                }
                _ => {}
            }
            for t in targets.iter().cloned() {
                let (title, text, priority) =
                    (title.clone(), text.clone(), notice.trigger.priority());
                // A target that is down is not the hub's problem; the next notice tries again.
                std::thread::spawn(move || {
                    let _ = deliver::send(&t.url, &deliver::request(&t, &title, &text, priority));
                });
            }
        }
    });
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use agora_protocol::v1::{AgentStateChanged, ApprovalRequested, NodeOffline};

    fn ev(body: Body) -> Event {
        Event {
            producer_id: "laptop".into(),
            seq: 1,
            at_ms: 5,
            body: Some(body),
        }
    }

    #[test]
    fn an_approval_link_points_into_the_page_and_keeps_the_address_safe() {
        assert_eq!(
            approval_link("https://hub.example/", "laptop/p 1"),
            "https://hub.example/#/approvals/laptop%2Fp%201"
        );
        assert_eq!(
            agent_link("https://hub.example", "laptop/agent-1"),
            "https://hub.example/#/agent/laptop%2Fagent-1"
        );
    }

    #[test]
    fn only_events_that_need_a_person_become_notices() {
        let blocked = ev(Body::AgentStateChanged(AgentStateChanged {
            agent_id: "a1".into(),
            state: AgentState::Blocked as i32,
            reported: true,
        }));
        let (n, name) = notice_for(&blocked).unwrap();
        assert_eq!(
            (n.trigger, name.as_str()),
            (Trigger::AgentBlocked, "laptop/a1")
        );
        let working = ev(Body::AgentStateChanged(AgentStateChanged {
            agent_id: "a1".into(),
            state: AgentState::Working as i32,
            reported: true,
        }));
        assert!(notice_for(&working).is_none());
        let approval = ev(Body::ApprovalRequested(ApprovalRequested {
            approval_id: "p1".into(),
            agent_id: "a1".into(),
            ..Default::default()
        }));
        assert_eq!(
            notice_for(&approval).unwrap().0.trigger,
            Trigger::ApprovalRequested
        );
        let off = ev(Body::NodeOffline(NodeOffline {
            node_id: "laptop".into(),
        }));
        assert_eq!(notice_for(&off).unwrap().0.trigger, Trigger::NodeOffline);
    }

    #[test]
    fn text_comes_from_the_catalog_in_the_chosen_language() {
        let off = notice_for(&ev(Body::NodeOffline(NodeOffline {
            node_id: "laptop".into(),
        })))
        .unwrap();
        assert_eq!(
            render(&Catalog::english(), &off.0, &off.1),
            "laptop went offline."
        );
        assert!(render(&Catalog::turkish(), &off.0, &off.1).contains("laptop"));
    }

    #[test]
    fn a_machine_going_offline_reaches_the_target_once() {
        use std::io::{Read, Write};
        use std::time::{Duration, Instant};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let hub =
            Arc::new(Hub::open(crate::store::SqliteStore::open_in_memory().unwrap()).unwrap());
        let (targets, _) = deliver::parse_targets(&format!("webhook http://127.0.0.1:{port}/hook"));
        spawn(Arc::clone(&hub), targets, Catalog::english(), None);
        let t0 = Instant::now();
        while hub.subscriber_count() == 0 {
            assert!(
                t0.elapsed() < Duration::from_secs(5),
                "the notifier never subscribed"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        // The same news twice inside the dedupe window is sent once.
        hub.node_disconnected("laptop", 1_000).unwrap();
        hub.node_disconnected("laptop", 1_001).unwrap();
        let (mut s, _) = l.accept().unwrap();
        let mut got = vec![0u8; 2048];
        let n = s.read(&mut got).unwrap();
        s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .unwrap();
        let text = String::from_utf8_lossy(&got[..n]).to_string();
        assert!(
            text.starts_with("POST /hook") && text.contains("laptop went offline."),
            "{text}"
        );
        l.set_nonblocking(true).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        assert!(l.accept().is_err(), "a repeat was delivered");
    }
}
