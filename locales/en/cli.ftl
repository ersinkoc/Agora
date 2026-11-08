# Command-line output. Machine-readable data (ids, states, paths) is printed as-is; only sentences live here.

cli-usage =
    Usage:
      agora node run [--socket PATH] [--manifests DIR] [--project PATH]... [--data DIR] [--worktrees DIR]
      agora agent start KIND [--project NAME] [--name NAME] [--socket PATH]
      agora agent stop AGENT_ID [--socket PATH]
      agora agent list [--socket PATH]
      agora agent kinds [--socket PATH]
      agora terminal attach AGENT_ID [--socket PATH]
      agora node stdio [--socket PATH]
      agora mesh stdio --socket PATH
      agora hub run [--listen ADDR] [--hub-dir DIR] [--name HOST]... [--web DIR]
      agora hub code [--hub-dir DIR]
      agora hub login [--hub-dir DIR]
      agora hub backup [FILE] [--keep N] [--hub-dir DIR]
      agora hub restore FILE [--hub-dir DIR]
      agora node enroll --hub URL --ca FILE --code CODE --id NAME
      agora node trust DEVICE_ID PUBLIC_KEY_HEX [--data DIR]
      agora doctor [--data DIR] [--manifests DIR] [--socket PATH]
cli-node-listening = Node is running. Control socket: { $socket }
cli-agent-started = Started agent { $agent_id } in { $worktree }
cli-agent-stopped = Stopped agent { $agent_id }
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
doctor-manifest-invalid = The agent description { $detail } could not be read.
doctor-summary-ok = Everything Agora needs is in place.
doctor-summary-problems = Something Agora needs is missing. Fix the items marked PROBLEM.

cli-hub-listening = Hub is running on { $addr }. Its data folder: { $dir }
cli-hub-notify = Notifications go to { $count } target(s) from { $file }.
cli-hub-notify-skipped = { $count } line(s) of { $file } were not understood and are ignored. Each line is `ntfy URL` or `webhook URL`.
cli-hub-web = Web app: https://{ $addr }/ . Sign in with the code { $code } (valid for 10 minutes; `agora hub login` makes another). The hub's certificate is signed by its own CA, so the browser warns until you trust it.
cli-hub-login = Sign-in code for the web app: { $code } (valid for 10 minutes). Open the hub's address in a browser and enter it.
cli-trusted = Device { $id } can now answer approvals on this node. Restart the node to apply it.
cli-trust-failed = Could not trust that device: { $reason }
cli-hub-backup = Backup written to { $path }.
cli-hub-restored = Database restored from the backup into { $path }. The previous one is kept next to it as before-restore.
cli-hub-failed = The hub command failed: { $reason }
cli-hub-code = Enrollment code: { $code } (valid for 10 minutes). On the other machine run: agora node enroll --hub wss://HOST:PORT --ca { $ca } --code { $code } --id NAME
cli-enrolled = This machine is enrolled as { $id }. Start it with "agora node run"; it will connect to the hub by itself. Saved in { $dir }.
cli-enroll-refused = The hub refused to enroll this machine ({ $code }). The code may be wrong, expired or already used.
