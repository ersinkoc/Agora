//! Error model: stable codes with parameters, never human text (ADR-0007).
//!
//! The code list mirrors `docs/specs/error-codes.md`; a test keeps them in sync. Each code has an English
//! message in `locales/en/errors.ftl` under the key `code.replace('.', "-")`.

use std::collections::BTreeMap;

/// A stable error code. Codes are never renamed or reused. A code from a newer peer is kept as `Unknown`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    AuthUnauthenticated,
    AuthForbidden,
    AuthDeviceRevoked,
    ProtocolUnsupportedGeneration,
    ProtocolCapabilityMissing,
    ProtocolInvalidMessage,
    ProtocolProducerMismatch,
    NodeOffline,
    NodeUnknown,
    NodeTimeout,
    NodePolicyDenied,
    NodeResourceLimit,
    ProjectNotFound,
    WorktreeDirty,
    WorktreeExists,
    WorktreeUnmerged,
    AgentNotFound,
    AgentKindUnknown,
    AgentPublisherRevoked,
    AgentSandboxUnavailable,
    AgentNotAuthenticated,
    AgentBlocked,
    AgentModeUnsupported,
    /// ADR-0059: a launch option the node's policy does not allow, or that cannot be used this way.
    AgentOptionForbidden,
    /// ADR-0059: a launch option id that is malformed (`[a-z0-9-]{1,32}`), unknown, repeated, or too many.
    AgentOptionInvalid,
    /// ADR-0059: an option that skips permissions was sent without the user's signed confirmation marker.
    AgentOptionUnconfirmed,
    AgentRunning,
    ApprovalExpired,
    ApprovalSignatureInvalid,
    ApprovalGrantRequired,
    LeaseHeld,
    BoardUnavailable,
    MeshRecipientUnknown,
    MeshRateLimited,
    MeshBodyTooLarge,
    MeshTooManyRefs,
    MeshTooManyAsks,
    MeshTimeout,
    MeshFolderNotAllowed,
    MeshHelperLimit,
    MeshStartDenied,
    TaskNotFound,
    TaskNotOpen,
    TaskBlocked,
    TaskNotOwner,
    LeaseNotFound,
    LeaseInvalidPath,
    FilesNotFound,
    FilesTooLarge,
    FilesOutsideRoot,
    AdapterSignatureInvalid,
    AdapterCrashed,
    AdapterAgentExited,
    AdapterAgentRefused,
    AdapterOptionRefused,
    Internal,
    /// A code this build does not know (sent by a newer peer).
    Unknown(String),
}

impl ErrorCode {
    /// Every known code, in specification order.
    pub const ALL: &[ErrorCode] = &[
        ErrorCode::AuthUnauthenticated,
        ErrorCode::AuthForbidden,
        ErrorCode::AuthDeviceRevoked,
        ErrorCode::ProtocolUnsupportedGeneration,
        ErrorCode::ProtocolCapabilityMissing,
        ErrorCode::ProtocolInvalidMessage,
        ErrorCode::ProtocolProducerMismatch,
        ErrorCode::NodeOffline,
        ErrorCode::NodeUnknown,
        ErrorCode::NodeTimeout,
        ErrorCode::NodePolicyDenied,
        ErrorCode::NodeResourceLimit,
        ErrorCode::ProjectNotFound,
        ErrorCode::WorktreeDirty,
        ErrorCode::WorktreeExists,
        ErrorCode::WorktreeUnmerged,
        ErrorCode::AgentNotFound,
        ErrorCode::AgentKindUnknown,
        ErrorCode::AgentPublisherRevoked,
        ErrorCode::AgentSandboxUnavailable,
        ErrorCode::AgentNotAuthenticated,
        ErrorCode::AgentBlocked,
        ErrorCode::AgentModeUnsupported,
        ErrorCode::AgentOptionForbidden,
        ErrorCode::AgentOptionInvalid,
        ErrorCode::AgentOptionUnconfirmed,
        ErrorCode::AgentRunning,
        ErrorCode::ApprovalExpired,
        ErrorCode::ApprovalSignatureInvalid,
        ErrorCode::ApprovalGrantRequired,
        ErrorCode::LeaseHeld,
        ErrorCode::BoardUnavailable,
        ErrorCode::MeshRecipientUnknown,
        ErrorCode::MeshRateLimited,
        ErrorCode::MeshBodyTooLarge,
        ErrorCode::MeshTooManyRefs,
        ErrorCode::MeshTooManyAsks,
        ErrorCode::MeshTimeout,
        ErrorCode::MeshFolderNotAllowed,
        ErrorCode::MeshHelperLimit,
        ErrorCode::MeshStartDenied,
        ErrorCode::TaskNotFound,
        ErrorCode::TaskNotOpen,
        ErrorCode::TaskBlocked,
        ErrorCode::TaskNotOwner,
        ErrorCode::LeaseNotFound,
        ErrorCode::LeaseInvalidPath,
        ErrorCode::FilesNotFound,
        ErrorCode::FilesTooLarge,
        ErrorCode::FilesOutsideRoot,
        ErrorCode::AdapterSignatureInvalid,
        ErrorCode::AdapterCrashed,
        ErrorCode::AdapterAgentExited,
        ErrorCode::AdapterAgentRefused,
        ErrorCode::AdapterOptionRefused,
        ErrorCode::Internal,
    ];

    pub fn as_str(&self) -> &str {
        match self {
            ErrorCode::AuthUnauthenticated => "auth.unauthenticated",
            ErrorCode::AuthForbidden => "auth.forbidden",
            ErrorCode::AuthDeviceRevoked => "auth.device_revoked",
            ErrorCode::ProtocolUnsupportedGeneration => "protocol.unsupported_generation",
            ErrorCode::ProtocolCapabilityMissing => "protocol.capability_missing",
            ErrorCode::ProtocolInvalidMessage => "protocol.invalid_message",
            ErrorCode::ProtocolProducerMismatch => "protocol.producer_mismatch",
            ErrorCode::NodeOffline => "node.offline",
            ErrorCode::NodeUnknown => "node.unknown",
            ErrorCode::NodeTimeout => "node.timeout",
            ErrorCode::NodePolicyDenied => "node.policy_denied",
            ErrorCode::NodeResourceLimit => "node.resource_limit",
            ErrorCode::ProjectNotFound => "project.not_found",
            ErrorCode::WorktreeDirty => "worktree.dirty",
            ErrorCode::WorktreeExists => "worktree.exists",
            ErrorCode::WorktreeUnmerged => "worktree.unmerged",
            ErrorCode::AgentNotFound => "agent.not_found",
            ErrorCode::AgentKindUnknown => "agent.kind_unknown",
            ErrorCode::AgentPublisherRevoked => "agent.publisher_revoked",
            ErrorCode::AgentSandboxUnavailable => "agent.sandbox_unavailable",
            ErrorCode::AgentNotAuthenticated => "agent.not_authenticated",
            ErrorCode::AgentBlocked => "agent.blocked",
            ErrorCode::AgentModeUnsupported => "agent.mode_unsupported",
            ErrorCode::AgentOptionForbidden => "agent.option_forbidden",
            ErrorCode::AgentOptionInvalid => "agent.option_invalid",
            ErrorCode::AgentOptionUnconfirmed => "agent.option_unconfirmed",
            ErrorCode::AgentRunning => "agent.running",
            ErrorCode::ApprovalExpired => "approval.expired",
            ErrorCode::ApprovalSignatureInvalid => "approval.signature_invalid",
            ErrorCode::ApprovalGrantRequired => "approval.grant_required",
            ErrorCode::LeaseHeld => "lease.held",
            ErrorCode::BoardUnavailable => "board.unavailable",
            ErrorCode::MeshRecipientUnknown => "mesh.recipient_unknown",
            ErrorCode::MeshRateLimited => "mesh.rate_limited",
            ErrorCode::MeshBodyTooLarge => "mesh.body_too_large",
            ErrorCode::MeshTooManyRefs => "mesh.too_many_refs",
            ErrorCode::MeshTooManyAsks => "mesh.too_many_asks",
            ErrorCode::MeshTimeout => "mesh.timeout",
            ErrorCode::MeshFolderNotAllowed => "mesh.folder_not_allowed",
            ErrorCode::MeshHelperLimit => "mesh.helper_limit",
            ErrorCode::MeshStartDenied => "mesh.start_denied",
            ErrorCode::TaskNotFound => "task.not_found",
            ErrorCode::TaskNotOpen => "task.not_open",
            ErrorCode::TaskBlocked => "task.blocked",
            ErrorCode::TaskNotOwner => "task.not_owner",
            ErrorCode::LeaseNotFound => "lease.not_found",
            ErrorCode::LeaseInvalidPath => "lease.invalid_path",
            ErrorCode::FilesNotFound => "files.not_found",
            ErrorCode::FilesTooLarge => "files.too_large",
            ErrorCode::FilesOutsideRoot => "files.outside_root",
            ErrorCode::AdapterSignatureInvalid => "adapter.signature_invalid",
            ErrorCode::AdapterCrashed => "adapter.crashed",
            ErrorCode::AdapterAgentExited => "adapter.agent_exited",
            ErrorCode::AdapterAgentRefused => "adapter.agent_refused",
            ErrorCode::AdapterOptionRefused => "adapter.option_refused",
            ErrorCode::Internal => "internal",
            ErrorCode::Unknown(s) => s,
        }
    }

    /// Parse a wire code; unrecognised codes become `Unknown`.
    pub fn parse(code: &str) -> ErrorCode {
        Self::ALL
            .iter()
            .find(|c| c.as_str() == code)
            .cloned()
            .unwrap_or_else(|| ErrorCode::Unknown(code.to_string()))
    }

    /// Parameter names a client may use when rendering the message.
    pub fn params(&self) -> &'static [&'static str] {
        match self {
            ErrorCode::AuthUnauthenticated => &[],
            ErrorCode::AuthForbidden => &["action"],
            ErrorCode::AuthDeviceRevoked => &["device_id"],
            ErrorCode::ProtocolUnsupportedGeneration => &["offered", "supported"],
            ErrorCode::ProtocolCapabilityMissing => &["capability"],
            ErrorCode::ProtocolInvalidMessage => &["field"],
            ErrorCode::ProtocolProducerMismatch => &["node_id", "producer_id"],
            ErrorCode::NodeOffline => &["node_id"],
            ErrorCode::NodeUnknown => &["node_id"],
            ErrorCode::NodeTimeout => &["node_id"],
            ErrorCode::NodePolicyDenied => &["node_id", "rule"],
            ErrorCode::NodeResourceLimit => &["resource", "limit"],
            ErrorCode::ProjectNotFound => &["project_id"],
            ErrorCode::WorktreeDirty => &["path"],
            ErrorCode::WorktreeExists => &["branch"],
            ErrorCode::WorktreeUnmerged => &["branch"],
            ErrorCode::AgentNotFound => &["agent_id"],
            ErrorCode::AgentKindUnknown => &["kind"],
            ErrorCode::AgentPublisherRevoked => &["kind", "publisher"],
            ErrorCode::AgentSandboxUnavailable => &["kind", "level", "reason"],
            ErrorCode::AgentNotAuthenticated => &["kind", "node_id"],
            ErrorCode::AgentBlocked => &["agent_id"],
            ErrorCode::AgentModeUnsupported => &["kind", "mode"],
            ErrorCode::AgentOptionForbidden => &["kind", "option", "policy"],
            ErrorCode::AgentOptionInvalid => &["kind", "option"],
            ErrorCode::AgentOptionUnconfirmed => &["kind", "option"],
            ErrorCode::AgentRunning => &["agent_id"],
            ErrorCode::ApprovalExpired => &["approval_id"],
            ErrorCode::ApprovalSignatureInvalid => &["approval_id"],
            ErrorCode::ApprovalGrantRequired => &["command"],
            ErrorCode::LeaseHeld => &["path", "holder_agent_id", "expires_at"],
            ErrorCode::BoardUnavailable => &["project_id"],
            ErrorCode::MeshRecipientUnknown => &["to"],
            ErrorCode::MeshRateLimited => &[],
            ErrorCode::MeshBodyTooLarge => &["limit"],
            ErrorCode::MeshTooManyRefs => &["limit"],
            ErrorCode::MeshTooManyAsks => &["limit"],
            ErrorCode::MeshTimeout => &[],
            ErrorCode::MeshFolderNotAllowed => &["folder"],
            ErrorCode::MeshHelperLimit => &["rule", "limit"],
            ErrorCode::MeshStartDenied => &["approval_id"],
            ErrorCode::TaskNotFound => &["task_id"],
            ErrorCode::TaskNotOpen => &["task_id"],
            ErrorCode::TaskBlocked => &["task_id", "depends_on"],
            ErrorCode::TaskNotOwner => &["task_id"],
            ErrorCode::LeaseNotFound => &["lease_id"],
            ErrorCode::LeaseInvalidPath => &["path"],
            ErrorCode::FilesNotFound => &["path"],
            ErrorCode::FilesTooLarge => &["path", "limit"],
            ErrorCode::FilesOutsideRoot => &["path"],
            ErrorCode::AdapterSignatureInvalid => &["package"],
            ErrorCode::AdapterCrashed => &["kind", "exit_code"],
            ErrorCode::AdapterAgentExited => &["stage", "exit_code", "stderr_tail"],
            ErrorCode::AdapterAgentRefused => &["stage"],
            ErrorCode::AdapterOptionRefused => &["id"],
            ErrorCode::Internal => &["incident_id"],
            ErrorCode::Unknown(_) => &[],
        }
    }

    /// Fluent message id in `locales/<lang>/errors.ftl`.
    pub fn fluent_key(&self) -> String {
        self.as_str().replace('.', "-")
    }
}

/// A parameter value: flat strings and numbers only.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamValue {
    Str(String),
    Int(i64),
}

impl From<&str> for ParamValue {
    fn from(v: &str) -> Self {
        ParamValue::Str(v.to_string())
    }
}

impl From<i64> for ParamValue {
    fn from(v: i64) -> Self {
        ParamValue::Int(v)
    }
}

/// An error as carried on the wire: `{ code, params }`.
#[derive(Debug, Clone, PartialEq)]
pub struct Error {
    pub code: ErrorCode,
    pub params: BTreeMap<String, ParamValue>,
}

impl Error {
    pub fn new(code: ErrorCode) -> Self {
        Self {
            code,
            params: BTreeMap::new(),
        }
    }

    pub fn with(mut self, name: &str, value: impl Into<ParamValue>) -> Self {
        self.params.insert(name.to_string(), value.into());
        self
    }
}

impl Error {
    /// The wire form of this error.
    pub fn to_info(&self) -> crate::v1::ErrorInfo {
        use crate::v1::param_value::Value;
        crate::v1::ErrorInfo {
            code: self.code.as_str().to_string(),
            params: self
                .params
                .iter()
                .map(|(k, v)| {
                    let value = match v {
                        ParamValue::Str(s) => Value::Str(s.clone()),
                        ParamValue::Int(i) => Value::Int(*i),
                    };
                    (k.clone(), crate::v1::ParamValue { value: Some(value) })
                })
                .collect(),
        }
    }
}

impl std::fmt::Display for Error {
    /// Debug-oriented only (logs); user-facing text comes from Fluent.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.code.as_str())
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: &str = include_str!("../../../docs/specs/error-codes.md");
    const EN: &str = include_str!("../../../locales/en/errors.ftl");

    fn message(key: &str) -> Option<&'static str> {
        EN.lines()
            .find_map(|l| l.strip_prefix(&format!("{key} = ")))
    }

    #[test]
    fn code_list_matches_the_spec_table() {
        let spec: Vec<String> = SPEC
            .lines()
            .filter_map(|l| l.strip_prefix("| `"))
            .filter_map(|l| l.split('`').next())
            .map(str::to_string)
            .collect();
        let ours: Vec<String> = ErrorCode::ALL
            .iter()
            .map(|c| c.as_str().to_string())
            .collect();
        assert_eq!(ours, spec);
    }

    #[test]
    fn every_code_has_an_english_message_using_its_params() {
        for code in ErrorCode::ALL {
            let key = code.fluent_key();
            let msg = message(&key).unwrap_or_else(|| panic!("missing Fluent key {key}"));
            for p in code.params() {
                assert!(msg.contains(&format!("${p}")), "{key} does not use ${p}");
            }
        }
    }

    #[test]
    fn errors_ftl_has_no_key_without_a_code() {
        let known: Vec<String> = ErrorCode::ALL.iter().map(ErrorCode::fluent_key).collect();
        for line in EN
            .lines()
            .filter(|l| !l.starts_with('#') && l.contains(" = "))
        {
            let id = line.split(" = ").next().unwrap_or("");
            assert!(known.iter().any(|k| k == id), "key without a code: {id}");
        }
    }

    #[test]
    fn parse_round_trips_and_keeps_unknown_codes() {
        for c in ErrorCode::ALL {
            assert_eq!(&ErrorCode::parse(c.as_str()), c);
        }
        assert_eq!(
            ErrorCode::parse("future.thing"),
            ErrorCode::Unknown("future.thing".into())
        );
        assert_eq!(ErrorCode::parse("future.thing").as_str(), "future.thing");
    }

    #[test]
    fn builds_with_params() {
        let e = Error::new(ErrorCode::NodeOffline).with("node_id", "n1");
        assert_eq!(e.params.get("node_id"), Some(&ParamValue::Str("n1".into())));
        assert_eq!(
            ErrorCode::LeaseHeld.params(),
            &["path", "holder_agent_id", "expires_at"]
        );
    }
}
