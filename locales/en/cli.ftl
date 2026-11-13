# Command-line output. Machine-readable data (ids, states, paths) is printed as-is; only sentences live here.

cli-usage =
    Usage:
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
      agora hub notify [--ntfy URL | --webhook URL | --test] [--hub-dir DIR]
      agora hub prices update [--url URL] [--hub-dir DIR]
      agora hub ssh [add NAME DESTINATION [--port N]] [--hub-dir DIR]
      agora hub backup [FILE] [--keep N] [--hub-dir DIR]
      agora hub restore FILE [--hub-dir DIR]
      agora hub devices [forget DEVICE_ID] [--hub-dir DIR]
      agora node enroll --hub URL --ca FILE --code CODE --id NAME [--yes --fingerprint FP]... [--include-others] [--replace]
      agora panel
      agora node trust DEVICE_ID PUBLIC_KEY_HEX [--data DIR]
      agora node trust --from-hub [--yes --fingerprint FP]... [--include-others] [--replace] [--data DIR]
      agora node untrust DEVICE_ID [--data DIR]
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
doctor-manifests-missing = No agent descriptions folder at { $detail }.
doctor-agent-installed = { $detail } is installed.
doctor-agent-missing = { $detail } is not installed on this machine.
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
cli-trusted = Device { $id } can now answer approvals on this node.
cli-trust-failed = Could not trust that device: { $reason }
cli-devices-offer = The hub lists { $count ->
        [one] a device
       *[other] { $count } devices
    } this machine does not trust yet. Compare each fingerprint with the one the window shows under "This device":
cli-devices-line =   { $name } ({ $id }) · registered { $when }
cli-devices-line-user =   { $name } ({ $id }) · of { $user } · registered { $when }
cli-devices-fingerprint =     fingerprint { $fingerprint }
cli-devices-replaces =     This machine already trusts { $id } with another key: trusting this one replaces it (only with --replace).
cli-devices-unproven = The hub lists { $id } without its own signature (proof that its key is held); it is not offered.
cli-devices-others = Devices of other people on this hub ({ $count }), trusted only with --include-others and their fingerprints:
cli-devices-others-need-flag = Nothing was trusted: these devices are not the hub owner's. To trust one, pass --yes --include-others --fingerprint FP.
cli-devices-replace-refused = Not trusted: { $id } is already trusted here with another key. Pass --replace to replace it.
cli-devices-question = Trust these devices on this machine? [y/N]
cli-devices-yes = y, yes
cli-devices-trusted = Trusted { $name } ({ $id }, { $fingerprint }) on this machine.
cli-devices-none = No device is registered on the hub yet. Sign in from the window, then run "agora node trust --from-hub" here.
cli-devices-none-new = Every device registered on the hub is already trusted on this machine.
cli-devices-declined = Nothing was trusted.
cli-devices-not-asked = Nothing was trusted: this is not an interactive terminal. Run "agora node trust --from-hub" on this machine, or pass --yes with --fingerprint for each device.
cli-devices-unmatched = No device the hub lists has the fingerprint { $fingerprint }; it was not trusted.
cli-devices-not-enrolled = This machine is not enrolled with a hub. Run "agora node enroll" first.
cli-devices-failed = Could not get the devices from the hub ({ $reason }).
cli-untrusted = Device { $id } is no longer trusted on this machine.
cli-untrust-unknown = Device { $id } was not trusted on this machine. It stays refused if the hub carries an earlier approval of it.
cli-hub-device-forgotten = Device { $id } is no longer listed. Machines keep refusing it.
cli-hub-device-not-forgotten = Device { $id } is not a removed device on this hub; only removed devices can be forgotten.
cli-hub-devices-none = No device is registered yet. A browser or the terminal client registers itself when it signs in.
cli-hub-device-line = { $name } · { $fingerprint } · { $id } · { $user } · registered { $created } · last used { $used }{ $state }
cli-device-state-revoked = {" "}· removed
cli-device-state-waiting = {" "}· waiting for approval
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
