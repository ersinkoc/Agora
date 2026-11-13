# Error messages. Key = error code with dots replaced by dashes (docs/specs/error-codes.md).
# Every code must have a message here; a test enforces it.

auth-unauthenticated = You are not signed in.
auth-forbidden = You are not allowed to do this ({ $action }).
auth-device_revoked = This device ({ $device_id }) has been removed and can no longer connect.
protocol-unsupported_generation = This version is not compatible: offered { $offered }, supported { $supported }.
protocol-capability_missing = The other side does not support { $capability }.
protocol-invalid_message = The request was not valid (field: { $field }).
protocol-producer_mismatch = Machine { $node_id } sent events recorded under another name ({ $producer_id }); the hub did not accept them.
node-offline = Machine { $node_id } is not connected.
node-unknown = Machine { $node_id } is not known to this hub.
node-timeout = Machine { $node_id } did not answer in time.
node-policy_denied = Machine { $node_id } does not allow this (rule: { $rule }).
node-resource_limit = Machine limit reached for { $resource } (limit: { $limit }).
project-not_found = Project { $project_id } was not found.
worktree-dirty = There are uncommitted changes in { $path }.
worktree-exists = A worktree for branch { $branch } already exists.
worktree-unmerged = Branch { $branch } has work that is not saved anywhere else.
agent-not_found = Agent { $agent_id } was not found.
agent-kind_unknown = Agent type { $kind } is not available.
agent-publisher_revoked = The adapter for { $kind } comes from { $publisher }, which the registry has revoked. Remove it or install it from another publisher.
agent-sandbox_unavailable = This machine is set to confine agents ({ $level }), but { $kind } cannot be confined here, so it was not started. { $reason ->
        [broad_write_grant] A folder it would use lets every user write to it.
        [label_failed] A folder it would use cannot be marked for confined agents.
       *[other] {""}
    }
agent-not_authenticated = { $kind } is not signed in on machine { $node_id }.
agent-blocked = Agent { $agent_id } is waiting for input, so the message was not sent.
agent-mode_unsupported = { $kind } does not support { $mode } mode.
agent-running = { $agent_id } is still running. Stop it first.
approval-expired = Approval { $approval_id } has expired.
approval-signature_invalid = Approval { $approval_id } could not be verified.
approval-grant_required = This machine needs your permission first: { $command }.
lease-held = { $path } is in use by agent { $holder_agent_id } until { $expires_at }.
board-unavailable = The shared board for { $project_id } is not reachable right now. Your work continues locally.
mesh-recipient_unknown = No agent or channel named { $to }.
mesh-rate_limited = You are sending messages too fast. Try again in a moment.
mesh-body_too_large = The message is too large (limit: { $limit }). Send a reference instead.
mesh-too_many_refs = Too many references (limit: { $limit }).
mesh-too_many_asks = Too many questions are waiting for answers (limit: { $limit }).
mesh-timeout = No answer arrived in time.
mesh-folder_not_allowed = A helper can only work in the asking agent's own folder: { $folder } is outside it.
mesh-helper_limit = The agent cannot start another helper now (limit: { $limit }). { $rule ->
        [depth] A helper cannot start helpers of its own.
        [pending] Another start is still waiting for your answer.
        [prompt] Its first prompt is too long.
       *[other] It already runs as many helpers as allowed.
    }
mesh-start_denied = The start was refused ({ $approval_id }).
task-not_found = Task { $task_id } was not found.
task-not_open = Task { $task_id } is already taken or done.
task-blocked = Task { $task_id } waits for { $depends_on }.
task-not_owner = Task { $task_id } belongs to another agent.
lease-not_found = Lease { $lease_id } was not found.
lease-invalid_path = { $path } cannot be locked.
files-not_found = { $path } was not found.
files-too_large = { $path } is too large (limit: { $limit }).
files-outside_root = { $path } is outside the project folder.
adapter-signature_invalid = Package { $package } could not be verified.
adapter-crashed = { $kind } stopped unexpectedly (exit code { $exit_code }).
adapter-agent_exited = The agent exited before it was ready (step { $stage }, exit code { $exit_code }). Its last output: { $stderr_tail }
adapter-agent_refused = The agent refused to start (step { $stage }).
adapter-option_refused = The agent did not switch to { $id }.
internal = Something went wrong. Reference: { $incident_id }.
