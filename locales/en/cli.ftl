# Command-line output. Machine-readable data (ids, states, paths) is printed as-is; only sentences live here.

cli-usage =
    Usage:
      agora [--ui tui|web|none [--save]]
      agora web
      agora status
      agora down
      agora node run [--socket PATH] [--manifests DIR] [--project PATH]... [--data DIR] [--worktrees DIR]
      agora agent start KIND [--cwd PATH] [--worktree] [--project NAME] [--name NAME] [--accept-project-id-change] [--socket PATH]
      agora project init [--cwd PATH] [--socket PATH]
      agora agent stop AGENT_ID [--socket PATH]
      agora agent prompt AGENT_ID TEXT... [--socket PATH]
      agora agent list [--socket PATH]
      agora agent kinds [--socket PATH]
      agora terminal attach AGENT_ID [--socket PATH]
      agora node stdio [--socket PATH]
      agora mesh stdio --socket PATH
      agora hub run [--listen ADDR] [--hub-dir DIR] [--name HOST]... [--web DIR]
      agora hub code [--hub-dir DIR]
      agora hub ssh install NAME [--yes] [--unsigned] [--release-url URL] [--hub-dir DIR]
      agora hub login [--hub-dir DIR]
      agora password set|clear|status [--hub-dir DIR]
      agora hub notify [--ntfy URL | --webhook URL | --test] [--hub-dir DIR]
      agora hub prices update [--url URL] [--hub-dir DIR]
      agora hub ssh [add NAME DESTINATION [--port N]] [--hub-dir DIR]
      agora hub backup [FILE] [--keep N] [--hub-dir DIR]
      agora hub restore FILE [--hub-dir DIR]
      agora node enroll --hub URL --ca FILE --code CODE --id NAME
      agora panel
      agora node trust DEVICE_ID PUBLIC_KEY_HEX [--data DIR]
      agora tui [--hub HOST:PORT --ca FILE --code CODE] [--hub-dir DIR]
      agora update [--check] [--version X] [--release-url URL] [--release-key FILE] [--unsigned]
      agora adapter install NAME [--registry URL] [--root-key FILE] [--yes] | update [NAME] | list | remove NAME
      agora doctor [--data DIR] [--manifests DIR] [--socket PATH]
cli-node-listening = Node is running. Control socket: { $socket }
cli-node-events-set-aside = Events recorded before this machine was linked as { $node_id } (under { $old_id }) were set aside and will not be sent to the hub: { $path }
cli-agent-started = Started agent { $agent_id } in { $worktree }
cli-agent-stopped = Stopped agent { $agent_id }
cli-agent-prompted = Sent the prompt to agent { $agent_id }
cli-agent-list-empty = This node has no agents.
cli-agent-kinds-empty = This node has no agent descriptions.
cli-terminal-detached = Detached from { $agent_id }.
cli-attach-needs-terminal = Attaching needs an interactive terminal.
cli-node-not-running = No node is listening at { $socket }. Start one with "agora node run".
cli-unknown-project = No project named { $project } is registered on this node.
cli-ambiguous-project = More than one project is registered; choose one with --project.
cli-no-projects = This node has no projects. Start it with --project PATH.
cli-bad-arguments = The arguments were not understood.
cli-error = Error { $code }: { $message }
cli-invalid-directory-identity_changed = This folder's project identity changed since an agent last started here. Add --accept-project-id-change to start one anyway.

doctor-status-ok = ok
doctor-status-warn = note
doctor-status-fail = PROBLEM
doctor-line = [{ $status }] { $text }
doctor-version = Agora { $detail }
doctor-pty = Terminals are supported on this system.
doctor-pty-unsupported = Terminals are not supported on { $detail } yet.
doctor-local-sockets = Local control sockets are supported.
doctor-local-sockets-unsupported = Local control sockets are not supported on { $detail }; the CLI cannot reach a node here.
doctor-sandbox = Agents can be confined to their workspace on this machine (the node's policy: { $detail }).
doctor-sandbox-unsupported = Agents cannot be confined on this machine (the node's policy: { $detail }). Anything but "off" means agents will not start here.
doctor-git = Git is available ({ $detail }).
doctor-git-missing = Git was not found. Agora needs it to give each agent its own workspace.
doctor-data-dir = The data folder is writable: { $detail }
doctor-data-dir-unwritable = The data folder cannot be written: { $detail }
doctor-node = A node is running at { $detail }.
doctor-node-not-running = No node is running at { $detail }. Start one with "agora node run".
doctor-manifests = { $detail ->
        [one] 1 agent description loaded.
       *[other] { $detail } agent descriptions loaded.
    }
doctor-agent-installed = { $detail } ({ $from ->
        [built-in] built in
       *[folder] from the manifests folder
    }): `{ $program }` is found.
doctor-agent-missing = { $detail } ({ $from ->
        [built-in] built in
       *[folder] from the manifests folder
    }): this machine cannot find `{ $program }` in its PATH. Install it there, or start the node from a terminal where `{ $program }` runs.
doctor-policy-path-dropped = The node's policy line "{ $detail }" names a path the node cannot use (relative, "..", or "~" without a home folder); agents start without it. Write a full path or one starting with "~/".
doctor-policy-path-missing = The node's policy line "{ $detail }" names a path that does not exist yet; agents start without it until it does.
doctor-manifest-invalid = The agent description { $detail } could not be read.
doctor-summary-ok = Everything Agora needs is in place.
doctor-summary-problems = Something Agora needs is missing. Fix the items marked PROBLEM.

cli-hub-listening = Hub is running on { $addr }. Its data folder: { $dir }
cli-hub-notify = Notifications go to { $count } target(s) from { $file }.
cli-hub-notify-skipped = { $count } line(s) of { $file } were not understood and are ignored. Each line is `ntfy URL` or `webhook URL`.
cli-hub-web = Web app: https://{ $addr }/ . Sign in with the code { $code } (valid for 10 minutes; `agora hub login` makes another). The hub's certificate is signed by its own CA, so the browser warns until you trust it.
cli-hub-login = Sign-in code for the web app: { $code } (valid for 10 minutes). Open the hub's address in a browser and enter it.
cli-password-prompt = New screen password (not shown as you type):
cli-password-repeat = Type it again:
cli-password-needs-terminal = Setting the screen password needs an interactive terminal: it is never read from a pipe or a file.
cli-password-too-short = That is too short: use at least { $min } characters.
cli-password-too-long = That is too long.
cli-password-mismatch = The two did not match; nothing was changed.
cli-password-set = Screen password set. A sign-in from another machine now needs it; this machine's own browser is never asked. { $count } open sessions were signed out.
cli-password-cleared = Screen password removed. A sign-in code alone is enough again. { $count } open sessions were signed out.
cli-password-on = A screen password is set: sign-ins from other machines need it.
cli-password-off = No screen password is set: a sign-in code alone is enough.
cli-trusted = Device { $id } can now answer approvals on this node.
cli-trust-failed = Could not trust that device: { $reason }
cli-hub-backup = Backup written to { $path }.
cli-hub-restored = Database restored from the backup into { $path }. The previous one is kept next to it as before-restore.
cli-hub-failed = The hub command failed: { $reason }
cli-hub-fingerprint = The hub CA fingerprint (SHA-256): { $fingerprint }
cli-hub-user-added = Added { $id } as { $role }. Their sign-in code: { $code } (valid for 10 minutes). Have them open the hub's address and enter it; `agora hub user login { $id }` makes another.
cli-hub-user-none = No team members yet. Add one with `agora hub user add NAME --role member`.
cli-hub-user-line = { $id }  { $name }  { $role }
cli-hub-user-unknown = There is no team member called { $id }.
cli-hub-user-project =   works in { $project }
cli-hub-code = Enrollment code: { $code } (valid for 10 minutes). On the other machine run: agora node enroll --hub wss://HOST:PORT --ca { $ca } --code { $code } --id NAME
cli-enrolled = This machine is enrolled as { $id }. Start it with "agora node run"; it will connect to the hub by itself. Saved in { $dir }.
cli-enroll-refused = The hub refused to enroll this machine ({ $code }). The code may be wrong, expired or already used.
cli-hub-notify-added = Added to { $file }. Restart the hub to use it.
cli-hub-notify-none = No notification targets yet. Add one with --ntfy URL or --webhook URL ({ $file }).
cli-hub-notify-bad-url = That is not an http:// or https:// address.
cli-hub-notify-sent = Sent a test message to { $target }.
cli-hub-notify-refused = { $target } answered with status { $status }.
cli-hub-notify-unreachable = Could not reach { $target }: { $reason }
cli-hub-prices-updated = Price table updated: { $count } models.
cli-hub-prices-failed = Could not update the price table: { $reason }
cli-hub-prices-skipped = { $count } entries of prices.json were not understood and are ignored. Each needs input and output prices (USD per million tokens).
cli-hub-ssh-started = Keeping SSH tunnels open to { $count } machine(s) from { $file }.
cli-hub-ssh-skipped = { $count } line(s) of { $file } were not understood and are ignored. Each line is `NAME DESTINATION [PORT]`.
cli-hub-ssh-none = No SSH machines yet. Add one with "agora hub ssh add NAME DESTINATION".
cli-hub-ssh-bad = That machine cannot be added. The name needs letters, digits, "." "_" or "-" and must be new; the destination cannot start with "-" or contain spaces.
cli-hub-ssh-added = Added { $name } to { $file }. Restart the hub to open its tunnel (the machine's host key must already be in your known_hosts). Then, on { $name }, copy the hub's CA ({ $ca }) and run: agora node enroll --hub wss://localhost:{ $port } --ca CA_FILE --code { $code } --id { $name }   (the code is valid for 10 minutes), then: agora node run --project PATH
cli-hub-ssh-unknown = No SSH machine named { $name }. Add it first with "agora hub ssh add NAME DESTINATION".
cli-hub-ssh-install-plan = Install the node on { $name } ({ $destination }), version { $version }, from { $base }. Steps: check what is there, get the release binary (checked before use), enroll through the tunnel, run as a user service. Nothing needs root. Continue? [y/N]
cli-hub-ssh-install-cancelled = Nothing was changed.
cli-hub-ssh-install-check = 1/4 Checking the machine…
cli-hub-ssh-install-binary = 2/4 Getting and checking the release binary…
cli-hub-ssh-install-enroll = 3/4 Enrolling the node (the tunnel must be up: the hub has to be running)…
cli-hub-ssh-install-run = 4/4 Starting the node…
cli-hub-ssh-install-done = Done: { $name } is enrolled and its node runs as a { $service } user service. Approvals there still need you to trust a device on { $name } ("agora node trust").
cli-hub-ssh-install-manual = { $name } is enrolled, but it has neither systemd nor launchd for user services. Start the node there yourself, for example: { $command }
cli-hub-ssh-install-failed = Stopped at step "{ $step }" on { $name }: { $detail } Already done: { $done }. Nothing was rolled back.
cli-update-current = Agora { $version } is the newest release.
cli-update-available = Agora { $version } is available (this is { $current }). Run "agora update" to install it.
cli-update-done = Updated to Agora { $version }. Running agents keep going; restart the node to use the new version.
cli-update-failed = The update did not happen: { $reason }
cli-update-checksum = The download does not match its published checksum. Nothing was changed.
cli-update-signature = The download's signature is not valid for the pinned release key. Nothing was changed.
cli-update-needs-key = The release has to be signed: save its key as release-key in the Agora folder (or pass --release-key FILE), or accept a checksum only with --unsigned. Nothing was changed.
cli-adapter-failed = The adapter was not installed: { $reason }
cli-adapter-no-registry = No registry is set. Pass --registry URL, or put its address in registry-url in the hub folder. Nothing was changed.
cli-adapter-no-key = The registry's root key is not pinned: save it as registry-root-key in the Agora folder (or pass --root-key FILE). Nothing was changed.
cli-adapter-exists = A manifest with this name is already in the manifests folder and was not installed from a registry. Nothing was changed.
cli-adapter-about = { $name } { $version }, published by { $publisher } (key { $key }), checksum { $hash }. It runs: { $command }
cli-adapter-ask-new = You have not installed anything from this publisher before. Install it? [y/N]
cli-adapter-ask-known = Install it? [y/N]
cli-adapter-declined = Nothing was installed.
cli-adapter-installed = Installed { $name } { $version }. It starts only when an agent of its kind is started.
cli-adapter-removed = Removed { $name }.
cli-adapter-current = { $name } { $version } is already the newest version in the registry.
cli-adapter-none = No adapters are installed.
cli-adapter-listed = { $name } { $version } (publisher { $publisher })
cli-adapter-listed-revoked = { $name } { $version } (publisher { $publisher }, revoked in the newest registry index: remove it or find a new publisher)

cli-project-identity = Project identity: { $id } (folder: { $path }). Share .agent-project.json with the other copies of this project.

cli-panel = Open the hub's panel at { $url } in a browser. This machine's browser must trust the hub's certificate: import { $ca } as a certificate authority (on Windows: certutil -user -addstore Root { $ca }). Sign in with a code from "agora hub login" on the hub machine.
cli-panel-none = This machine is not enrolled with a hub, so there is no panel to open. Enroll it first with "agora node enroll".

# ADR-0062: plain `agora` (the front door), `agora web`, `agora status`, `agora down`.
frontdoor-welcome = Welcome to Agora. What should this machine do?
frontdoor-choice-here = 1) Start Agora here: your agents run on this machine and you open them from here
frontdoor-choice-alone = 2) Just this machine, alone
frontdoor-choice-join = 3) Join an Agora that runs on another machine
frontdoor-choose = Type 1, 2 or 3 and press Enter:
frontdoor-lan-question = Let other machines of yours connect? [y/N]
frontdoor-join-later = Joining from here is not available yet. On the machine that runs Agora, run `agora hub code`; then here run `agora node enroll` and `agora node run` (see `agora --help`).
frontdoor-get-started = Agora is not set up on this machine yet. Run `agora` in a terminal and answer one question. (Settings are kept in { $path }.)
frontdoor-set-up-by-hand = This machine already has an Agora started by hand ({ $path }). `agora` does not take it over yet: keep using `agora hub run` and `agora node run`, or set AGORA_HOME to a new folder to let `agora` set one up there.
frontdoor-saved = Saved in { $path }.
frontdoor-bad-machine-file = { $path } could not be read. Fix it, or delete it and run `agora` again to answer the first question anew.
frontdoor-not-set-up = Agora is not set up on this machine yet. Run `agora` first.
frontdoor-no-port = No free port was found from { $port } on.
frontdoor-failed = Agora could not start: { $reason }
frontdoor-role-hub = hub
frontdoor-role-node = node
frontdoor-starting = Starting the { $role }…
frontdoor-not-ready = The { $role } did not start within { $seconds } s. What it wrote is in { $log }
frontdoor-not-linked = This machine did not connect to its hub within { $seconds } s. What it wrote is in { $log }
frontdoor-running = Agora is running on this machine. It keeps running when you close this window; `agora status` shows it, `agora down` stops it.
frontdoor-web-failed = Could not sign the browser in: { $reason }. Run `agora web` again; `agora status` shows whether Agora is running.
frontdoor-web-opened = Opened Agora in your browser, signed in.
frontdoor-web-url = Open this address in your browser to sign in (it works once, for 10 minutes; if it expired, run `agora web` again): { $url }
status-machine = This machine's Agora ({ $path }); interface: { $ui }
status-running = The { $role } is running (process { $pid }, version { $version })
status-running-other = The { $role } is running (it was not started by `agora`)
status-stopped = The { $role } is not running
status-hub = listens on { $addr }; data folder { $dir }
status-node-linked = connected to { $hub } as { $id }
status-node-not-linked = not connected to a hub
status-agents = agents running: { $count }
status-log = log: { $log }
down-stopped = Stopped the { $role } (process { $pid }).
down-failed = Could not stop the { $role }: { $reason }
down-nothing = Nothing to stop: `agora` did not start anything that is still running.
cli-node-policy-not-understood = node.policy_not_understood: the policy file has a line this node does not understand ({ $line }), so no agent may start without asking until it is fixed.
