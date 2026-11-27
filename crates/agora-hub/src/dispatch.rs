//! What the hub does with one client command (architecture review R9): the decision, apart from the transport that carries it.
//!
//! [`route`] is pure: it reads who asks ([`Access`]), what they may see ([`Scope`]) and the request, and says whether the command is
//! refused, answered by the hub itself, or forwarded to a node. `server.rs` then does only what the answer says (send the response,
//! call the hub, route). So a change to who may do what is reviewed and tested here, on plain requests, without frames or sockets.
//!
//! Not here yet: a person's own devices (`server.rs::device_command`, ADR-0061), which decides and answers in one place, and the
//! shared terminal size, which is the router's state rather than a rule about who may act.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use agora_protocol::v1::command_request::Command as C;
use agora_protocol::v1::CommandRequest;
use agora_protocol::{Error, ErrorCode};

use crate::scope::Scope;
use crate::users::{viewer_may, Access, Role};

/// What happens to one client command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// Refused before any node hears of it (ADR-0049): a viewer's command that changes something, or a member's outside their
    /// projects. `auth.forbidden` with `action`; the audit trail names `action` against the request's `target_node`.
    Forbidden {
        action: String,
    },
    /// A command the hub answers itself was sent with a `target_node`: `protocol.invalid_message` (`field: target_node`).
    TargetNotAllowed,
    /// ADR-0053: the hub forgets a closed agent, named by its hub-wide key `node/agent` (`node` is its first part, for the audit).
    ForgetAgent {
        agent_id: String,
        node: String,
    },
    /// ADR-0073: the hub forgets an offline machine.
    ForgetNode {
        node_id: String,
        refuse_return: bool,
    },
    /// ADR-0073: `node.forget` from anyone but the owner. `auth.forbidden`, audited against the named machine.
    ForgetNodeForbidden {
        node_id: String,
    },
    /// ADR-0033: the hub answers from its own view and prices.
    UsageReport,
    AcknowledgeTaskHandoff {
        delivery_id: String,
    },
    /// ADR-0113 decision 6: the hub answers from the community theme catalog it keeps (any signed-in person).
    ThemeCatalogList,
    ThemeCatalogGet {
        id: String,
    },
    /// Everything else goes to the node the request names (the router still checks that it is known, connected and able).
    Forward,
}

/// The command's schema name (`AgentStop`), for an error that names what was refused and for the audit trail.
pub fn command_name(req: &CommandRequest) -> String {
    let dbg = format!("{:?}", req.command);
    let inner = dbg.strip_prefix("Some(").unwrap_or(&dbg);
    inner.split('(').next().unwrap_or("").to_string()
}

/// Decides what happens to `req` from `who` (with `scope` for a member or viewer limited to projects; `None` for the owner).
pub fn route(who: &Access, scope: Option<&Scope>, req: &CommandRequest) -> Route {
    // ADR-0049: a viewer's commands stop here, before any node hears of them. (The node still decides what its devices may do.)
    let may = req.command.as_ref().is_some_and(|c| {
        // ADR-0109: only the hub can issue a durable handoff delivery.
        !matches!(c, C::DeliverTaskHandoff(_))
            && (!matches!(c, C::AcknowledgeTaskHandoff(_)) || who.role == Role::Owner)
            && (who.role != Role::Viewer || viewer_may(c))
            && scope.is_none_or(|s| s.command_allowed(&req.target_node, c))
    });
    if !may {
        return Route::Forbidden {
            action: command_name(req),
        };
    }
    let names_a_node = !req.target_node.is_empty();
    match &req.command {
        Some(C::AgentForget(_)) if names_a_node => Route::TargetNotAllowed,
        Some(C::AgentForget(f)) => Route::ForgetAgent {
            agent_id: f.agent_id.clone(),
            node: f.agent_id.split('/').next().unwrap_or_default().to_string(),
        },
        // Machines belong to the owner, not to a project; the command names the machine, not a target.
        Some(C::NodeForget(_)) if names_a_node => Route::TargetNotAllowed,
        Some(C::NodeForget(f)) if who.role != Role::Owner => Route::ForgetNodeForbidden {
            node_id: f.node_id.clone(),
        },
        Some(C::NodeForget(f)) => Route::ForgetNode {
            node_id: f.node_id.clone(),
            refuse_return: f.refuse_return,
        },
        Some(C::AcknowledgeTaskHandoff(_)) if names_a_node => Route::TargetNotAllowed,
        Some(C::AcknowledgeTaskHandoff(cmd)) => Route::AcknowledgeTaskHandoff {
            delivery_id: cmd.delivery_id.clone(),
        },
        Some(C::UsageReport(_)) if names_a_node => Route::TargetNotAllowed,
        Some(C::UsageReport(_)) => Route::UsageReport,
        Some(C::ThemeCatalogList(_) | C::ThemeCatalogGet(_)) if names_a_node => {
            Route::TargetNotAllowed
        }
        Some(C::ThemeCatalogList(_)) => Route::ThemeCatalogList,
        Some(C::ThemeCatalogGet(g)) => Route::ThemeCatalogGet { id: g.id.clone() },
        _ => Route::Forward,
    }
}

/// ADR-0095: the most tasks one window may give to agents in [`DISPATCH_WINDOW`].
pub const MAX_DISPATCHES: usize = 8;
/// ADR-0095: the span [`MAX_DISPATCHES`] is counted over.
pub const DISPATCH_WINDOW: Duration = Duration::from_secs(60);

/// How a write was counted against its person's limit ([`WriteRates`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admit {
    Yes,
    /// Over the limit, and the first refusal in this span: worth one audit row.
    First,
    /// Over the limit again in the same span: refused without a row, so a loop of refused writes cannot fill the audit trail.
    Again,
}

/// ADR-0113 step 4 (and the plugin grants of ADR-0114): each person's recent writes of one kind (their settings value, their plugin
/// grants), kept across all their windows, at most `max` in `window`. Only people who wrote in the last span are kept.
#[derive(Debug)]
pub struct WriteRates {
    max: usize,
    window: Duration,
    seen: std::sync::Mutex<HashMap<String, (VecDeque<Instant>, bool)>>,
}

impl WriteRates {
    pub fn new(max: usize, window: Duration) -> Self {
        Self {
            max,
            window,
            seen: std::sync::Mutex::default(),
        }
    }

    pub fn max(&self) -> usize {
        self.max
    }

    /// Counts one write of `user` at `now`, when it fits.
    pub fn admit(&self, user: &str, now: Instant) -> Admit {
        let mut g = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let window = self.window;
        g.retain(|_, (recent, _)| {
            while recent
                .front()
                .is_some_and(|t| now.saturating_duration_since(*t) >= window)
            {
                recent.pop_front();
            }
            !recent.is_empty()
        });
        let (recent, refused) = g.entry(user.to_string()).or_default();
        if recent.len() >= self.max {
            if std::mem::replace(refused, true) {
                return Admit::Again;
            }
            return Admit::First;
        }
        *refused = false;
        recent.push_back(now);
        Admit::Yes
    }
}

/// ADR-0113 step 4: the most writes of their own settings value one person may make in [`WRITE_WINDOW`] (a window writes once per
/// change after 1.5 s, and again after a stale refusal).
pub const MAX_SETTINGS_WRITES: usize = 60;
/// ADR-0114: the most plugin grant changes (set or remove) one person may make in [`WRITE_WINDOW`].
pub const MAX_GRANT_WRITES: usize = 30;
pub const WRITE_WINDOW: Duration = Duration::from_secs(60);

/// ADR-0095: the times of each person's recent dispatches, kept by the hub across all of that person's windows and connections (so
/// reconnecting or opening another window does not reset the count). Only people who dispatched in the last minute are kept.
#[derive(Debug, Default)]
pub struct DispatchRates(std::sync::Mutex<HashMap<String, VecDeque<Instant>>>);

impl DispatchRates {
    /// Counts one dispatch of `user` at `now`; false when [`MAX_DISPATCHES`] were already counted in the last [`DISPATCH_WINDOW`].
    fn admit(&self, user: &str, now: Instant) -> bool {
        let mut g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        g.retain(|_, recent| {
            while recent
                .front()
                .is_some_and(|t| now.saturating_duration_since(*t) >= DISPATCH_WINDOW)
            {
                recent.pop_front();
            }
            !recent.is_empty()
        });
        let recent = g.entry(user.to_string()).or_default();
        if recent.len() >= MAX_DISPATCHES {
            return false;
        }
        recent.push_back(now);
        true
    }
}

/// ADR-0095, for a command that [`route`] lets through: a `TaskDispatch` gets `by_user` from the person who asks (whatever the client
/// wrote there is replaced) and counts against that person's limit (`task.dispatch_limited`, nothing forwarded). Every try counts,
/// also one the node then refuses. Any other command is left as it is.
pub fn prepare_dispatch(
    who: &Access,
    req: &mut CommandRequest,
    rates: &DispatchRates,
    now: Instant,
) -> Result<(), Error> {
    let Some(C::TaskDispatch(d)) = req.command.as_mut() else {
        return Ok(());
    };
    if !rates.admit(&who.user, now) {
        return Err(Error::new(ErrorCode::TaskDispatchLimited).with("limit", MAX_DISPATCHES as i64));
    }
    d.by_user = who.user.clone();
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use agora_protocol::v1;

    fn who(role: Role) -> Access {
        Access {
            role,
            user: "u".into(),
            projects: None,
            session: String::new(),
        }
    }

    #[test]
    fn only_the_owner_can_acknowledge_a_handoff_without_a_target() {
        let mut request = CommandRequest {
            command: Some(C::AcknowledgeTaskHandoff(v1::AcknowledgeTaskHandoff {
                delivery_id: "opaque".into(),
            })),
            ..Default::default()
        };
        for role in [Role::Viewer, Role::Member] {
            assert!(matches!(
                route(&who(role), None, &request),
                Route::Forbidden { .. }
            ));
        }
        assert!(matches!(
            route(&who(Role::Owner), None, &request),
            Route::AcknowledgeTaskHandoff { .. }
        ));
        request.target_node = "node".into();
        assert!(matches!(
            route(&who(Role::Owner), None, &request),
            Route::TargetNotAllowed
        ));
    }

    /// A person's writes past the limit are refused until the span has passed; only the first refusal of a span is worth a row; other
    /// people are counted apart.
    #[test]
    fn writes_are_limited_per_person_and_refusals_coalesce() {
        let rates = WriteRates::new(3, Duration::from_secs(60));
        let t0 = Instant::now();
        for _ in 0..3 {
            assert_eq!(rates.admit("eda", t0), Admit::Yes);
        }
        assert_eq!(rates.admit("eda", t0), Admit::First);
        assert_eq!(rates.admit("eda", t0), Admit::Again);
        assert_eq!(
            rates.admit("eda", t0 + Duration::from_secs(30)),
            Admit::Again
        );
        assert_eq!(rates.admit("bo", t0), Admit::Yes);
        let later = t0 + Duration::from_secs(61);
        assert_eq!(rates.admit("eda", later), Admit::Yes);
        assert_eq!(rates.max(), 3);
    }

    fn req(target: &str, command: C) -> CommandRequest {
        CommandRequest {
            request_id: "r".into(),
            idempotency_key: String::new(),
            target_node: target.into(),
            command: Some(command),
        }
    }

    fn stop() -> C {
        C::AgentStop(v1::AgentStop {
            agent_id: "a1".into(),
            stop_helpers: false,
        })
    }

    fn forget_node(refuse_return: bool) -> C {
        C::NodeForget(v1::NodeForget {
            node_id: "old".into(),
            refuse_return,
        })
    }

    #[test]
    fn the_owners_commands_are_forwarded_or_answered_by_the_hub() {
        let owner = who(Role::Owner);
        assert_eq!(route(&owner, None, &req("n1", stop())), Route::Forward);
        let usage = C::UsageReport(v1::UsageReport::default());
        assert_eq!(
            route(&owner, None, &req("", usage.clone())),
            Route::UsageReport
        );
        assert_eq!(
            route(&owner, None, &req("", forget_node(true))),
            Route::ForgetNode {
                node_id: "old".into(),
                refuse_return: true
            }
        );
        let forget_agent = C::AgentForget(v1::AgentForget {
            agent_id: "n1/a1".into(),
        });
        assert_eq!(
            route(&owner, None, &req("", forget_agent.clone())),
            Route::ForgetAgent {
                agent_id: "n1/a1".into(),
                node: "n1".into()
            }
        );
        // What the hub answers itself names no node.
        for c in [usage, forget_node(false), forget_agent] {
            assert_eq!(route(&owner, None, &req("n1", c)), Route::TargetNotAllowed);
        }
    }

    #[test]
    fn a_window_cannot_send_a_hub_handoff_delivery() {
        let delivery = C::DeliverTaskHandoff(v1::DeliverTaskHandoff::default());
        assert_eq!(
            route(&who(Role::Owner), None, &req("n1", delivery)),
            Route::Forbidden {
                action: "DeliverTaskHandoff".into()
            }
        );
    }

    #[test]
    fn a_viewer_only_reads_and_a_request_without_a_command_is_refused() {
        let viewer = who(Role::Viewer);
        assert_eq!(
            route(&viewer, None, &req("n1", stop())),
            Route::Forbidden {
                action: "AgentStop".into()
            }
        );
        let list = C::AgentList(v1::AgentList::default());
        assert_eq!(route(&viewer, None, &req("n1", list)), Route::Forward);
        // Forgetting a machine changes something: refused as any viewer command that does, before the owner check.
        assert_eq!(
            route(&viewer, None, &req("", forget_node(false))),
            Route::Forbidden {
                action: "NodeForget".into()
            }
        );
        let empty = CommandRequest {
            request_id: "r".into(),
            ..Default::default()
        };
        assert_eq!(
            route(&who(Role::Owner), None, &empty),
            Route::Forbidden {
                action: "None".into()
            }
        );
    }

    #[test]
    fn a_member_acts_only_in_their_projects_and_never_forgets_a_machine() {
        let member = Access {
            projects: Some(["n1/mine".to_string()].into()),
            ..who(Role::Member)
        };
        let scope = Scope::from_view(
            member.projects.clone().unwrap(),
            &crate::projection::View::default(),
        );
        // An agent the scope cannot place in a granted project is not theirs.
        assert_eq!(
            route(&member, Some(&scope), &req("n1", stop())),
            Route::Forbidden {
                action: "AgentStop".into()
            }
        );
        // A command about a whole machine is no project's: the scope refuses it first.
        assert_eq!(
            route(&member, Some(&scope), &req("", forget_node(false))),
            Route::Forbidden {
                action: "NodeForget".into()
            }
        );
        // Without a scope that refuses it first, a member's `node.forget` still stops at the owner check.
        assert_eq!(
            route(&member, None, &req("", forget_node(false))),
            Route::ForgetNodeForbidden {
                node_id: "old".into()
            }
        );
        // The target check comes before the owner check, as it did when this was inline: a member who also names a target is told
        // about the target, not about the owner rule (a reviewer found no test held this order).
        assert_eq!(
            route(&member, None, &req("n1", forget_node(false))),
            Route::TargetNotAllowed
        );
    }

    /// ADR-0113 decision 6: every signed-in person may read the theme catalog (whether it is on is the hub's answer, not a refusal), and
    /// the hub answers it itself: one that names a machine is refused, never forwarded.
    #[test]
    fn everyone_may_ask_the_hub_for_the_theme_catalog_and_it_never_goes_to_a_node() {
        let list = C::ThemeCatalogList(v1::ThemeCatalogList::default());
        let get = C::ThemeCatalogGet(v1::ThemeCatalogGet {
            id: "harbour".into(),
        });
        let member = Access {
            projects: Some(["n1/mine".to_string()].into()),
            ..who(Role::Member)
        };
        let scope = Scope::from_view(
            member.projects.clone().unwrap(),
            &crate::projection::View::default(),
        );
        let viewer = Access {
            projects: Some(["n1/mine".to_string()].into()),
            ..who(Role::Viewer)
        };
        for (person, scope) in [
            (who(Role::Owner), None),
            (member.clone(), Some(&scope)),
            (viewer, Some(&scope)),
            (who(Role::Viewer), None),
        ] {
            assert_eq!(
                route(&person, scope, &req("", list.clone())),
                Route::ThemeCatalogList
            );
            assert_eq!(
                route(&person, scope, &req("", get.clone())),
                Route::ThemeCatalogGet {
                    id: "harbour".into()
                }
            );
            for c in [list.clone(), get.clone()] {
                assert_eq!(
                    route(&person, scope, &req("n1", c)),
                    Route::TargetNotAllowed
                );
            }
        }
    }

    /// #1044: clients tell the hub's refusal for a person's role from a machine's refusal for a limited device by `action`: the hub names
    /// the refused command's schema name, a node names its own lowercase action (`directory`, `grant.create`, `approval.resolve`). The
    /// window and the terminal client word a start, a folder, a resume and an answer for the person when it is the schema name.
    #[test]
    fn a_refusal_for_a_persons_role_names_the_refused_command_by_its_schema_name() {
        let member = Access {
            projects: Some(["n1/mine".to_string()].into()),
            ..who(Role::Member)
        };
        let scope = Scope::from_view(
            member.projects.clone().unwrap(),
            &crate::projection::View::default(),
        );
        let commands = [
            ("AgentLaunch", C::AgentLaunch(v1::AgentLaunch::default())),
            ("AgentStart", C::AgentStart(v1::AgentStart::default())),
            ("AgentResume", C::AgentResume(v1::AgentResume::default())),
            (
                "DirectoryInspect",
                C::DirectoryInspect(v1::DirectoryInspect::default()),
            ),
            (
                "DirectoryList",
                C::DirectoryList(v1::DirectoryList::default()),
            ),
            (
                "ApprovalResolve",
                C::ApprovalResolve(v1::ApprovalResolve::default()),
            ),
        ];
        for (name, command) in commands {
            let refused = Route::Forbidden {
                action: name.into(),
            };
            assert_eq!(
                route(&who(Role::Viewer), None, &req("n1", command.clone())),
                refused
            );
            assert_eq!(route(&member, Some(&scope), &req("n1", command)), refused);
        }
        // What the window asks for its cost is the owner's only (it stops asking when the hub says the session is not the owner's).
        assert_eq!(
            route(
                &member,
                Some(&scope),
                &req("", C::UsageReport(v1::UsageReport::default()))
            ),
            Route::Forbidden {
                action: "UsageReport".into()
            }
        );
    }

    /// ADR-0049 Amendment A (#1049): a member chooses folders and starts agents inside a project granted on that machine; a viewer
    /// granted the same project stays read-only; a folder outside it is refused as #1044 words a role refusal (the schema name).
    #[test]
    fn a_member_works_in_folders_of_a_granted_project_and_a_viewer_with_the_same_grant_does_not() {
        let mut view = crate::projection::View::default();
        view.apply(&v1::Event {
            producer_id: "n1".into(),
            seq: 1,
            at_ms: 1,
            body: Some(v1::event::Body::CheckoutDiscovered(
                v1::CheckoutDiscovered {
                    checkout_id: "c1".into(),
                    node_id: "n1".into(),
                    project_id: "mine".into(),
                    path: "/srv/mine".into(),
                },
            )),
        });
        let granted = || Some(["n1/mine".to_string()].into());
        let member = Access {
            projects: granted(),
            ..who(Role::Member)
        };
        let viewer = Access {
            projects: granted(),
            ..who(Role::Viewer)
        };
        let scope = Scope::from_view(granted().unwrap_or_default(), &view);
        let commands = |path: &str| {
            [
                (
                    "DirectoryList",
                    C::DirectoryList(v1::DirectoryList {
                        path: path.into(),
                        authorization: None,
                    }),
                ),
                (
                    "DirectoryInspect",
                    C::DirectoryInspect(v1::DirectoryInspect {
                        path: path.into(),
                        ..Default::default()
                    }),
                ),
                (
                    "AgentLaunch",
                    C::AgentLaunch(v1::AgentLaunch {
                        path: path.into(),
                        project_id: "mine".into(),
                        kind: "shell".into(),
                        ..Default::default()
                    }),
                ),
            ]
        };
        for (name, command) in commands("/srv/mine/api") {
            assert_eq!(
                route(&member, Some(&scope), &req("n1", command.clone())),
                Route::Forward,
                "{name}"
            );
            assert_eq!(
                route(&viewer, Some(&scope), &req("n1", command)),
                Route::Forbidden {
                    action: name.into()
                }
            );
        }
        for path in ["/srv", "/srv/mine/../theirs", ""] {
            for (name, command) in commands(path) {
                assert_eq!(
                    route(&member, Some(&scope), &req("n1", command.clone())),
                    Route::Forbidden {
                        action: name.into()
                    },
                    "{path}"
                );
                assert_eq!(
                    route(&member, Some(&scope), &req("n2", command)),
                    Route::Forbidden {
                        action: name.into()
                    },
                    "{path} on another machine"
                );
            }
        }
        for (name, command) in commands("/srv/mine") {
            assert_eq!(
                route(&member, Some(&scope), &req("n2", command)),
                Route::Forbidden {
                    action: name.into()
                },
                "{name} on another machine"
            );
        }
    }

    /// ADR-0095: the hub writes who asked (a client's own `by_user` is replaced) and lets a person give 8 tasks a minute, no more,
    /// however many windows or connections they use.
    #[test]
    fn a_dispatch_names_the_person_and_a_window_may_give_eight_a_minute() {
        let dispatch = |by: &str| {
            req(
                "n1",
                C::TaskDispatch(v1::TaskDispatch {
                    project_id: "p".into(),
                    task_id: "t".into(),
                    agent_id: "a".into(),
                    by_user: by.into(),
                }),
            )
        };
        let rate = DispatchRates::default();
        let t0 = Instant::now();
        let person = who(Role::Member);
        for i in 0..MAX_DISPATCHES {
            let mut r = dispatch("someone-else");
            prepare_dispatch(&person, &mut r, &rate, t0 + Duration::from_secs(i as u64)).unwrap();
            assert!(matches!(&r.command, Some(C::TaskDispatch(d)) if d.by_user == "u"));
        }
        let mut r = dispatch("");
        let e = prepare_dispatch(&person, &mut r, &rate, t0 + Duration::from_secs(30)).unwrap_err();
        assert_eq!(e.code, ErrorCode::TaskDispatchLimited);
        assert_eq!(
            e.params.get("limit"),
            Some(&agora_protocol::ParamValue::Int(8))
        );
        // Another person has a count of their own.
        let mut theirs = dispatch("");
        prepare_dispatch(
            &Access {
                user: "v".into(),
                ..who(Role::Member)
            },
            &mut theirs,
            &rate,
            t0 + Duration::from_secs(30),
        )
        .unwrap();
        // A refused one does not count, and the first falls out of the window after a minute.
        prepare_dispatch(&person, &mut r, &rate, t0 + DISPATCH_WINDOW).unwrap();
        // Other commands are not touched and not counted.
        let mut other = req("n1", stop());
        let before = other.clone();
        for _ in 0..20 {
            prepare_dispatch(&person, &mut other, &rate, t0).unwrap();
        }
        assert_eq!(other, before);
    }
}
