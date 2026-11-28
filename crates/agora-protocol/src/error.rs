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
    NodeOnline,
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
    /// ADR-0066: the start names a setting the kind's manifest does not declare.
    AgentSettingUnknown,
    /// ADR-0066: a setting's value is not one it takes (not listed, or not a plain value).
    AgentSettingInvalid,
    AgentRunning,
    AgentNotInterrupted,
    AgentSessionMissing,
    AgentFolderChanged,
    AgentParentNotRunning,
    AgentStillRunning,
    ApprovalExpired,
    ApprovalSignatureInvalid,
    ApprovalGrantRequired,
    /// ADR-0101: the node's `skip_permissions` policy refuses an "always" answer for this agent.
    ApprovalScopeForbidden,
    /// ADR-0101: the agent did not offer that answer (scope) for this request; nothing was sent.
    ApprovalScopeUnavailable,
    /// ADR-0101 (conversation notice): an always-only request under a policy that refuses "always" was answered denied at once.
    ApprovalAlwaysOnlyForbidden,
    /// ADR-0101 (Codex review): the agent's bridge predates strict answers and may remember a plain Allow; the policy refuses that.
    ApprovalAdapterOutdated,
    /// ADR-0061: devices and the trust machines give them.
    DeviceUnknown,
    DeviceKeyMismatch,
    DeviceUntrusted,
    DeviceSignatureInvalid,
    DeviceEndorsementExpired,
    DeviceNotInScope,
    DeviceRevoked,
    LeaseHeld,
    BoardUnavailable,
    BoardSubtasksUnavailable,
    /// ADR-0112: the hub does not answer `fleet_status` for this hub (it has people besides the owner and the owner's switch is off).
    BoardNotAllowed,
    MeshRecipientUnknown,
    MeshRateLimited,
    MeshBodyTooLarge,
    MeshTooManyRefs,
    MeshTooManyAsks,
    MeshTimeout,
    MeshFolderNotAllowed,
    MeshRemoteHelpersOff,
    /// ADR-0122: a node will not give a shared item to its agents (`reason`).
    ShareRefused,
    /// ADR-0122: a shared MCP server needs a secret this node does not hold (`secret`).
    ShareSecretMissing,
    /// ADR-0122: no shared item by that name (or not that version).
    ShareUnknown,
    MeshHelperLimit,
    MeshStartDenied,
    MeshUnavailable,
    TaskNotFound,
    TaskNotOpen,
    TaskReviewStale,
    TaskSubtaskLimit,
    TaskBlocked,
    TaskNotOwner,
    LeaseNotFound,
    LeaseInvalidPath,
    FilesNotFound,
    FilesTooLarge,
    FilesOutsideRoot,
    FilesSecret,
    AdapterSignatureInvalid,
    AdapterCrashed,
    AdapterAgentExited,
    AdapterAgentRefused,
    AdapterOptionRefused,
    /// ADR-0101 (conversation notice): the agent asked without offering a way to allow it, so it was refused.
    AdapterPermissionUnanswerable,
    /// ADR-0110: an adapter did not answer a call made for a client (a prompt, an approval answer) within the node's wait.
    AdapterNoAnswer,
    PairClosed,
    PairLimit,
    PairExpired,
    PairMismatch,
    PairDenied,
    PairUnknown,
    PairState,
    PairInsecureTransport,
    PairNameUnverified,
    PairNamesMissing,
    PairDiscoveryUnconfirmed,
    PairCaMismatch,
    PreviewPortRefused,
    PreviewLimit,
    PreviewExpired,
    PreviewUnknown,
    PreviewUnavailable,
    AgentOptionUnknown,
    AgentQueueFull,
    AgentApprovalNeeded,
    EnrollRenewRefused,
    TaskDispatchLimited,
    /// RM-19 (#903): the kind's program could not be started because this machine cannot find it (not in the node's `PATH`, not
    /// installed by `agora agent install`, or a path that does not exist).
    AgentProgramMissing,
    /// ADR-0113 decision 6: the hub's owner has not turned the community theme catalog on (`agora hub themes on`).
    ThemeCatalogOff,
    /// ADR-0113 decision 6: the community theme catalog has no theme with this id.
    ThemeNotFound,
    /// ADR-0114: no plugin with that id is installed on the hub (at the version or package asked for).
    PluginNotFound,
    /// ADR-0120: the hub's owner has not turned the community plugin catalog on (`agora hub plugins on`).
    PluginCatalogOff,
    /// ADR-0113 decision 6 (step 4): a person's settings value is larger than the hub keeps (262144 bytes).
    SettingsTooLarge,
    /// ADR-0113 decision 6 (step 4): a settings write was based on an older version than the hub's; read again, merge and write again.
    SettingsStale,
    /// ADR-0113 step 4 and ADR-0114: this person made more writes of this kind (their settings value, their plugin grants) than the hub
    /// takes in a minute (`limit`); nothing changed.
    RateLimited,
    /// ADR-0131: automatic pickup does not apply to this caller now (`reason`: who said it and why).
    TaskPickupOff,
    /// ADR-0131: an older request sequence than the one the hub committed for the run (`committed_seq`); nothing was tried.
    TaskPickupStale,
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
        ErrorCode::NodeOnline,
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
        ErrorCode::AgentSettingUnknown,
        ErrorCode::AgentSettingInvalid,
        ErrorCode::AgentRunning,
        ErrorCode::AgentNotInterrupted,
        ErrorCode::AgentSessionMissing,
        ErrorCode::AgentFolderChanged,
        ErrorCode::AgentParentNotRunning,
        ErrorCode::AgentStillRunning,
        ErrorCode::ApprovalExpired,
        ErrorCode::ApprovalSignatureInvalid,
        ErrorCode::ApprovalGrantRequired,
        ErrorCode::ApprovalScopeForbidden,
        ErrorCode::ApprovalScopeUnavailable,
        ErrorCode::ApprovalAlwaysOnlyForbidden,
        ErrorCode::ApprovalAdapterOutdated,
        ErrorCode::DeviceUnknown,
        ErrorCode::DeviceKeyMismatch,
        ErrorCode::DeviceUntrusted,
        ErrorCode::DeviceSignatureInvalid,
        ErrorCode::DeviceEndorsementExpired,
        ErrorCode::DeviceNotInScope,
        ErrorCode::DeviceRevoked,
        ErrorCode::LeaseHeld,
        ErrorCode::BoardUnavailable,
        ErrorCode::BoardSubtasksUnavailable,
        ErrorCode::BoardNotAllowed,
        ErrorCode::MeshRecipientUnknown,
        ErrorCode::MeshRateLimited,
        ErrorCode::MeshBodyTooLarge,
        ErrorCode::MeshTooManyRefs,
        ErrorCode::MeshTooManyAsks,
        ErrorCode::MeshTimeout,
        ErrorCode::MeshFolderNotAllowed,
        ErrorCode::MeshRemoteHelpersOff,
        ErrorCode::ShareRefused,
        ErrorCode::ShareSecretMissing,
        ErrorCode::ShareUnknown,
        ErrorCode::MeshHelperLimit,
        ErrorCode::MeshStartDenied,
        ErrorCode::MeshUnavailable,
        ErrorCode::TaskNotFound,
        ErrorCode::TaskNotOpen,
        ErrorCode::TaskReviewStale,
        ErrorCode::TaskSubtaskLimit,
        ErrorCode::TaskBlocked,
        ErrorCode::TaskNotOwner,
        ErrorCode::LeaseNotFound,
        ErrorCode::LeaseInvalidPath,
        ErrorCode::FilesNotFound,
        ErrorCode::FilesTooLarge,
        ErrorCode::FilesOutsideRoot,
        ErrorCode::FilesSecret,
        ErrorCode::AdapterSignatureInvalid,
        ErrorCode::AdapterCrashed,
        ErrorCode::AdapterAgentExited,
        ErrorCode::AdapterAgentRefused,
        ErrorCode::AdapterOptionRefused,
        ErrorCode::AdapterPermissionUnanswerable,
        ErrorCode::AdapterNoAnswer,
        ErrorCode::PairClosed,
        ErrorCode::PairLimit,
        ErrorCode::PairExpired,
        ErrorCode::PairMismatch,
        ErrorCode::PairDenied,
        ErrorCode::PairUnknown,
        ErrorCode::PairState,
        ErrorCode::PairInsecureTransport,
        ErrorCode::PairNameUnverified,
        ErrorCode::PairNamesMissing,
        ErrorCode::PairDiscoveryUnconfirmed,
        ErrorCode::PairCaMismatch,
        ErrorCode::PreviewPortRefused,
        ErrorCode::PreviewLimit,
        ErrorCode::PreviewExpired,
        ErrorCode::PreviewUnknown,
        ErrorCode::PreviewUnavailable,
        ErrorCode::AgentOptionUnknown,
        ErrorCode::AgentQueueFull,
        ErrorCode::AgentApprovalNeeded,
        ErrorCode::EnrollRenewRefused,
        ErrorCode::TaskDispatchLimited,
        ErrorCode::AgentProgramMissing,
        ErrorCode::ThemeCatalogOff,
        ErrorCode::ThemeNotFound,
        ErrorCode::PluginNotFound,
        ErrorCode::PluginCatalogOff,
        ErrorCode::SettingsTooLarge,
        ErrorCode::SettingsStale,
        ErrorCode::RateLimited,
        ErrorCode::TaskPickupOff,
        ErrorCode::TaskPickupStale,
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
            ErrorCode::AgentSettingUnknown => "agent.setting_unknown",
            ErrorCode::AgentSettingInvalid => "agent.setting_invalid",
            ErrorCode::AgentRunning => "agent.running",
            ErrorCode::NodeOnline => "node.online",
            ErrorCode::AgentNotInterrupted => "agent.not_interrupted",
            ErrorCode::AgentSessionMissing => "agent.session_missing",
            ErrorCode::AgentFolderChanged => "agent.folder_changed",
            ErrorCode::AgentParentNotRunning => "agent.parent_not_running",
            ErrorCode::AgentStillRunning => "agent.still_running",
            ErrorCode::ApprovalExpired => "approval.expired",
            ErrorCode::ApprovalSignatureInvalid => "approval.signature_invalid",
            ErrorCode::ApprovalGrantRequired => "approval.grant_required",
            ErrorCode::ApprovalScopeForbidden => "approval.scope_forbidden",
            ErrorCode::ApprovalScopeUnavailable => "approval.scope_unavailable",
            ErrorCode::ApprovalAlwaysOnlyForbidden => "approval.always_only_forbidden",
            ErrorCode::ApprovalAdapterOutdated => "approval.adapter_outdated",
            ErrorCode::DeviceUnknown => "device.unknown",
            ErrorCode::DeviceKeyMismatch => "device.key_mismatch",
            ErrorCode::DeviceUntrusted => "device.untrusted",
            ErrorCode::DeviceSignatureInvalid => "device.signature_invalid",
            ErrorCode::DeviceEndorsementExpired => "device.endorsement_expired",
            ErrorCode::DeviceNotInScope => "device.not_in_scope",
            ErrorCode::DeviceRevoked => "device.revoked",
            ErrorCode::LeaseHeld => "lease.held",
            ErrorCode::BoardUnavailable => "board.unavailable",
            ErrorCode::BoardNotAllowed => "board.not_allowed",
            ErrorCode::MeshRecipientUnknown => "mesh.recipient_unknown",
            ErrorCode::MeshRateLimited => "mesh.rate_limited",
            ErrorCode::MeshBodyTooLarge => "mesh.body_too_large",
            ErrorCode::MeshTooManyRefs => "mesh.too_many_refs",
            ErrorCode::MeshTooManyAsks => "mesh.too_many_asks",
            ErrorCode::MeshTimeout => "mesh.timeout",
            ErrorCode::MeshFolderNotAllowed => "mesh.folder_not_allowed",
            ErrorCode::MeshRemoteHelpersOff => "mesh.remote_helpers_off",
            ErrorCode::ShareRefused => "share.refused",
            ErrorCode::ShareSecretMissing => "share.secret_missing",
            ErrorCode::ShareUnknown => "share.unknown",
            ErrorCode::MeshHelperLimit => "mesh.helper_limit",
            ErrorCode::MeshStartDenied => "mesh.start_denied",
            ErrorCode::MeshUnavailable => "mesh.unavailable",
            ErrorCode::TaskNotFound => "task.not_found",
            ErrorCode::TaskNotOpen => "task.not_open",
            ErrorCode::TaskReviewStale => "task.review_stale",
            ErrorCode::TaskSubtaskLimit => "task.subtask_limit",
            ErrorCode::BoardSubtasksUnavailable => "board.subtasks_unavailable",
            ErrorCode::TaskBlocked => "task.blocked",
            ErrorCode::TaskNotOwner => "task.not_owner",
            ErrorCode::LeaseNotFound => "lease.not_found",
            ErrorCode::LeaseInvalidPath => "lease.invalid_path",
            ErrorCode::FilesNotFound => "files.not_found",
            ErrorCode::FilesTooLarge => "files.too_large",
            ErrorCode::FilesOutsideRoot => "files.outside_root",
            ErrorCode::FilesSecret => "files.secret",
            ErrorCode::AdapterSignatureInvalid => "adapter.signature_invalid",
            ErrorCode::AdapterCrashed => "adapter.crashed",
            ErrorCode::AdapterAgentExited => "adapter.agent_exited",
            ErrorCode::AdapterAgentRefused => "adapter.agent_refused",
            ErrorCode::AdapterOptionRefused => "adapter.option_refused",
            ErrorCode::AdapterPermissionUnanswerable => "adapter.permission_unanswerable",
            ErrorCode::AdapterNoAnswer => "adapter.no_answer",
            ErrorCode::PairClosed => "pair.closed",
            ErrorCode::PairLimit => "pair.limit",
            ErrorCode::PairExpired => "pair.expired",
            ErrorCode::PairMismatch => "pair.mismatch",
            ErrorCode::PairDenied => "pair.denied",
            ErrorCode::PairUnknown => "pair.unknown",
            ErrorCode::PairState => "pair.state",
            ErrorCode::PairInsecureTransport => "pair.insecure_transport",
            ErrorCode::PairNameUnverified => "pair.name_unverified",
            ErrorCode::PairNamesMissing => "pair.names_missing",
            ErrorCode::PairDiscoveryUnconfirmed => "pair.discovery_unconfirmed",
            ErrorCode::PairCaMismatch => "pair.ca_mismatch",
            ErrorCode::PreviewPortRefused => "preview.port_refused",
            ErrorCode::PreviewLimit => "preview.limit",
            ErrorCode::PreviewExpired => "preview.expired",
            ErrorCode::PreviewUnknown => "preview.unknown",
            ErrorCode::PreviewUnavailable => "preview.unavailable",
            ErrorCode::AgentOptionUnknown => "agent.option_unknown",
            ErrorCode::AgentQueueFull => "agent.queue_full",
            ErrorCode::AgentApprovalNeeded => "agent.approval_needed",
            ErrorCode::EnrollRenewRefused => "enroll.renew_refused",
            ErrorCode::TaskDispatchLimited => "task.dispatch_limited",
            ErrorCode::AgentProgramMissing => "agent.program_missing",
            ErrorCode::ThemeCatalogOff => "theme.catalog_off",
            ErrorCode::ThemeNotFound => "theme.not_found",
            ErrorCode::PluginNotFound => "plugin.not_found",
            ErrorCode::PluginCatalogOff => "plugin.catalog_off",
            ErrorCode::SettingsTooLarge => "settings.too_large",
            ErrorCode::SettingsStale => "settings.stale",
            ErrorCode::RateLimited => "rate.limited",
            ErrorCode::TaskPickupOff => "task.pickup_off",
            ErrorCode::TaskPickupStale => "task.pickup_stale",
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
            ErrorCode::AgentSettingUnknown => &["kind", "setting"],
            ErrorCode::AgentSettingInvalid => &["kind", "setting"],
            ErrorCode::AgentRunning => &["agent_id"],
            ErrorCode::NodeOnline => &["node_id"],
            ErrorCode::AgentNotInterrupted => &["agent_id"],
            ErrorCode::AgentSessionMissing => &["agent_id"],
            ErrorCode::AgentFolderChanged => &["agent_id", "path"],
            ErrorCode::AgentParentNotRunning => &["agent_id", "parent_agent_id"],
            ErrorCode::AgentStillRunning => &["agent_id"],
            ErrorCode::ApprovalExpired => &["approval_id"],
            ErrorCode::ApprovalSignatureInvalid => &["approval_id"],
            ErrorCode::ApprovalGrantRequired => &["command"],
            ErrorCode::ApprovalScopeForbidden => &[],
            ErrorCode::ApprovalScopeUnavailable => &[],
            ErrorCode::ApprovalAlwaysOnlyForbidden => &["action"],
            ErrorCode::ApprovalAdapterOutdated => &[],
            ErrorCode::DeviceUnknown => &["device_id"],
            ErrorCode::DeviceKeyMismatch => &["device_id"],
            ErrorCode::DeviceUntrusted => &["device_id", "node_id"],
            ErrorCode::DeviceSignatureInvalid => &["device_id"],
            ErrorCode::DeviceEndorsementExpired => &["device_id"],
            ErrorCode::DeviceNotInScope => &["device_id", "node_id"],
            ErrorCode::DeviceRevoked => &["device_id"],
            ErrorCode::LeaseHeld => &["path", "holder_agent_id", "expires_at"],
            ErrorCode::BoardUnavailable | ErrorCode::BoardSubtasksUnavailable => &["project_id"],
            ErrorCode::BoardNotAllowed => &[],
            ErrorCode::MeshRecipientUnknown => &["to"],
            ErrorCode::MeshRateLimited => &[],
            ErrorCode::MeshBodyTooLarge => &["limit"],
            ErrorCode::MeshTooManyRefs => &["limit"],
            ErrorCode::MeshTooManyAsks => &["limit"],
            ErrorCode::MeshTimeout => &[],
            ErrorCode::MeshFolderNotAllowed => &["folder"],
            ErrorCode::MeshRemoteHelpersOff => &[],
            ErrorCode::ShareRefused => &["reason"],
            ErrorCode::ShareSecretMissing => &["secret"],
            ErrorCode::ShareUnknown => &["name"],
            ErrorCode::MeshHelperLimit => &["rule", "limit"],
            ErrorCode::MeshStartDenied => &["approval_id"],
            ErrorCode::MeshUnavailable => &[],
            ErrorCode::TaskNotFound => &["task_id"],
            ErrorCode::TaskNotOpen | ErrorCode::TaskReviewStale => &["task_id"],
            ErrorCode::TaskSubtaskLimit => &["limit"],
            ErrorCode::TaskBlocked => &["task_id", "depends_on"],
            ErrorCode::TaskNotOwner => &["task_id"],
            ErrorCode::LeaseNotFound => &["lease_id"],
            ErrorCode::LeaseInvalidPath => &["path"],
            ErrorCode::FilesNotFound => &["path"],
            ErrorCode::FilesTooLarge => &["path", "limit"],
            ErrorCode::FilesOutsideRoot => &["path"],
            ErrorCode::FilesSecret => &["path"],
            ErrorCode::AdapterSignatureInvalid => &["package"],
            ErrorCode::AdapterCrashed => &["kind", "exit_code"],
            ErrorCode::AdapterAgentExited => &["stage", "exit_code", "stderr_tail"],
            ErrorCode::AdapterAgentRefused => &["stage"],
            ErrorCode::AdapterOptionRefused => &["id"],
            ErrorCode::AdapterPermissionUnanswerable => &["action"],
            ErrorCode::AdapterNoAnswer => &["kind", "call", "waited_ms"],
            ErrorCode::PairClosed => &[],
            ErrorCode::PairLimit => &["seconds"],
            ErrorCode::PairExpired => &[],
            ErrorCode::PairMismatch => &[],
            ErrorCode::PairDenied => &[],
            ErrorCode::PairUnknown => &[],
            ErrorCode::PairState => &["state"],
            ErrorCode::PairInsecureTransport => &[],
            ErrorCode::PairNameUnverified => &["name"],
            ErrorCode::PairNamesMissing => &["reason", "names"],
            ErrorCode::PairDiscoveryUnconfirmed => &[],
            ErrorCode::PairCaMismatch => &[],
            ErrorCode::PreviewPortRefused => &["port", "reason"],
            ErrorCode::PreviewLimit => &["limit"],
            ErrorCode::PreviewExpired => &["forward_id"],
            ErrorCode::PreviewUnknown => &["forward_id"],
            ErrorCode::PreviewUnavailable => &["reason"],
            ErrorCode::AgentOptionUnknown => &["id"],
            ErrorCode::AgentQueueFull => &["limit"],
            ErrorCode::AgentApprovalNeeded => &["approval_id"],
            ErrorCode::EnrollRenewRefused => &["reason"],
            ErrorCode::TaskDispatchLimited => &["limit"],
            ErrorCode::AgentProgramMissing => &["kind", "program"],
            ErrorCode::ThemeCatalogOff => &[],
            ErrorCode::ThemeNotFound => &["id"],
            ErrorCode::PluginNotFound => &["plugin_id"],
            ErrorCode::PluginCatalogOff => &[],
            ErrorCode::SettingsTooLarge => &["max"],
            ErrorCode::SettingsStale => &["version"],
            ErrorCode::RateLimited => &["limit"],
            ErrorCode::TaskPickupOff => &["reason"],
            ErrorCode::TaskPickupStale => &["committed_seq"],
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

    /// The whole message of `key`: its first line and the indented lines that continue it (a Fluent select spans several).
    fn message(key: &str) -> Option<String> {
        let mut lines = EN.lines();
        let first = lines.find_map(|l| l.strip_prefix(&format!("{key} = ")))?;
        let mut msg = first.to_string();
        for l in lines.take_while(|l| l.starts_with(' ')) {
            msg.push('\n');
            msg.push_str(l);
        }
        Some(msg)
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
