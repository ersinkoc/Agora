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
node-policy_denied = { $rule ->
        [registry_signed_start] { $kind } downloads its program when it starts, so only a start signed on one of your trusted devices can start it (the window's Start, or a folder start in the terminal client), not the hub alone.
        [registry_off] This machine's policy switches the ACP registry's agents off (`agents.registry` is `off`), so { $kind } is not started.
        [registry_denied] This machine's policy switches { $kind } off (`agents.registry.deny`).
       *[other] Machine { $node_id } does not allow this (rule: { $rule }).
    }
node-resource_limit = Machine limit reached for { $resource } (limit: { $limit }).
project-not_found = Project { $project_id } was not found.
worktree-dirty = There are uncommitted changes in { $path }.
worktree-exists = A worktree for branch { $branch } already exists.
worktree-unmerged = Branch { $branch } has work that is not saved anywhere else.
agent-not_found = Agent { $agent_id } was not found.
agent-kind_unknown = Agent type { $kind } is not available.
agent-publisher_revoked = The adapter for { $kind } comes from { $publisher }, which the registry has revoked. Remove it or install it from another publisher.
agent-sandbox_unavailable = This machine is set to confine agents ({ $level }), but { $kind } cannot be confined here, so it was not started.{ $reason ->
        [broad_write_grant] {" "}A folder it would use lets every user write to it.
        [control_token_reachable] {" "}A folder it would be given contains the key this machine's Agora uses to tell you apart from agents, so it could act as you. Keep this machine's Agora folder outside the folders agents may use; agora doctor names it.
        [label_failed] {" "}A folder it would use cannot be marked for confined agents.
        [protected_path_unresolved] {" "}This machine's own Agora folder cannot be found, so it could not be kept closed to the agent.
        [protected_path_reachable] {" "}A folder it would be given contains or leads to this machine's own Agora files (its data or agent types), and this system cannot keep them closed inside a folder the agent may use. Move them out of it (not under /tmp, /var or /opt); agora doctor names it when a setting is the cause. If it names nothing, a folder the agent is given was a link to them when it started: check that link and remove it.
        [secrets_reachable] {" "}A folder it would be given contains the secrets this machine stores. Remove that folder from this machine's rules for agents.
        [writable_is_link] {" "}A file or folder it may use is reached through a link in a place it may write, so it could have made that link itself. Check that link: if you did not make it, remove it. Agora does not follow such links, also ones you made yourself (a dotfiles link).
        [writable_has_links] {" "}A file it may write also has another name in a different place (a hard link), so writing it would change that place too. Remove that link (or replace it with a copy) and start it again. Links inside node_modules, target and .git are not checked.
        [worktree_not_folder] {" "}Its folder is no longer a plain folder (something, such as a link, was put in its place).
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
agent-option_unconfirmed = Option { $option } of { $kind } lets the agent act without asking, and this request did not carry your confirmation for that.
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
approval-scope_forbidden = This machine does not let agents act without asking, so only this one request can be allowed.
approval-scope_unavailable = The agent does not offer that answer for this request. Nothing was sent; answer it again.
approval-always_only_forbidden = The agent asked for “{ $action }” and offered only an answer it would remember. This machine allows single answers only, so it was refused.
approval-adapter_outdated = This agent was started by an older Agora, which could make it remember a single Allow. This machine allows single answers only: start the agent again, then answer.
device-unknown = There is no device { $device_id } of yours on the hub.
device-key_mismatch = The device { $device_id } is known with another key.
device-untrusted = The device { $device_id } is not trusted on the machine { $node_id }.
device-signature_invalid = The signature for the device { $device_id } does not verify.
device-endorsement_expired = The approval of the device { $device_id } has expired or lasts too long.
device-not_in_scope = The approval of the device { $device_id } is not for the machine { $node_id }.
device-revoked = The device { $device_id } was removed after it was approved.
lease-held = { $path } is in use by agent { $holder_agent_id } until { $expires_at }.
board-unavailable = The shared board for { $project_id } is not reachable right now. Your work continues locally.
board-not_allowed = This hub does not show every machine and agent to agents, because other people use it. The hub's owner can turn this on.
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
adapter-agent_exited = { $stage ->
        [turn] The agent exited while it worked (exit code { $exit_code }). Start it again. Its last output: { $stderr_tail }
       *[other] The agent exited before it was ready (step { $stage }, exit code { $exit_code }). Start it again. Its last output: { $stderr_tail }
    }
adapter-agent_refused = The agent refused to start (step { $stage }).
adapter-option_refused = The agent did not switch to { $id }.
adapter-permission_unanswerable = The agent asked for “{ $action }” without offering a way to allow it, so it was refused.
adapter-no_answer = { $call ->
        [approval] { $kind } did not take your answer within { $waited_ms } ms. The request is still open: wait a moment and answer again.
       *[prompt] { $kind } did not confirm your request within { $waited_ms } ms. Wait a moment, and send it again only if nothing happens.
    }
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
preview-port_refused = Port { $port } cannot be previewed ({ $reason }). Only a program on this machine's own address, on a port of 1024 or above that is not Agora's own, can be shown.
preview-limit = This machine already has the most previews it allows ({ $limit }). Close one first.
preview-expired = The preview { $forward_id } has ended. Open it again to keep going.
preview-unknown = There is no preview { $forward_id } on this machine.
preview-unavailable = Previews do not work in this setup ({ $reason }). They need an address of their own on the hub.
agent-option_unknown = The agent has no setting { $id } with that value.
agent-queue_full = { $limit } messages are already waiting for this agent. Wait for it to finish or stop its current turn.
agent-approval_needed = This choice lets the agent act without asking, so it waits for your approval ({ $approval_id }). Nothing has changed yet.
enroll-renew_refused = This machine's certificate could not be renewed ({ $reason }). It keeps the one it has; if that one is revoked or has ended, join the hub again.
task-dispatch_limited = You tried to give { $limit } tasks to agents in the last minute. Wait a moment, then try again.
agent-program_missing = { $kind } did not start: this machine cannot find { $program } in its PATH. Install it there, or start the node from a terminal where { $program } runs.
theme-catalog_off = The community theme catalog is off on this hub. Its owner can turn it on with agora hub themes on.
theme-not_found = The theme catalog has no theme { $id }. It may have been removed at the last update.
plugin-not_found = The plugin { $plugin_id } is not installed on this hub, or not in this version.
settings-too_large = Your themes are too large to keep in your Agora (at most { $max } bytes). They stay in this browser.
settings-stale = Your themes changed on another device just now (version { $version }). They will be merged and saved again.
internal = Something went wrong. Reference: { $incident_id }.
