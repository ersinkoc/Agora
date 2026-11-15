//! What the hub does with one client command (architecture review R9): the decision, apart from the transport that carries it.
//!
//! [`route`] is pure: it reads who asks ([`Access`]), what they may see ([`Scope`]) and the request, and says whether the command is
//! refused, answered by the hub itself, or forwarded to a node. `server.rs` then does only what the answer says (send the response,
//! call the hub, route). So a change to who may do what is reviewed and tested here, on plain requests, without frames or sockets.
//!
//! Not here yet: a person's own devices (`server.rs::device_command`, ADR-0061), which decides and answers in one place, and the
//! shared terminal size, which is the router's state rather than a rule about who may act.

use agora_protocol::v1::command_request::Command as C;
use agora_protocol::v1::CommandRequest;

use crate::scope::Scope;
use crate::users::{viewer_may, Access, Role};

/// What happens to one client command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// Refused before any node hears of it (ADR-0049): a viewer's command that changes something, or a member's outside their
    /// projects. `auth.forbidden` with `action`; the audit trail names `action` against the request's `target_node`.
    Forbidden { action: String },
    /// A command the hub answers itself was sent with a `target_node`: `protocol.invalid_message` (`field: target_node`).
    TargetNotAllowed,
    /// ADR-0053: the hub forgets a closed agent, named by its hub-wide key `node/agent` (`node` is its first part, for the audit).
    ForgetAgent { agent_id: String, node: String },
    /// ADR-0073: the hub forgets an offline machine.
    ForgetNode {
        node_id: String,
        refuse_return: bool,
    },
    /// ADR-0073: `node.forget` from anyone but the owner. `auth.forbidden`, audited against the named machine.
    ForgetNodeForbidden { node_id: String },
    /// ADR-0033: the hub answers from its own view and prices.
    UsageReport,
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
        (who.role != Role::Viewer || viewer_may(c))
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
        Some(C::UsageReport(_)) if names_a_node => Route::TargetNotAllowed,
        Some(C::UsageReport(_)) => Route::UsageReport,
        _ => Route::Forward,
    }
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
    }
}
