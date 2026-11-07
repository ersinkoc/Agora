# Command-line output. Machine-readable data (ids, states, paths) is printed as-is; only sentences live here.

cli-usage =
    Usage:
      agora node run [--socket PATH] [--manifests DIR] [--project PATH]... [--data DIR] [--worktrees DIR]
      agora agent start KIND [--project NAME] [--name NAME] [--socket PATH]
      agora agent stop AGENT_ID [--socket PATH]
      agora agent list [--socket PATH]
      agora terminal attach AGENT_ID [--socket PATH]
cli-node-listening = Node is running. Control socket: { $socket }
cli-agent-started = Started agent { $agent_id } in { $worktree }
cli-agent-stopped = Stopped agent { $agent_id }
cli-agent-list-unavailable = Listing agents is not available yet.
cli-terminal-detached = Detached from { $agent_id }.
cli-attach-needs-terminal = Attaching needs an interactive terminal.
cli-node-not-running = No node is listening at { $socket }. Start one with "agora node run".
cli-unknown-project = No project named { $project } is registered on this node.
cli-ambiguous-project = More than one project is registered; choose one with --project.
cli-no-projects = This node has no projects. Start it with --project PATH.
cli-bad-arguments = The arguments were not understood.
cli-error = Error { $code }: { $message }
