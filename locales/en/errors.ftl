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
        [protected_path_unresolved] This machine's own Agora folder cannot be found, so it could not be kept closed to the agent.
        [writable_is_link] A file or folder it may use is reached through a link in a place it may write, so it could have made that link itself. Check that link: if you did not make it, remove it. Agora does not follow such links, also ones you made yourself (a dotfiles link).
        [writable_has_links] A file it may write also has another name in a different place (a hard link), so writing it would change that place too. Remove that link (or replace it with a copy) and start it again. Links inside node_modules, target and .git are not checked.
        [worktree_not_folder] Its folder is no longer a plain folder (something, such as a link, was put in its place).
       *[other] {""}
    }
agent-not_authenticated = { $kind } is not signed in on machine { $node_id }.
agent-blocked = Agent { $agent_id } is waiting for input, so the message was not sent.
agent-mode_unsupported = { $kind } does not support { $mode } mode.
agent-option_forbidden = { $kind } was not started with option { $option }: { $policy ->
        [confined_only] this machine lets an agent act without asking only while it is confined to its folder, and it would not be confined here.
        [signature] starting it that way needs your device's signature on the start.
       *[other] this machine does not let agents act without asking.
    }
agent-option_invalid = { $kind } was not started: the launch option "{ $option }" is not one it offers (or there are too many).
agent-setting_unknown = { $kind } was not started: it has no setting "{ $setting }" on this machine.
agent-setting_invalid = { $kind } was not started: the value chosen for "{ $setting }" is not one it takes.
agent-option_unconfirmed = { $kind } was not started with option { $option }: it lets the agent act without asking, and the start did not carry your confirmation for that.
agent-running = { $agent_id } is still running. Stop it first.
node-online = { $node_id } is connected. Only a machine that is offline can be forgotten.
agent-not_interrupted = { $agent_id } was not interrupted, or it is no longer offered to resume.
agent-session_missing = { $agent_id } cannot continue its own session. Start it again instead.
agent-folder_changed = The folder of { $agent_id } is gone or belongs to another project now: { $path }
agent-still_running = { $agent_id } is still running from before its machine's node restarted. Stop it first, then resume it.
agent-parent_not_running = { $agent_id } was started by { $parent_agent_id }. Resume that agent first.
approval-expired = Approval { $approval_id } has expired.
approval-signature_invalid = Approval { $approval_id } could not be verified.
approval-grant_required = This machine needs your permission first: { $command }.
device-unknown = There is no device { $device_id } of yours on the hub.
device-key_mismatch = The device { $device_id } is known with another key.
device-untrusted = The device { $device_id } is not trusted on the machine { $node_id }.
device-signature_invalid = The signature for the device { $device_id } does not verify.
device-endorsement_expired = The approval of the device { $device_id } has expired or lasts too long.
device-not_in_scope = The approval of the device { $device_id } is not for the machine { $node_id }.
device-revoked = The device { $device_id } was removed after it was approved.
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
mesh-unavailable = The mesh is reconnecting. Try again in a moment.
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
files-secret = { $path } is a secret file and is not shown.
adapter-signature_invalid = Package { $package } could not be verified.
adapter-crashed = { $kind } stopped unexpectedly (exit code { $exit_code }).
adapter-agent_exited = The agent exited before it was ready (step { $stage }, exit code { $exit_code }). Its last output: { $stderr_tail }
adapter-agent_refused = The agent refused to start (step { $stage }).
adapter-option_refused = The agent did not switch to { $id }.
pair-closed = This hub is not open for new machines right now.
pair-limit = Too many pairing attempts. Try again in { $seconds } seconds.
pair-expired = The pairing took too long and was cancelled.
pair-mismatch = The pairing did not check out and was cancelled.
pair-denied = The owner denied the request.
pair-unknown = This pairing does not exist or has ended.
pair-state = The pairing was cancelled because it was not at that step ({ $state }).
pair-insecure_transport = Signing in this way needs a secure connection (https) to the hub.
pair-name_unverified = The hub at { $name } could not be verified, so nothing was saved.
pair-names_missing = The hub could not be sure its certificate names the address a new machine will use ({ $reason }: { $names }). Nothing was opened. Start the hub with `--name ADDRESS` for each address a new machine will use, for example `--name 192.168.1.20`.
pair-discovery_unconfirmed = A hub that was found on the network is only used when you confirm it at a terminal. Nothing was saved. Run `agora join` in a terminal window, or give the hub's address: `agora join 192.168.1.20:7443`.
pair-ca_mismatch = The hub did not present the certificate authority you confirmed (the fingerprint it announced is not the one it delivered), so this machine did not join it. Nothing was saved.
agent-option_unknown = The agent has no setting { $id } with that value.
agent-queue_full = { $limit } messages are already waiting for this agent. Wait for it to finish or stop its current turn.
internal = Something went wrong. Reference: { $incident_id }.
