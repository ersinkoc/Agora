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
    NodeOffline,
    NodePolicyDenied,
    NodeResourceLimit,
    ProjectNotFound,
    WorktreeDirty,
    WorktreeExists,
    AgentNotFound,
    AgentKindUnknown,
    AgentNotAuthenticated,
    AgentBlocked,
    AgentModeUnsupported,
    ApprovalExpired,
    ApprovalSignatureInvalid,
    LeaseHeld,
    MeshRecipientUnknown,
    FilesNotFound,
    FilesTooLarge,
    AdapterSignatureInvalid,
    AdapterCrashed,
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
        ErrorCode::NodeOffline,
        ErrorCode::NodePolicyDenied,
        ErrorCode::NodeResourceLimit,
        ErrorCode::ProjectNotFound,
        ErrorCode::WorktreeDirty,
        ErrorCode::WorktreeExists,
        ErrorCode::AgentNotFound,
        ErrorCode::AgentKindUnknown,
        ErrorCode::AgentNotAuthenticated,
        ErrorCode::AgentBlocked,
        ErrorCode::AgentModeUnsupported,
        ErrorCode::ApprovalExpired,
        ErrorCode::ApprovalSignatureInvalid,
        ErrorCode::LeaseHeld,
        ErrorCode::MeshRecipientUnknown,
        ErrorCode::FilesNotFound,
        ErrorCode::FilesTooLarge,
        ErrorCode::AdapterSignatureInvalid,
        ErrorCode::AdapterCrashed,
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
            ErrorCode::NodeOffline => "node.offline",
            ErrorCode::NodePolicyDenied => "node.policy_denied",
            ErrorCode::NodeResourceLimit => "node.resource_limit",
            ErrorCode::ProjectNotFound => "project.not_found",
            ErrorCode::WorktreeDirty => "worktree.dirty",
            ErrorCode::WorktreeExists => "worktree.exists",
            ErrorCode::AgentNotFound => "agent.not_found",
            ErrorCode::AgentKindUnknown => "agent.kind_unknown",
            ErrorCode::AgentNotAuthenticated => "agent.not_authenticated",
            ErrorCode::AgentBlocked => "agent.blocked",
            ErrorCode::AgentModeUnsupported => "agent.mode_unsupported",
            ErrorCode::ApprovalExpired => "approval.expired",
            ErrorCode::ApprovalSignatureInvalid => "approval.signature_invalid",
            ErrorCode::LeaseHeld => "lease.held",
            ErrorCode::MeshRecipientUnknown => "mesh.recipient_unknown",
            ErrorCode::FilesNotFound => "files.not_found",
            ErrorCode::FilesTooLarge => "files.too_large",
            ErrorCode::AdapterSignatureInvalid => "adapter.signature_invalid",
            ErrorCode::AdapterCrashed => "adapter.crashed",
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
            ErrorCode::NodeOffline => &["node_id"],
            ErrorCode::NodePolicyDenied => &["node_id", "rule"],
            ErrorCode::NodeResourceLimit => &["resource", "limit"],
            ErrorCode::ProjectNotFound => &["project_id"],
            ErrorCode::WorktreeDirty => &["path"],
            ErrorCode::WorktreeExists => &["branch"],
            ErrorCode::AgentNotFound => &["agent_id"],
            ErrorCode::AgentKindUnknown => &["kind"],
            ErrorCode::AgentNotAuthenticated => &["kind", "node_id"],
            ErrorCode::AgentBlocked => &["agent_id"],
            ErrorCode::AgentModeUnsupported => &["kind", "mode"],
            ErrorCode::ApprovalExpired => &["approval_id"],
            ErrorCode::ApprovalSignatureInvalid => &["approval_id"],
            ErrorCode::LeaseHeld => &["path", "holder_agent_id", "expires_at"],
            ErrorCode::MeshRecipientUnknown => &["to"],
            ErrorCode::FilesNotFound => &["path"],
            ErrorCode::FilesTooLarge => &["path", "limit"],
            ErrorCode::AdapterSignatureInvalid => &["package"],
            ErrorCode::AdapterCrashed => &["kind", "exit_code"],
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
