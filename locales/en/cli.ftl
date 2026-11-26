# Command-line output. Machine-readable data (ids, states, paths) is printed as-is; only sentences live here.

cli-usage =
    Usage:
      agora [--ui tui|web|none [--save]] [--resume ask|auto [--save]]
      agora web
      agora status
      agora down [--agents]
      agora service install|remove|status
      agora node run [--socket PATH] [--manifests DIR] [--project PATH]... [--data DIR] [--worktrees DIR] [--resume auto]
      agora agent start KIND [--cwd PATH] [--worktree] [--project NAME] [--name NAME] [--setting ID=VALUE]... [--accept-project-id-change] [--socket PATH]
      agora project init [--cwd PATH] [--socket PATH]
      agora agent stop AGENT_ID [--socket PATH]
      agora agent prompt AGENT_ID TEXT... [--wait [--timeout SECS]] [--force] [--socket PATH]
      agora agent wait AGENT_ID [--until working|blocked|idle|done|exited[,...]] [--timeout SECS] [--socket PATH]
      agora events [--agent AGENT_ID] [--json] [--socket PATH]
      agora agent resume AGENT_ID [--again] [--socket PATH]
      agora agent list [--socket PATH]
      agora agent kinds [--socket PATH]
      agora agent explain KIND --file SCREEN [--size COLSxROWS] [--manifests DIR]
      agora terminal attach AGENT_ID [--socket PATH]
      agora node stdio [--socket PATH]
      agora mesh stdio --socket PATH
      agora hub run [--listen ADDR] [--hub-dir DIR] [--name HOST]... [--web DIR] [--metrics]
      agora hub code [--hub-dir DIR]
      agora hub user add NAME [--role owner|member|viewer] | list | role ID ROLE | remove ID | login ID [--hub-dir DIR]
      agora hub user projects | grant|revoke ID MACHINE/PROJECT | grant|revoke ID MACHINE FOLDER [--hub-dir DIR]
      agora hub fleet on|off|status [--hub-dir DIR]
      agora hub plugin add FILE|list|remove ID [--hub-dir DIR]
      agora hub audit export [--since MS] [--who ID] [--hub-dir DIR]
      agora hub metrics-token create NAME [--expires DAYS] | list | revoke NAME [--hub-dir DIR]
      agora hub ssh install NAME [--yes] [--unsigned] [--release-url URL] [--hub-dir DIR]
      agora hub login [--hub-dir DIR]
      agora password set|clear|status [--hub-dir DIR]
      agora remote tailscale [--serve] [--port N] | status | off [--hub-dir DIR]
      agora hub notify [--ntfy URL | --webhook URL | --test] [--hub-dir DIR]
      agora hub prices update [--url URL] | set MODEL INPUT OUTPUT [CACHE_READ [CACHE_WRITE]] | unset MODEL [--hub-dir DIR]
      agora hub prices auto on [--url URL] | auto off | show [--hub-dir DIR]
      agora hub themes on|off|status | update [--url URL] [--hub-dir DIR]
      agora hub plugins on|off|status | update [--url URL] | key set FILE | install ID [--yes] | upgrade ID [--yes] [--hub-dir DIR]
      agora hub ssh [add NAME DESTINATION [--port N]] [--hub-dir DIR]
      agora hub backup [FILE] [--keep N] [--hub-dir DIR]
      agora hub restore FILE [--hub-dir DIR]
      agora hub machines [forget NAME | forget --old] [--refuse-return] [--yes] [--hub-dir DIR]
      agora hub devices [forget DEVICE_ID] [--hub-dir DIR]
      agora pair open [--minutes N] | close | status [--hub-dir DIR]
      agora join [ADDRESS] [--id NAME] [--replace]
      agora node enroll --hub URL --ca FILE --code CODE --id NAME [--yes --fingerprint FP]... [--include-others] [--replace]
      agora panel
      agora node trust DEVICE_ID PUBLIC_KEY_HEX [--data DIR]
      agora node trust --from-hub [--yes --fingerprint FP]... [--include-others] [--replace] [--data DIR]
      agora node untrust DEVICE_ID [--data DIR]
      agora node secret set NAME | list | remove NAME [--data DIR]
      agora history purge [--data DIR]
      agora tui [--hub HOST:PORT --ca FILE --code CODE] [--hub-dir DIR]
      agora update [--check] [--version X] [--release-url URL] [--release-key FILE] [--unsigned]
      agora adapter install NAME [--registry URL] [--root-key FILE] [--yes] | update [NAME] | list | remove NAME
      agora agent install NAME --list FILE [--root-key HEX|FILE] [--go-back]
      agora plugin new ID [FOLDER] | check FOLDER|FILE | pack FOLDER [--out FILE] [--write-hashes] [--watch]
      agora doctor [--data DIR] [--manifests DIR] [--socket PATH]
      agora manual [TOPIC] [--json]
cli-node-listening = Node is running. Control socket: { $socket }
cli-node-events-set-aside = Events recorded before this machine was linked as { $node_id } (under { $old_id }) were set aside and will not be sent to the hub: { $path }
cli-agent-started = Started agent { $agent_id } in { $worktree }
cli-agent-stopped = Stopped agent { $agent_id }
cli-agent-prompted = Sent the prompt to agent { $agent_id }
cli-agent-prompt-blocked-hint = Answer the agent first, or add --force to type the text into it anyway.
cli-agent-wait-bad-state = --until takes states separated by commas: working, blocked, idle, done, exited.
cli-agent-wait-blocked = Agent { $agent_id } needs you: it is waiting for an answer.
cli-agent-wait-exited = Agent { $agent_id } is no longer running (last state: { $state }).
cli-agent-wait-timeout = Stopped waiting for agent { $agent_id } after { $seconds } s; it is { $state }.
cli-event-present = { $agent_id } ({ $kind }): { $state }
cli-event-added = { $agent_id } ({ $kind }) appeared: { $state }
cli-event-state = { $agent_id }: { $previous } → { $state }
cli-event-exited = { $agent_id } stopped (last state: { $state })
cli-event-running = { $agent_id } is running again: { $state }
cli-event-restarted = { $agent_id } was restarted ({ $restarts } so far)
cli-event-interrupted = { $agent_id } was interrupted (last state: { $state })
cli-event-removed = { $agent_id } is no longer listed
cli-agent-resumed = Resumed agent { $agent_id } in { $worktree }
cli-agent-started-again = Started agent { $agent_id } again in { $worktree } (a new conversation)
cli-agent-list-empty = This node has no agents.
cli-agent-kinds-empty = This node has no agent descriptions.
cli-terminal-detached = Detached from { $agent_id }.
cli-attach-needs-terminal = Attaching needs an interactive terminal.
cli-node-not-running = No node is listening at { $socket }. Start one with "agora node run".
cli-node-refused = The node at { $socket } refused this process. The node accepts only its own user, and never a confined agent; run the command as the user who started the node.
cli-node-needs-token = The node asks for its control token, which only the user who started the node can read, and this process could not present one. A confined agent cannot use the node's control socket.
cli-unknown-project = No project named { $project } is registered on this node.
cli-ambiguous-project = More than one project is registered; choose one with --project.
cli-no-projects = This node has no projects. Start it with --project PATH.
cli-plugin-usage = Usage: agora plugin new ID [FOLDER] | check FOLDER|FILE | pack FOLDER [--out FILE] [--write-hashes] [--watch]
cli-plugin-new-done = Made the plugin { $id } in { $folder }. Next: "agora plugin pack { $folder } --out { $id }.json --write-hashes --watch", then in the window open Plugins, Load from a file…, and choose { $id }.json. { $folder }/README.md says more.
cli-plugin-new-bad-id = "{ $id }" cannot be a plugin's id: use lower-case letters, digits and "-" (at most 40, not starting with "-").
cli-plugin-new-exists = { $folder } already exists, so nothing was written. Choose another folder.
cli-plugin-write-failed = Could not write { $path }: { $reason }
cli-plugin-ok = { $path } passes the window's checks: { $name } { $version } ({ $id }), { $panels ->
    [one] one panel
   *[other] { $panels } panels
    }, asks to see: { $capabilities }.
cli-plugin-sees-nothing = nothing
cli-plugin-refused = { $path } would be refused ({ $code }): { $reason }
cli-plugin-error-too-large = it is larger than 2 MiB, the most a plugin can be.
cli-plugin-error-format = it is not a plugin package.
cli-plugin-error-newer-format = the package was made for a newer version of Agora.
cli-plugin-error-too-many = it holds more files than a plugin may have (64 besides the manifest).
cli-plugin-error-path = a file's name has a folder, a path or a leading dot in it ({ $detail }).
cli-plugin-error-link = the package holds a link or something else that is not a file's bytes ({ $detail }).
cli-plugin-error-type = a file is of a kind a plugin cannot use ({ $detail }); scripts, stylesheets, images, fonts, JSON and text only.
cli-plugin-error-manifest = the manifest (agora-plugin.json) is missing or wrong at "{ $detail }".
cli-plugin-error-capability = the manifest asks for a capability the window does not know ({ $detail }).
cli-plugin-error-unlisted = a file is not listed in the manifest ({ $detail }).
cli-plugin-error-missing = a file the manifest lists is not there ({ $detail }).
cli-plugin-error-hash = a file is not the one the manifest lists ({ $detail }); pack with --write-hashes to write its hash.
cli-plugin-error-newer = the plugin needs Agora { $detail } or newer, and this is { $version }.
cli-plugin-error-not-a-file = the folder holds something that is not a plain file ({ $detail }): plugin folders are flat, and links are not followed.
cli-plugin-error-unreadable = it could not be read ({ $detail }).
cli-plugin-watch-needs-out = --watch needs --out FILE.
cli-plugin-out-inside = The package would be written into { $folder } itself ({ $out }); write it beside the folder.
cli-plugin-hashes-written = Wrote the files' hashes into { $folder }/agora-plugin.json.
cli-plugin-packed = Packed { $folder } into { $out } ({ $bytes } bytes).
cli-plugin-watching = Packing { $folder } again on every change. Ctrl+C stops.
cli-bad-arguments = The arguments were not understood.
# `agora manual` (G10): how to drive and customize Agora, written for coding agents. The topics themselves are in docs/manual/
# (English; see crates/agora/src/manualcmd.rs for why they are not here).
cli-manual-heading = The manual has these topics. "agora manual TOPIC" prints one; "agora manual --json" lists them as JSON.
cli-manual-unknown = The manual has no topic named "{ $topic }". Its topics: { $topics }.
cli-manual-topic-scripting = Start, prompt, wait for and stop agents from a shell; exit codes and events.
cli-manual-topic-themes = Write a theme file for the web window and the terminal client.
cli-manual-topic-layouts = Write a layout file: splits, tab groups and panes.
cli-manual-topic-keys = Write a keybindings file for the web window.
cli-manual-topic-plugins = Write a plugin: a sandboxed panel for the web window.
cli-manual-topic-mesh = The mesh tools an agent inside Agora can call.
cli-manual-topic-files = Where each file lives in the Agora folder, and how it is read.
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
doctor-git-identity-missing = Agents here have no git identity, so they cannot commit. Set "user.name" and "user.email" with "git config --global".
doctor-control-token-reachable = Agents here could read the node's control token ({ $detail }), because the node's folder lies inside a folder agents may read or write (a node folder under /tmp). Agents will not start. Move the node's folder (--data) outside the folders in the policy.
doctor-protected-path-reachable = Agents here could read this node's own files ({ $detail }), because they lie inside a folder agents may read or write (under /tmp, /var, /opt, /etc or /run, or a folder in the policy), and this system cannot keep them closed there. Agents will not start. Move the node's data folder (--data) or its agent types folder (--manifests) outside those folders, for example under your home folder.
doctor-sockets-profile = The sandbox profile on this machine refuses unix sockets agents were not given (the ssh-agent, the session bus, docker, tmux). A macOS developer measured this on a real Mac on 2026-11-17: the agent's own socket works, another socket is refused, and the name resolver works. Other system sockets (DNS in every tool, git over https, cargo, npm, Claude Code, Codex) have not been measured with real agents yet, so one that needs a socket the list does not name may fail; add it with `sandbox.sockets`.
doctor-sockets-abstract = This kernel cannot restrict unix sockets that are files: an agent can still reach an ssh-agent, a session bus, docker or tmux socket (only abstract sockets are closed). The environment scrub is the only other layer, and an agent that looks for the socket can find it.
doctor-sockets-none = This machine ({ $detail }) cannot restrict unix sockets: the environment scrub is the only layer, and an agent that looks for an ssh-agent, session bus, docker or tmux socket can find it.
doctor-env-scrubbed = Agents do not inherit these variables from the node: { $detail }. Name one in "sandbox.pass_env" in the node's policy to let it through.
doctor-env-passed = The node's policy passes these variables on to agents: { $detail }. Each can hand an agent the user's keys or sessions.
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
        [registry] from the ACP registry, untested
       *[folder] from the manifests folder
    }): `{ $program }` is found{ $by ->
        [agora] , installed by Agora (version { $version })
        [you] , installed by you
       *[other] {""}
    }.
doctor-agent-missing = { $detail } ({ $from ->
        [built-in] built in
        [registry] from the ACP registry, untested
       *[folder] from the manifests folder
    }): this machine cannot find `{ $program }` in its PATH. Install it there, or start the node from a terminal where `{ $program }` runs.
doctor-policy-path-dropped = The node's policy line "{ $detail }" names a path the node cannot use (relative, "..", or "~" without a home folder); agents start without it. Write a full path or one starting with "~/".
doctor-policy-path-missing = The node's policy line "{ $detail }" names a path that does not exist yet; agents start without it until it does.
doctor-policy-restart = The node reads its policy file { $detail } only when it starts: after you change it, restart the node ("agora node run"). Trusting or removing a device needs no restart.
doctor-policy-changed = The policy file { $detail } changed after this node started, so the change does not apply yet. Restart the node ("agora node run") to apply it.
doctor-signed-commands = Commands sent to this machine through the hub need a signature from one of your trusted devices: { $detail }.
doctor-signed-commands-team = Some devices are limited to certain projects, so every command sent through the hub that changes something needs a signature from a trusted device: { $detail }.
doctor-signed-commands-none = Commands sent to this machine through the hub (stop an agent, type into its terminal, write a file) run without a signature from your device, so whoever controls the hub could send them. Starting an agent in a folder from the window is always signed. To require a signature, add this line to { $detail } and restart the node: signed_commands = agent.start, agent.stop, agent.prompt, terminal.input, files.write
doctor-devices = { $detail ->
    [one] One device is trusted on this machine
   *[other] { $detail } devices are trusted on this machine
}; { $unlimited } of them may act in every project.
doctor-devices-none = No device is trusted on this machine yet, so it accepts no signed request: approvals and starts from the window are refused here. Trust your browser or terminal with "agora node trust --from-hub".
doctor-skip-permissions = { $detail ->
    [allow] Agents may be started in a mode that skips their permission questions, after your confirmation and signature (policy: skip_permissions = allow).
    [confined_only] Agents may skip their permission questions only while they are confined (policy: skip_permissions = confined_only).
   *[other] No agent may be started in a mode that skips its permission questions (policy: skip_permissions = forbid).
}
doctor-policy-problems = { $detail ->
    [one] One line
   *[other] { $detail } lines
} of the node's policy file could not be understood; until they are fixed, no agent may skip its permission questions.
doctor-show-secret = The file browser and the review screen show secret files (.env files, private keys, ...) in full to everyone who may look at a project (policy: files.show_secret = true).
doctor-agent-broken = { $detail } ({ $from ->
        [built-in] built in
        [registry] from the ACP registry, untested
       *[folder] from the manifests folder
    }): `{ $program }` is there but this account may not run it. Make the file executable, or reinstall it.
doctor-agent-unsafe = { $detail }: `{ $program }` was installed by Agora, but its folder or file is not safe (another user could change it), so it is not used: using the one on your PATH, if there is one. Reinstall it with `agora agent install`, or make the install folders yours alone.
doctor-registry-kinds = { $detail ->
        [one] 1 agent
       *[other] { $detail } agents
    } of the ACP registry are known here, untested in Agora; { $ready } of them find their program on this machine. Settings → Agents in the window lists them all.
doctor-runner-folders-left = { $detail ->
        [one] 1 folder
       *[other] { $detail } folders
    } that agents of the ACP registry started in could not be removed in full (deeper or larger than the node removes, or with names it does not use); they stay in the data folder's `runner` folder, named `….left`, and can be deleted by hand.
doctor-registry-off = The ACP registry's agents are switched off on this machine (policy line `agents.registry = off`): it neither offers nor starts them. Your own agent descriptions are not affected.
doctor-registry-denied = { $detail ->
        [one] 1 agent of the ACP registry is
       *[other] { $detail } agents of the ACP registry are
    } switched off on this machine by the policy line `agents.registry.deny`.
doctor-manifest-older = The manifests folder holds an older description of { $detail } (version { $from }; this program has { $shipped } built in) and it replaces the built-in one, so agents of this kind may not resume after a restart. Remove the file { $file } from the manifests folder: the shipped descriptions are built in.
doctor-holder-old = Chat agents on this machine are resumed after a node restart, not kept running, because this machine's agent keeper is older than this program. It ends when the agents it keeps have ended ({ $detail ->
    [one] 1 agent now
   *[other] { $detail } agents now
}), and a new one then starts.
doctor-manifest-invalid = The agent description { $detail } could not be read.
doctor-cert-node = This machine's hub certificate is valid for { $detail ->
    [one] one more day
   *[other] { $detail } more days
}. It renews itself when a third of its life is left, while this machine is connected to the hub.
doctor-cert-node-soon = This machine's hub certificate ends in { $detail ->
    [one] one day
   *[other] { $detail } days
}. It should have renewed itself by now: it does so when this machine is connected to a hub that is up to date. If this stays so, check that the hub is running and reachable from here; otherwise this machine cannot connect to the hub once it ends, and must join again (on the hub run "agora hub code", then here run "agora node enroll" with the new code).
doctor-cert-node-expired = This machine's hub certificate has ended, so it cannot connect to the hub ({ $file }). Join it again: on the hub run "agora hub code", then here run "agora node enroll" with the new code.
doctor-cert-node-unusable = This machine's hub certificate cannot be read as a certificate ({ $file }). Check that the file is whole and readable by this user; if it is damaged, join this machine again: on the hub run "agora hub code", then here run "agora node enroll" with the new code.
doctor-cert-renew-join-again = The hub would not renew this machine's hub certificate ({ $detail }), so when it ends this machine cannot connect. Join this machine again with the owner's approval: on the hub run "agora hub code" (or pair it), then here run "agora node enroll" with the new code. Its agents and its waiting events are not touched. { $asked ->
    [yes] The hub was last asked on { $when }; it is asked again about once a day, so if the hub was at fault (restored from a backup, a wrong clock) this clears by itself.
   *[no] It is asked again about once a day, so if the hub was at fault (restored from a backup, a wrong clock) this clears by itself.
}
doctor-cert-renew-superseded = This machine's hub certificate is no longer the newest one the hub issued for it: another holder of this machine's certificate may exist. Ask the owner to check the hub's audit view for this machine; if you do not know why, remove this machine on the hub ("agora hub machines") and join it again. Until then it keeps working with this certificate until it ends. { $asked ->
    [yes] The hub was last asked on { $when }; it is asked again about once a day.
   *[no] It is asked again about once a day.
}
doctor-cert-renew-rejected = The hub answered this machine's certificate renewal with a certificate that did not check out ({ $detail }), so it was not used and the certificate in use is unchanged. It is asked again after an hour, and after five times in a row with a new request. If this repeats, make sure this machine reaches the hub it enrolled with.
doctor-cert-renew-hub-old = The hub this machine is connected to is older than certificate renewal, so this machine's certificate cannot renew itself yet. Update the hub (it is asked again about once an hour); until then, join this machine again before its certificate ends.
doctor-cert-renew-pending = A new certificate for this machine is being fetched; the hub has not answered yet. This machine keeps working with the certificate it has, and asks again by itself.
doctor-cert-hub = The hub's own certificate is valid for { $detail ->
    [one] one more day
   *[other] { $detail } more days
}. The hub replaces it by itself when 30 days are left, without a restart.
doctor-cert-hub-soon = The hub's own certificate ends in { $detail ->
    [one] one day
   *[other] { $detail } days
}. The hub replaces it by itself when under 30 days are left (it checks every day), so this file is older than it should be. If this stays so, restart the hub ("agora hub run", or restart its service); a hub that keeps running past the end refuses new connections.
doctor-cert-hub-expired = The hub's own certificate has ended ({ $file }): new connections to it are refused. Restart the hub ("agora hub run", or restart its service) to get a new one.
doctor-cert-hub-unusable = The hub's own certificate cannot be read as a certificate ({ $file }). Check that the file is whole and readable by the user the hub runs as; restarting the hub ("agora hub run", or restart its service) writes a new one.
doctor-summary-ok = Everything Agora needs is in place.
doctor-summary-problems = Something Agora needs is missing. Fix the items marked PROBLEM.

cli-hub-listening = Hub is running on { $addr }. Its data folder: { $dir }
cli-hub-names = The hub's certificate names: { $names }.
cli-hub-names-unlisted = This computer would not list its network addresses, so the hub's certificate does not name them yet. `agora pair open` says what to do if that is still so.
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
cli-password-set = Screen password set. A sign-in from another machine now needs it; this machine's own browser is never asked. { $count ->
    [one] 1 open session was signed out.
   *[other] { $count } open sessions were signed out.
  }
cli-password-cleared = Screen password removed. A sign-in code alone is enough again. { $count ->
    [one] 1 open session was signed out.
   *[other] { $count } open sessions were signed out.
  }
cli-password-on = A screen password is set: sign-ins from other machines need it.
cli-password-off = No screen password is set: a sign-in code alone is enough.
cli-trusted = Device { $id } can now answer approvals on this node.
cli-trust-failed = Could not trust that device: { $reason }
cli-trust-revocation-overridden = Device { $id } was revoked by { $revoker }. It stays trusted here even when that revocation is sent again.
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
cli-node-in-use = Another node already runs on the data folder { $dir } (or on its control socket). Nothing was read or changed. Stop that node first, or use another data folder.
cli-link-busy = The link folder { $dir } is held by another Agora command that has not finished in time (stopped, or not answering). Nothing was read from it. End that command, then try again.
cli-link-unreadable = The link folder { $dir } could not be read ({ $reason }). Nothing was read from it.
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
cli-hub-not-here = This machine does not run the hub (there is no { $dir }), so there is nothing to show here. Run this command on the machine that runs the hub, or open the window.
cli-hub-fingerprint = The hub CA fingerprint (SHA-256): { $fingerprint }
cli-hub-user-added = Added { $id } as { $role }. Their sign-in code: { $code } (valid for 10 minutes). Have them open the hub's address and enter it; `agora hub user login { $id }` makes another.
cli-hub-user-none = No team members yet. Add one with `agora hub user add NAME --role member`.
cli-hub-fleet-solo = Agents see every machine and agent of this hub: nobody but you uses it. Once you add people, they see them only if you run `agora hub fleet on`.
cli-hub-fleet-solo-on = Agents see every machine and agent of this hub: nobody but you uses it. The switch is on, so once you add people their agents will see everything too, also in projects they have no access to. `agora hub fleet off` turns it off.
cli-hub-fleet-user-warning = `agora hub fleet` is on for this hub, so this person's agents see every machine and agent, also in projects they have no access to. `agora hub fleet off` turns it off.
cli-hub-fleet-on = Agents see every machine and agent of this hub, also in projects their person has no access to. `agora hub fleet off` turns it off.
cli-hub-fleet-off = Agents see only the machines and agents of their own projects, because other people use this hub. `agora hub fleet on` lets them see every machine and agent.
cli-hub-plugin-added = Installed { $name } { $version } ({ $id }) for this hub. It asks to see: { $capabilities }. Each person chooses in the window whether to show it and what it may see.
cli-hub-plugin-replaced = Installed { $name } { $version } ({ $id }) for this hub, in place of { $old }. It asks to see: { $capabilities }. Anyone who allowed the old version is asked again in the window if this one asks to see more.
cli-hub-plugin-line = { $id } { $version }  { $name }  asks to see: { $capabilities }  (installed { $installed })
cli-hub-plugin-none = No plugins are installed on this hub. `agora hub plugin add FILE` installs one.
cli-hub-plugin-removed = Removed { $id } from this hub, with what each person allowed it. Windows leave it out the next time they load.
cli-hub-plugin-unknown = No plugin { $id } is installed on this hub. `agora hub plugin list` shows them.
cli-hub-plugin-full = This hub keeps at most { $limit } plugins. Remove one first (`agora hub plugin remove ID`).
cli-hub-plugin-unreadable = Could not read { $file }.
cli-hub-plugin-error-too-large = This file is larger than 2 MB, the most a plugin can be. Nothing was installed.
cli-hub-plugin-error-format = This file is not a plugin package. Nothing was installed.
cli-hub-plugin-error-newer-format = This plugin package was made for a newer version. Nothing was installed.
cli-hub-plugin-error-too-many = This package holds more files than a plugin may have. Nothing was installed.
cli-hub-plugin-error-path = This package names a file with a folder or path in it ({ $detail }). Nothing was installed.
cli-hub-plugin-error-link = This package holds a link or something else that is not a file ({ $detail }). Nothing was installed.
cli-hub-plugin-error-type = This package holds a kind of file a plugin cannot use ({ $detail }). Nothing was installed.
cli-hub-plugin-error-manifest = The package's description (agora-plugin.json) is missing or wrong at "{ $detail }". Nothing was installed.
cli-hub-plugin-error-capability = This plugin asks to see something Agora does not know ({ $detail }). Nothing was installed.
cli-hub-plugin-error-unlisted = This package holds a file its description does not list ({ $detail }). Nothing was installed.
cli-hub-plugin-error-missing = A file the package's description lists is missing ({ $detail }). Nothing was installed.
cli-hub-plugin-error-hash = A file in this package is not the one its description lists ({ $detail }). Nothing was installed.
cli-hub-plugin-error-newer = This plugin needs Agora { $detail } or newer. Nothing was installed.
cli-hub-user-line = { $id }  { $name }  { $role }
cli-hub-user-unknown = There is no team member called { $id }.
cli-hub-user-project =   works in { $project }
cli-hub-user-project-path =   works in { $project } ({ $path })
cli-hub-user-projects-none = No machine has reported a project to this hub yet.
cli-hub-user-projects-line = { $name }    { $path }
cli-hub-user-projects-line-host = { $name }    { $path }    (machine { $host })
cli-hub-user-unknown-machine = No machine called { $machine } has reported to this hub. `agora hub user projects` lists the projects you can give access to.
cli-hub-user-unknown-project = This hub knows no project called { $project } on { $machine }. `agora hub user projects` lists the projects you can give access to.
cli-hub-user-ambiguous-project = More than one project fits { $project }. Choose one by its machine and folder, for example `agora hub user grant { $id } { $machine } { $path }`:
cli-hub-user-not-granted = { $id } has no access to { $project }.
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
cli-hub-prices-auto-running = The hub fetches the price table from { $url } by itself, about once a day ("agora hub prices auto off" stops it).
cli-hub-prices-bad-url = That is not an http:// or https:// address.
cli-hub-prices-status = The price catalog's server answered with status { $status }, so nothing was changed. A redirect is not followed: give its final address with --url.
cli-hub-prices-not-catalog = The answer is not a price catalog (or is too large), so nothing was changed.
cli-hub-prices-shrunk = The catalog has prices for only { $count } models, fewer than half of the table in use ({ $floor } is the least kept), so it was not used.
cli-hub-prices-store-failed = Could not write to the hub's folder: { $reason }
cli-hub-prices-set = The hub now prices { $model } at your price. A running hub uses it within a few seconds.
cli-hub-prices-unset = Your price for { $model } is removed; the catalog's price applies again. A running hub uses it within a few seconds.
cli-hub-prices-no-override = There is no price of yours for { $model }.
cli-hub-prices-bad-model = A model name must be 1 to 128 characters without spaces, for example anthropic/claude-sonnet-4-5.
cli-hub-prices-bad-price = A price is in US dollars per million tokens, a number from 0 to 1000000.
cli-hub-prices-too-many = Your price list already has { $count } models, the most it can hold. Remove one with "agora hub prices unset MODEL".
cli-hub-prices-overrides-unreadable = { $file } is not a list of prices this command can change, so it was left as it is. Fix or remove it, then try again.
cli-hub-prices-auto-on = The hub will fetch the price table from { $url } by itself, about once a day, starting within a few minutes if its table is older than a day. A running hub starts within a few seconds.
cli-hub-prices-auto-off = The hub no longer fetches the price table by itself. "agora hub prices update" still does it once when you ask.
cli-hub-prices-show-bundled = Prices: the table that came with this version ({ $count } models).
cli-hub-prices-show-refreshed = Prices: the table fetched at { $at } ({ $count } models).
cli-hub-prices-show-auto-off = Fetching by itself: off ("agora hub prices auto on" turns it on).
cli-hub-prices-show-auto-on = Fetching by itself: on, from { $url }, about once a day.
cli-hub-prices-show-last-ok = Last fetch: { $at }, done.
cli-hub-prices-show-last-failed = Last fetch: { $at }, failed ({ $code }); it is tried again later.
cli-hub-prices-show-no-overrides = Your prices: none.
cli-hub-prices-show-override = Your price for { $model }: input { $input }, output { $output }, cache read { $cache_read }, cache write { $cache_write } (USD per million tokens).
cli-hub-prices-show-override-bad = Your price for { $model } is not understood and is ignored (it needs input and output).
cli-hub-themes-on = The theme catalog is on: everyone signed in to this hub can browse and install its themes in the theme editor. `agora hub themes off` hides it again.
cli-hub-themes-off = The theme catalog is off: windows show no catalog. `agora hub themes on` lets everyone signed in to this hub browse and install its themes.
cli-hub-themes-never-updated = Nothing fetched yet. `agora hub themes update --url URL` fetches the catalog's index (the project's catalog repository does not exist yet, so give its address).
cli-hub-themes-updated-at = { $count } themes, fetched { $when } from { $url }.
cli-hub-themes-no-url = No catalog address yet. Give the address of the catalog's index.json with `--url` (the project's catalog repository does not exist yet); later updates use it again.
cli-hub-themes-updated = Theme catalog updated: { $kept } themes kept, { $refused } refused.
cli-hub-themes-still-off = The catalog is off, so windows do not show it yet. `agora hub themes on` turns it on.
cli-hub-themes-refused = Refused { $id }: { $reason }
cli-hub-themes-problem = {"  "}{ $text }
cli-hub-themes-failed = Could not update the theme catalog: { $reason } Nothing was changed.
cli-hub-themes-theme-unreachable = the file of { $id } could not be fetched: { $reason }
cli-hub-themes-index-address = the address is not https:// (plain http:// is allowed only to this machine).
cli-hub-themes-index-fetch = it could not be reached.
cli-hub-themes-index-timeout = it took too long to answer.
cli-hub-themes-index-status = it answered with status { $status } (a redirect is not followed: give the final address).
cli-hub-themes-index-too-large = it is larger than { $max } KiB.
cli-hub-themes-index-json = it is not JSON.
cli-hub-themes-index-format = it is not a theme catalog index (it needs "agora-theme-catalog": 1 and a "themes" list).
cli-hub-themes-index-newer = it is in a newer format than this version of Agora reads. Update Agora.
cli-hub-themes-index-unknown-field = it has a field this version does not know: { $field }.
cli-hub-themes-index-too-many = it lists more than { $max } themes.
cli-hub-themes-refused-entry = its entry in the index has no valid { $field }.
cli-hub-themes-refused-unknown-field = its entry in the index has a field this version does not know: { $field }.
cli-hub-themes-refused-duplicate = another theme in the index has the same id.
cli-hub-themes-refused-status = its file answered with status { $status } (a redirect is not followed).
cli-hub-themes-refused-too-large = its file is larger than { $max } KiB.
cli-hub-themes-refused-total = the catalog's themes together would be larger than { $max } KiB.
cli-hub-themes-refused-hash = its file does not have the SHA-256 the index gives.
cli-hub-themes-refused-not-text = its file is not UTF-8 text.
cli-hub-themes-refused-theme = it is not a theme Agora can use:
cli-hub-plugins-on = The plugin catalog is on: windows list its plugins as available, and you can install them on this machine with `agora hub plugins install ID`. `agora hub plugins off` hides it again.
cli-hub-plugins-off = The plugin catalog is off: windows show no catalog and nothing is installed from it. `agora hub plugins on` turns it on.
cli-hub-plugins-key-is = Catalog key: { $fingerprint }. Every index is checked against it before anything in it is used.
cli-hub-plugins-no-key = No catalog key is set, so the catalog cannot be used. Set the key the catalog publishes with `agora hub plugins key set FILE` (its .pub file; compare the fingerprint with the one the project publishes).
cli-hub-plugins-key-set = Catalog key set: { $fingerprint }. Compare it with the fingerprint the project publishes. If it replaced another key, the catalog fetched under the old one is forgotten: run `agora hub plugins update` again.
cli-hub-plugins-key-private = This is a private key. Give the public key (the .pub file); nothing was kept.
cli-hub-plugins-key-invalid = This is not one ssh-ed25519 public key (an `ssh-ed25519 AAAA…` line, or an allowed-signers line with one). Nothing was kept.
cli-hub-plugins-key-unreadable = Could not read { $file }.
cli-hub-plugins-never-updated = Nothing fetched yet. `agora hub plugins update --url URL` fetches the catalog's index (the project's catalog repository does not exist yet, so give its address).
cli-hub-plugins-updated-at = { $count } plugins (index serial { $serial }), fetched { $when } from { $url }.
cli-hub-plugins-no-url = No catalog address yet. Give the address of the catalog's index.json with `--url` (the project's catalog repository does not exist yet); later updates use it again.
cli-hub-plugins-updated = Plugin catalog updated: { $kept } plugins kept, { $refused } refused (index serial { $serial }). Nothing was installed or upgraded.
cli-hub-plugins-newer = The catalog has a newer { $id }: { $old } → { $new }. `agora hub plugins upgrade { $id }` shows what changes and asks before it upgrades.
cli-hub-plugins-still-off = The catalog is off, so windows do not list it and nothing is installed from it. `agora hub plugins on` turns it on.
cli-hub-plugins-failed = Could not update the plugin catalog: { $reason } Nothing was changed.
cli-hub-plugins-refused = Refused { $id }: { $reason }
cli-hub-plugins-refused-field = its entry in the index has no valid { $field }.
cli-hub-plugins-refused-unknown-field = its entry in the index has a field this version does not know: { $field }.
cli-hub-plugins-refused-duplicate = another plugin in the index has the same id.
cli-hub-plugins-index = the index { $reason }
cli-hub-plugins-signature-fetch = the index's signature (index.json.sig) { $reason }
cli-hub-plugins-fetch-address = has no usable address: it must be https:// (plain http:// only to this machine).
cli-hub-plugins-fetch-fetch = could not be reached.
cli-hub-plugins-fetch-timeout = took too long to answer.
cli-hub-plugins-fetch-status = answered with status { $status } (a redirect is not followed: give the final address).
cli-hub-plugins-fetch-too-large = is larger than { $max } KiB.
cli-hub-plugins-index-json = is not JSON.
cli-hub-plugins-index-format = is not a plugin catalog index (it needs "agora-plugin-catalog": 1, a "serial" and a "plugins" list).
cli-hub-plugins-index-newer = is in a newer format than this version of Agora reads. Update Agora.
cli-hub-plugins-index-unknown-field = has a field this version does not know: { $field }.
cli-hub-plugins-index-too-many = lists more than { $max } plugins.
cli-hub-plugins-signature-malformed = the index's signature is not an SSH signature this version reads.
cli-hub-plugins-signature-wrong-key = the index is signed by another key than the catalog key set on this hub.
cli-hub-plugins-signature-namespace = the index's signature was made for another use, not for the plugin catalog.
cli-hub-plugins-signature-invalid = the index's signature does not match it: the index or its signature was changed after it was signed.
cli-hub-plugins-rollback = the index is older (serial { $offered }) than the last one this hub took (serial { $seen }), so it may name versions since withdrawn.
cli-hub-plugins-serial-reused = the index has the serial of the last one this hub took (serial { $serial }) but other contents: a signed index is never changed without a new serial.
cli-hub-plugins-not-installed = Nothing was installed: { $reason }
cli-hub-plugins-line = {"  "}{ $text }
cli-hub-plugins-install-asks = { $id } asks to see: { $capabilities }
cli-hub-plugins-install-what = { $name } { $version }, by { $author }, licence { $licence }, from the plugin catalog; its package was checked against the signed index.
cli-hub-plugins-upgrade-asks = What { $id } asks to see changes:
cli-hub-plugins-caps-added = asks to see more: { $capabilities }
cli-hub-plugins-caps-removed = no longer asks for: { $capabilities }
cli-hub-plugins-caps-same = asks for nothing new
cli-hub-plugins-caps-kept = still asks for: { $capabilities }
cli-hub-plugins-upgrade-version = { $name } { $old } → { $new }, by { $author }, licence { $licence }; its package was checked against the signed index.
cli-hub-plugins-confirm = Install it on this hub? [y/N]
cli-hub-plugins-needs-yes = Nothing was installed: there is no terminal to ask in. Add --yes once you have read the above.
cli-hub-plugins-declined = Nothing was installed.
cli-hub-plugins-installed = Installed { $name } { $version } ({ $id }) from the catalog. Each person chooses in the window whether to show it and what it may see.
cli-hub-plugins-upgraded = Upgraded { $name } ({ $id }) to { $version }. Anyone who allowed the old version is asked again in the window if this one asks to see more.
cli-hub-plugins-install-off = the plugin catalog is off. `agora hub plugins on` turns it on.
cli-hub-plugins-install-not-verified = the catalog has not been fetched under the key set now. Run `agora hub plugins update` first.
cli-hub-plugins-install-not-in-catalog = the catalog has no plugin { $id }. `agora hub plugins update` fetches its newest index.
cli-hub-plugins-install-installed = { $id } { $version } is installed already. `agora hub plugins upgrade { $id }` takes a newer version.
cli-hub-plugins-install-not-installed = { $id } is not installed. `agora hub plugins install { $id }` installs it.
cli-hub-plugins-install-not-newer = the catalog's { $id } ({ $offered }) is not newer than the one installed ({ $installed }).
cli-hub-plugins-install-fetch = its package { $reason }
cli-hub-plugins-install-size = its package does not have the size the signed index gives.
cli-hub-plugins-install-hash = its package does not have the SHA-256 the signed index gives: it was changed after the index was signed.
cli-hub-plugins-install-mismatch = its package's description says another { $field } than the signed index.
cli-hub-plugins-install-changed = the catalog or the plugins installed on this hub changed while you were asked. Run the command again to see what it would install now.
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
cli-update-local-network = macOS may ask again whether Agora may use the local network: allow it (or turn `agora` on in System Settings > Privacy & Security > Local Network), then restart the node once more. The node that is running keeps failing to reach a hub on another machine until it is restarted after you allow it; `agora status` says whether it is connected.
cli-update-done = Updated to Agora { $version }. Running agents keep going; restart the node to use the new version.
cli-update-failed = The update did not happen: { $reason }
cli-update-check-failed = Could not check for updates. The newest release could not be found.
cli-update-release-not-found = The update did not happen: could not find the newest release.
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

cli-panel = The hub's panel is open at { $url } on this machine. Open that address in a browser; no certificate is needed. Sign in with a code from "agora hub login" on the hub machine, as from any browser. Press Ctrl+C to close it.
cli-panel-none = This machine is not enrolled with a hub, so there is no panel to open. Enroll it first with "agora node enroll".
cli-panel-link = This machine's hub address or certificate is unreadable. Enroll it again with "agora node enroll".
cli-panel-port-taken = The panel's port { $port } is taken by another program. Close that program and run "agora panel" again; the port stays the same so that your sign-in and device are kept.
cli-panel-no-port = The panel could not open a port on this machine.
cli-panel-port-unsaved = The panel's port could not be saved, so the next "agora panel" may use another port, and your browser would treat it as a new device and ask you to sign in again. Check that this machine's node folder is writable.
cli-panel-unreachable = The hub does not answer from this machine, so there is no panel to open. The agents here keep running.
cli-panel-untrusted = The hub's certificate is not from the certificate this machine was enrolled with, so the panel was not opened.
cli-panel-opened = Opened it in your browser.

# ADR-0062: plain `agora` (the front door), `agora web`, `agora status`, `agora down`.
frontdoor-welcome = Welcome to Agora. What should this machine do?
frontdoor-choice-here = 1) Start Agora here: your agents run on this machine and you open them from here
frontdoor-choice-alone = 2) Just this machine, alone
frontdoor-choice-join = 3) Join an Agora that runs on another machine
frontdoor-choose = Type 1, 2 or 3 and press Enter:
frontdoor-lan-question = Let other machines of yours connect? [y/N]
frontdoor-autostart-question = Start Agora when you log in? [Y/n]
frontdoor-autostart-later = Agora will not start by itself when you log in. `agora service install` sets that up whenever you want.
frontdoor-resume-question = Bring agents back by themselves after a restart? [Y/n]
frontdoor-resume-off = Agents will not come back by themselves after a restart; the window offers to bring each one back. `agora --resume auto --save` turns it on again.
frontdoor-resume-saved = Saved. It applies from the next time Agora starts on this machine: a node running now keeps what it started with until `agora down`.
frontdoor-get-started = Agora is not set up on this machine yet. Run `agora` in a terminal and answer one question. (Settings are kept in { $path }.)
frontdoor-set-up-by-hand = This machine already has an Agora started by hand ({ $path }). `agora` does not take it over yet: keep using `agora hub run` and `agora node run`, or set AGORA_HOME to a new folder to let `agora` set one up there.
frontdoor-saved = Saved in { $path }.
frontdoor-bad-machine-file = { $path } could not be read. Fix it, or delete it and run `agora` again to answer the first question anew.
frontdoor-not-set-up = Agora is not set up on this machine yet. Run `agora` first.
frontdoor-no-port = No free port was found from { $port } on.
frontdoor-failed = Agora could not start: { $reason }
frontdoor-lock-busy = Another process has held { $path } for more than 10 seconds (it may be stopped or hung), so Agora did not change what runs. Find it with `agora status`, or end it, and try again.
frontdoor-lock-unsafe = { $path } (or its folder) is a link, a second name of another file, or not a plain file or folder, so Agora did not use it. Remove it, and Agora makes a new one.
frontdoor-lock-not-yours = { $path } (or its folder) belongs to another user, so Agora did not use it. Remove it, or give it back to your user.
frontdoor-role-hub = hub
frontdoor-role-node = node
frontdoor-starting = Starting the { $role }…
frontdoor-in-job = The { $role } could not leave the job of the terminal that started it, so it ends when that terminal closes. Start `agora` from a terminal outside that job (or a service) to keep it running.
frontdoor-not-ready = The { $role } did not start within { $seconds } s. What it wrote is in { $log }
frontdoor-not-linked = This machine did not connect to its hub within { $seconds } s. What it wrote is in { $log }
frontdoor-running = Agora is running on this machine. It keeps running when you close this window; `agora status` shows it, `agora down` stops it.
frontdoor-web-failed = Could not sign the browser in: { $reason }. Run `agora web` again; `agora status` shows whether Agora is running.
frontdoor-web-opened = Opened Agora in your browser, signed in.
frontdoor-web-url = Open this address in your browser to sign in (it works once, for 10 minutes; if it expired, run `agora web` again): { $url }
status-machine = This machine's Agora ({ $path }); interface: { $ui }
status-running = The { $role } is running (process { $pid }, version { $version })
status-running-supervised = The { $role } is running (process { $pid }, version { $version }; its supervisor is process { $supervisor })
status-running-other = The { $role } is running (it was not started by `agora`)
status-stopped = The { $role } is not running
status-hub = listens on { $addr }; data folder { $dir }
status-node-linked = connected to { $hub } as { $id }
status-node-not-linked = not connected to a hub
status-node-connecting = not connected to { $hub } yet: the node started and is still trying
status-node-link-unknown = connection to { $hub } is not known: this node does not report it; restart the node to see it
status-node-link-unreachable = not connected to { $hub }: this machine cannot reach it ({ $reason })
status-node-link-unreachable-macos = not connected to { $hub }: macOS is blocking the way to it ({ $reason }). Allow `agora` in System Settings > Privacy & Security > Local Network, then restart the node once more.
status-node-link-not-permitted = not connected to { $hub }: this machine did not allow the connection ({ $reason })
status-node-link-not-permitted-macos = not connected to { $hub }: macOS did not allow the connection ({ $reason }). Allow `agora` in System Settings > Privacy & Security > Local Network, then restart the node once more.
status-node-link-refused = not connected to { $hub }: nothing is listening at that address ({ $reason })
status-node-link-certificate = not connected to { $hub }: the hub's certificate is not trusted by this machine ({ $reason })
status-node-link-hub-refused = not connected to { $hub }: the hub refused this machine (code { $reason })
status-node-link-closed = not connected to { $hub }: the hub ended the connection; the node is trying again
status-node-link-other = not connected to { $hub }: { $reason }
status-agents = agents running: { $count }
status-holder-old = chat agents are resumed, not kept, after a node restart: this machine's agent keeper is older ({ $count ->
    [one] it keeps 1 agent
   *[other] it keeps { $count } agents
}; a new one starts when they have ended)
status-agents-kept =agents still running without the node: { $count } (`agora` brings them back, `agora down --agents` stops them)
status-log = log: { $log }
down-stopped = Stopped the { $role } (process { $pid }).
down-stopped-supervised = Stopped the { $role } and its supervisor (process { $pid }).
down-failed = Could not stop the { $role }: { $reason }
down-still-running = it was still running 10 seconds after being asked to stop
down-lock-busy = The process ended, but another process has held { $path } for more than 10 seconds (it may be stopped or hung), so its old lock file stays: harmless, the next start replaces it. Find that process, or end it.
down-nothing = Nothing to stop: `agora` did not start anything that is still running.
down-agents-kept = { $count ->
    [one] 1 agent keeps running. `agora` brings it back; `agora down --agents` stops it.
   *[other] { $count } agents keep running. `agora` brings them back; `agora down --agents` stops them.
  }
down-agents-stopped = { $count ->
    [one] Stopped 1 agent.
   *[other] Stopped { $count } agents.
  }
down-holder = agents' keeper
service-description = Starts the Agora { $role } when you log in (it keeps it running).
service-unsupported = Starting Agora when you log in is not available on this system yet. Start it with `agora` after logging in.
service-registering = Registering "{ $name }" to start the { $role } when you log in. It runs: { $command }
service-installed = The { $role } now starts when you log in ("{ $name }"), and it was started now.
service-already = The { $role } already starts when you log in ("{ $name }"); nothing changed. To register it anew, run `agora service remove` first.
service-updated = The { $role }'s service ("{ $name }") was set up with older settings; its file is updated. The running service keeps the old settings until you restart it: `systemctl --user restart { $name }`.
service-installed-running = The { $role } now starts when you log in ("{ $name }"). It runs already (process { $pid }), so it was not started again.
service-installed-not-started = The { $role } now starts when you log in ("{ $name }"), but it could not be started now: { $reason }. `agora` starts it.
service-bad-path = "{ $path }" cannot go into the login entry: it contains `%` (the system would read it as a variable) or characters it cannot carry. Move Agora or its folder to a plain path.
supervise-log-link = The log { $log } (or its folder) is a link, a second name of another file, or not a plain file of yours, so the supervisor does not write to it. Remove it, and `agora` makes a new one.
service-install-failed = Could not register "{ $name }": { $reason }
service-removed = "{ $name }" is removed: the { $role } no longer starts when you log in.
service-none = The { $role } was not set to start when you log in.
service-remove-failed = Could not remove "{ $name }": { $reason }
service-status-installed = The { $role } starts when you log in ("{ $name }").
service-status-disabled = The { $role } is set to start when you log in ("{ $name }"), but that is turned off in this system's settings.
service-status-not-installed = The { $role } does not start by itself when you log in (`agora service install` sets that up).
service-status-failed = Could not read "{ $name }": { $reason }
service-status-running = running now (process { $pid })
service-status-stopped = not running now
service-unavailable = Starting Agora when you log in is not available on this machine: on Linux it needs systemd's per-user services (`systemctl`), which were not found; on macOS it needs your login session (log in at the screen; SSH alone is not enough). Start it with `agora` after logging in.
service-no-user-manager = Could not reach your user's service manager for "{ $name }": this session has no systemd user bus (for example `su`, or an SSH session while your user manager is not running), or on macOS no login session (an SSH session while you are not logged in at the screen). Log in on this machine and try again; nothing was changed.
service-in-the-way = { $path } is a link or not a plain file, so Agora does not replace it. If it is not yours to keep, remove it and try again.
service-bad-path-control = "{ $path }" cannot go into the login entry: it contains a line break or another control character. Move Agora or its folder to a plain path.
service-bad-path-relative = "{ $path }" cannot go into the login entry: it is not a full path. Run Agora by its full path, or name its folder with a full path.
service-not-ours = "{ $name }" is already set up on this machine, but not by Agora ({ $path }), so Agora leaves it exactly as it is: it neither replaces, stops nor removes it. To let Agora set it up, remove or rename that one yourself first.
service-root = `agora service` sets up a login service for your own user, so it does not run as root (with `sudo`). Run it as yourself, without `sudo`; nothing was changed.
service-no-config-folder = There is no folder for your login services: neither XDG_CONFIG_HOME nor HOME names a full path. Set HOME and try again; nothing was changed.
service-not-loaded = After reloading, systemd did not find "{ $name }" at { $path }; it reads its user units from another folder (its own XDG_CONFIG_HOME). Nothing was registered.
service-overridden = "{ $name }" would not run only what Agora wrote: systemd also reads { $paths }, which can change what it runs. Nothing was registered. Remove or move those files yourself if you do not need them, and try again.
service-drop-ins-left = These files that change "{ $name }" were not written by Agora and are left as they are: { $paths }
service-bad-path-backslash = "{ $path }" cannot go into the login entry: a folder with a backslash (\) cannot be written safely into a systemd unit. Move Agora's folder to a path without one.
service-bad-program = "{ $path }" cannot be started by systemd: the program's path contains a quote or a backslash. Move Agora to a path without one.
service-turned-off-in-settings = "{ $name }" is turned off in System Settings > General > Login Items & Extensions ("Allow in the Background"), and only you can turn it on there. Turn it on, then run `agora service install` again; nothing was changed.
service-was-off = The { $role } was set up to start when you log in ("{ $name }") but that was turned off; turning it on again.
service-already-overridden = The { $role } is set up to start when you log in ("{ $name }"), but systemd also reads { $paths }, which can change what it runs. Nothing was changed; remove or move those files yourself if you do not need them.
service-status-overridden = systemd also reads { $paths } for it, which can change what it runs
service-linger-hint = This starts Agora when you log in. To start it at boot without logging in, run `loginctl enable-linger` yourself; Agora did not do that.
supervise-not-bound = The { $role } is not started: this system did not let it be tied to its supervisor, so `agora down` could not stop it. Start it by hand (`agora { $role } run`) or report this.
supervise-stop-request = Supervisor: asked to stop; the { $role } was stopped with it.
supervise-stopped = Supervisor: the { $role } stopped (exit code 0); it is not started again.
supervise-already-running = Supervisor: the { $role } already runs here (it was not started by this supervisor), so another one is not started.
supervise-lost-lock = Supervisor: the { $role }'s lock no longer names this supervisor (`agora down` ran, or another took over), so the { $role } is not started again.
supervise-lock-held = Another process (process { $pid }) already looks after the { $role } on this Agora folder; this supervisor ends.
status-restarting = The { $role } is not running: it crashed ({ $end }) and its supervisor (process { $pid }) starts it again shortly (restart { $count } of { $max }).
status-restarts = { $count ->
    [one] restarted once in the last { $window } minutes
   *[other] restarted { $count } times in the last { $window } minutes
  }; the last crash: { $end }, { $minutes ->
    [0] less than a minute ago
   *[other] { $minutes } min ago
  }
status-gave-up = The { $role } crashed again after { $max } restarts and was not started again ({ $end }, { $minutes ->
    [0] less than a minute ago
   *[other] { $minutes } min ago
  }). Its log says why; `agora` starts it again.
status-refused = The { $role } ended right after starting ({ $end }, { $minutes ->
    [0] less than a minute ago
   *[other] { $minutes } min ago
  }), so it was not started again. Its log says why; `agora` tries again.
supervise-end-code = exit code { $code }
supervise-end-signal = signal { $signal }
supervise-refused = Supervisor: the { $role } ended { $seconds } s after starting ({ $end }), too soon to be a crash, so it is not started again. The reason is above.
supervise-restarting = Supervisor: the { $role } ended after { $seconds } s ({ $end }); starting it again in { $wait } s (restart { $count } of { $max } in { $minutes } minutes).
supervise-gave-up = Supervisor: the { $role } ended again ({ $end }) after { $max } restarts in { $minutes } minutes; giving up. `agora` starts it again.
supervise-handoff-lost = The { $role } was stopped while its supervisor was starting (`agora down` ran), so it is not started.
cli-node-policy-not-understood = node.policy_not_understood: the policy file has a line this node does not understand ({ $line }), so no agent may start without asking until it is fixed.
cli-version = Agora { $version }
cli-machines-none = The hub shows no machine.
cli-machines-none-old = No machine is old: every machine was seen in the last 24 hours.
cli-machine-line = { $id } · { $state ->
    [online] online
    [old] old, last seen { $seen }
   *[other] offline, last seen { $seen }
}
cli-machine-cert-soon = { $id } · its certificate ends on { $date }{ $days ->
    [0] , today
    [1] , in one day
   *[other] , in { $days } days
}. It renews itself while it is connected; if it does not, run "agora doctor" on that machine, or join it again: "agora hub code" here, then "agora node enroll" on that machine.
cli-machine-cert-expired = { $id } · its certificate ended on { $date }, so it cannot connect. Join it again: "agora hub code" here, then "agora node enroll" on that machine.
cli-machine-unknown = The hub shows no machine named { $id }.
cli-machines-question = Forget these machines? They leave every window; one that connects again comes back. [y/N]
cli-machines-question-refuse = Forget these machines and refuse them if they connect again? Each would need a new join code. [y/N]
cli-machines-declined = Nothing was forgotten.
cli-machines-not-asked = Nothing was forgotten: this terminal cannot answer. Run it in a terminal, or add --yes.
cli-machines-hub-unreachable = The running hub could not be reached from its folder ({ $reason }). Machines are forgotten by the running hub; start it with "agora" or "agora hub run".
cli-machine-forgotten = { $id } is forgotten.
cli-machine-not-forgotten = { $id } was not forgotten ({ $code }).

cli-metrics-token-created = Metrics token { $name }: { $token }  It is shown only now. It can read `/api/metrics` and nothing else (start the hub with --metrics); `agora hub metrics-token revoke { $name }` ends it.
cli-metrics-token-bad-name = A token name has 1 to 64 letters, digits, dots, dashes or underscores.
cli-metrics-token-exists = A metrics token called { $name } already exists. Revoke it first, or pick another name.
cli-metrics-token-none = No metrics tokens yet. Make one with `agora hub metrics-token create NAME`.
cli-metrics-token-line = { $name } · made { $created } · last used { $used } · expires { $expires }{ $state }
cli-metrics-token-never-used = never
cli-metrics-token-no-expiry = never
cli-metrics-token-state-revoked = {" "}· revoked
cli-metrics-token-state-expired = {" "}· expired
cli-metrics-token-revoked = Metrics token { $name } is revoked. It fails at its next use.
cli-metrics-token-unknown = There is no live metrics token called { $name }.
doctor-credential = { $detail } receives { $names } from this node when it starts (a stored secret, else the environment of the command that ran this check; the node's own environment may differ). Names only; this check does not know whether that login works.
doctor-credential-none = No credential found for { $detail }: the node has nothing to give it. If it says it is not logged in (a confined agent cannot see a login kept in a Keychain), make a token for it and run: agora node secret set { $name }, then add the line "credentials.{ $detail } = { $name }" to the node's policy.
doctor-credential-unlisted = A value for { $names } exists on this node, but no "credentials.{ $detail }" line in the policy lets { $detail } receive it, so it is not passed. Other agents never get it either.
doctor-credential-refused = The policy lists { $names } for { $detail }, which its description does not declare: they are not passed.
doctor-credential-malformed = The "credentials.{ $detail }" line in the policy is malformed (plain names, separated by commas, no values): nothing is passed to { $detail }.
cli-secret-prompt = Value of { $name } (not shown):
cli-secret-set = Stored { $name } on this machine. The value stays here: it is never sent to the hub. To let an agent kind receive it, add a line "credentials.KIND = { $name }" to the node's policy and restart the node.
cli-secret-removed = Removed { $name }.
cli-secret-not-found = No secret named { $name } is stored here.
cli-secret-none = No secret is stored on this machine.
cli-secret-name = { $name }
cli-secret-bad-name = That is not a usable name: use letters, digits and underscores only, not starting with a digit, and not a name the system or Agora uses.
cli-secret-bad-value = The value must be one line of at most 8192 bytes, with no control characters. Nothing was stored.
cli-secret-failed = The secret could not be stored or read here. Check that the node's data folder is yours and can be written.
cli-history-purged = Removed the chat history this machine kept for its agents.
cli-history-purge-failed = The chat history could not be removed. Check that the node's data folder is yours and can be written.
cli-secret-name-collides = A secret whose name differs only in capital letters is already stored. Use that exact name, or remove it first.

# ADR-0065: `agora pair` on the hub and `agora join` on the new machine.
pair-no-hub = No hub is running from { $dir }. Start it with `agora` (or `agora hub run`), then run this again on the machine that runs the hub.
pair-hub-unreachable = The hub did not answer ({ $reason }). Is it running? Start it with `agora` or `agora hub run`.
pair-minutes-usage = --minutes takes a number from 1 to { $max }.
pair-opened = Your hub is open for new machines for { $minutes } minutes.
pair-open-next = On the new machine, run `agora join { $address }`. The new machine shows a number, and this screen shows one too: approve only if they are the same.
pair-open-also = If that does not reach this computer from the new machine, the hub also answers at: { $addresses }.
pair-open-next-name = On the new machine, run `agora join { $address }`, if that name leads to this computer from there: the hub's certificate names it, but your network decides where it leads. The new machine shows a number, and this screen shows one too: approve only if they are the same.
pair-open-names = These names are in the hub's certificate and may also reach it, if your network resolves them to this computer: { $names }.
pair-open-next-elsewhere = On the new machine, run `agora join { $address }`, if that address leads to this computer from there: the hub's certificate names it, but no interface of this computer has it now, so it may be an old address, or one that leads here through your network. The new machine shows a number, and this screen shows one too: approve only if they are the same.
pair-open-elsewhere = The hub's certificate also names these addresses, which are not on this computer's interfaces now: { $addresses }. One may be an old address, or lead here through your network.
pair-open-local-only = This hub listens on this computer only ({ $listen }), so another machine cannot reach it. To let your other machines join, make it listen on this computer's address on your network: run `agora --listen { $lan }`, then `agora down` and `agora` (a hub you started with `agora hub run` takes the same `--listen`), then `agora pair open` again. `0.0.0.0:{ $port }` instead listens on every IPv4 interface of this computer, a public one too if it has one: anyone who can reach it can try to connect (a new machine still needs your approval and the number).
pair-open-local-only-several = This hub listens on this computer only ({ $listen }), so another machine cannot reach it. To let your other machines join, make it listen on this computer's address on the network they are on. This computer has several addresses: { $lans }. Run `agora --listen ADDRESS:{ $port }` with the one on that network, then `agora down` and `agora` (a hub you started with `agora hub run` takes the same `--listen`), then `agora pair open` again. `0.0.0.0:{ $port }` instead listens on every IPv4 interface of this computer, a public one too if it has one: anyone who can reach it can try to connect (a new machine still needs your approval and the number).
pair-open-local-only-no-lan = This hub listens on this computer only ({ $listen }), so another machine cannot reach it. To let your other machines join, make it listen on this computer's address on your network (ADDRESS): run `agora --listen ADDRESS:{ $port }`, then `agora down` and `agora` (a hub you started with `agora hub run` takes the same `--listen`), then `agora pair open` again. `0.0.0.0:{ $port }` instead listens on every IPv4 interface of this computer, a public one too if it has one: anyone who can reach it can try to connect (a new machine still needs your approval and the number).
pair-open-local-join = Another Agora on this same computer can join now: run `agora join { $address }` there. It shows a number, and this screen shows one too: approve only if they are the same.
pair-open-no-address = The hub's certificate names no address another machine can dial yet. Start the hub with `--name ADDRESS` (the address the new machine will dial), then run `agora pair open` again.
pair-open-waiting = Waiting for a machine. Leave with Ctrl+C: the hub stays open until its time is up, or until you run `agora pair close`.
pair-status-open = Open for new machines. It closes in { $minutes } minutes.
pair-status-closed = Closed to new machines. Run `agora pair open` to let one join.
pair-status-waiting = { $count ->
    [one] One machine is waiting for your answer.
   *[other] { $count } machines are waiting for your answer.
}
pair-closed-now = Your hub is closed to new machines.
pair-closed-after = Your hub is closed to new machines again. Run `agora pair open` to add another.
pair-reopened-elsewhere = Your hub was opened for new machines again somewhere else (it closes in { $minutes } minutes), so this screen leaves it open. Run `agora pair close` to close it.
pair-status-unknown = Could not tell whether your hub is still open to new machines ({ $reason }), so nothing was closed from here. Run `agora pair status` to see, and `agora pair close` to close it.
pair-time-up = The time you gave is up. Your hub is closed to new machines.
pair-needs-terminal = Approving a machine needs you at a terminal: `agora pair open` reads your answer from the keyboard only, never from a pipe or a script. Run it in a terminal window. Nothing was opened.
pair-too-fast = That key came too soon after the question to be an answer. Look at both numbers, then answer.
pair-input-ended = No answer can be typed here, so the hub stays open for its time; a machine that asks waits for you in a window, or run `agora pair close`.
pair-card = New machine "{ $name }" ({ $os }, { $address }) wants to join. Number { $number }.
pair-card-ask = Is it the same number on that machine? Approve it? [y/N]
pair-card-ended = "{ $name }" is no longer asking (it stopped, ran out of time, or your hub was closed to new machines), so there is nothing to answer.
pair-card-approved-elsewhere = Answered in a window: "{ $name }" was approved and is joining.
pair-card-denied-elsewhere = Answered in a window: "{ $name }" was denied. Nothing was given to that machine.
pair-card-next = ---- The last request ended without your answer here. A different machine is asking now: read its name and number before you answer. ----
pair-next-too-fast = That key came before you could read the new request, so it was ignored with everything typed before it. Wait two seconds, look at the name and number above, then answer.
pair-approved = Approved. "{ $name }" is joining.
pair-denied-now = Denied. Nothing was given to that machine.
pair-replace-ask = A machine named "{ $name }" has already joined. Replace it? The old one loses its access. [y/N]
pair-left-waiting = Not replaced. The request stays open on the hub until its time is up.

join-help = To join an Agora that runs on another machine, run `agora pair open` on that machine, then run `agora join` here: it finds the hub by itself. If it cannot, give the hub's address: `agora join 192.168.1.20:7443`.
join-looking = Looking for a hub that is open for new machines...
join-found-ask = Found the hub "{ $name }" at { $address }. Its fingerprint is { $fingerprint }. Compare it with what `agora pair open` shows on the hub: if it is not exactly the same, answer no. Is it the same?  [y/N]
pair-fingerprint = This hub's fingerprint is { $fingerprint }. A machine that finds this hub by itself shows this and asks the person to compare it.
join-none-found = No hub that is open for new machines answered. On the machine that runs the hub, run `agora pair open` (a hub can only be found while it is open), then run `agora join` here again. Or give its address: `agora join 192.168.1.20:7443`. Some networks do not carry this kind of search; the address always works.
join-found-list-head = More than one hub is open for new machines. Compare these fingerprints with what `agora pair open` shows on the hub you mean:
join-found-item = { $n }) "{ $name }" at { $address }, fingerprint { $fingerprint }
join-which = Which one has the same fingerprint? Type its number (1 to { $count }), or press Enter to stop:
join-found-note = Looking was only a way to find an address. This machine will join only a hub whose certificate authority has the fingerprint you confirmed, and the number below must be the same as the one on the hub's screen before you approve.
join-nothing-chosen = Nothing was chosen. Nothing was saved.
join-connecting = Connecting to { $address }...
join-number = Number { $number }. Approve this machine on the hub: the hub shows the same number. Waiting for the owner...
join-done = Joined. This machine is "{ $id }". The hub's certificate authority fingerprint (SHA-256) is { $fingerprint }; the owner can check it with `agora hub fingerprint`.
join-start = Start this machine with `agora`; it connects to the hub by itself.
join-already = This machine has already joined a hub (see { $dir }). To join another one, run `agora join ADDRESS --replace`.
join-kept-newer = While this machine waited for approval it was enrolled with the same hub under the same name again ("{ $id }"). That newer certificate is kept: the one this join got is no longer valid. Nothing was changed.
join-kept-unsure = While this machine waited for approval it was enrolled with the same hub under the same name again ("{ $id }"), in the same second as this join. Which certificate the hub kept cannot be told, so the link that was there is kept and nothing was changed. If this machine does not connect, run the join again with --replace.
join-bad-address = That is not an address this machine can use. Give the hub's address like 192.168.1.20:7443 or hub.example.
join-no-key = This machine could not make its key.
join-unreachable = Could not reach the hub at { $address } ({ $reason }). Is it running, and is the address right? Nothing was saved.
join-tls13-required = The hub at { $address } only offers an old kind of secure connection, so this machine will not join it. Nothing was saved.
join-tls-failed = The secure connection to { $address } could not be set up ({ $reason }). Nothing was saved.
join-timeout-answer = The hub did not answer in time. Nothing was saved.
join-timeout-decision = Nobody approved this machine in time. Nothing was saved. Ask the owner to run `agora pair open`, then try again.
join-denied = The owner said no. Nothing was saved.
join-closed = The hub ended the conversation before it answered. Nothing was saved.
join-unexpected = The hub answered in a way this machine does not accept. Nothing was saved.
join-name-hint = The hub's certificate has to name the address you typed. Ask the owner to start the hub with `--name { $name }`.
join-certificate-invalid = What the hub sent did not check out ({ $what }). Nothing was saved.
join-store-failed = The result could not be saved: { $reason }
join-settings-unsaved = This machine's settings could not be saved. Run `agora` to set it up.
frontdoor-join-address = The hub's address (like 192.168.1.20:7443; Enter if you do not have it):

# ADR-0067 rung 3: reaching the hub from outside this machine (`agora remote`).
remote-password_required = No way in from outside is turned on without a screen password. Set one first with `agora password set`, then run this again.
remote-password_in_use = The screen password protects a way in from outside this machine, so it stays. Turn that off first with `agora remote off` (and start the hub on a local address if it listens on a public one), then clear the password.
remote-tailscale_missing = Tailscale is not installed on this machine (there is no `tailscale` command). Install it from tailscale.com, sign in with `tailscale up`, then run this again.
remote-tailscale_logged_out = Tailscale is installed but this machine is not signed in to your tailnet. Run `tailscale up`, then run this again.
remote-tailscale_timeout = Tailscale did not answer in time. If it printed a link asking you to allow HTTPS for your tailnet, open it once, then run this again.
remote-tailscale_unreadable = Tailscale's answer could not be used ({ $reason }). Nothing was changed except what is listed above. Check that `tailscale status` works and that MagicDNS is turned on for your tailnet.
remote-serve_port_in_use = Tailscale already serves something of yours on HTTPS port { $port }, so Agora leaves it alone. Choose another port with `--port N`.
remote-names-added = The hub's certificate now names this machine on your tailnet: { $names }. The running hub picks them up within a few seconds; a stopped hub when it starts.
remote-serve-on = Open Agora from a phone or laptop on your tailnet at { $url } (sign in with a code and the screen password).
remote-tailscale-loopback-hint = The hub listens only on this machine, so other tailnet machines cannot reach it yet. Run this again with `--serve`, or start the hub with `--listen 0.0.0.0:7443`.
remote-password-reminder = The screen password is on for every sign-in from another machine. `agora remote off` turns this off again.
remote-status-listen = The hub listens on { $addr }.
remote-status-not-running = No running hub was found here ({ $dir } has no address).
remote-status-this-machine = This machine: reachable.
remote-status-lan = Local network: { $on ->
        [yes] reachable
       *[no] not reachable
    }.
remote-status-tailnet = Tailscale: { $on ->
        [yes] reachable
       *[no] not reachable
    }.
remote-status-names = Names in the hub's certificate for it: { $names }.
remote-status-serve = Tailscale serve: { $url }
remote-status-public = Public internet: { $on ->
        [yes] reachable
       *[no] not reachable
    }.
remote-status-password-set = Screen password: set.
remote-status-password-not-set = Screen password: not set.
remote-status-no-password = A way in from outside this machine is on and no screen password is set. Set one now with `agora password set`, or turn the way in off with `agora remote off`.
remote-public-warning = { $cause ->
        [funnel] Tailscale Funnel is on, so something of this machine is reachable from the whole internet. Only the screen password and the sign-in code protect it. Turn it off with `tailscale funnel reset` unless you meant it.
       *[listen] The hub listens on a public address. Only the screen password and the certificate you check on a new machine protect it; a plain address gives no other protection. Tailscale is the safer way: `agora remote tailscale --serve`.
    }
remote-off-names = The tailnet names are out of the hub's list; the certificate drops them the next time the hub starts.
remote-off-serve = The Tailscale serve rule on HTTPS port { $port } is removed. Your other serve rules were not touched.
remote-off-serve-gone = The Tailscale serve rule Agora added is already gone.
remote-off-serve-foreign = The rule on HTTPS port { $port } is no longer the one Agora added, so it was left exactly as it is.
remote-off-nothing = Agora has not turned on any way in from outside this machine.
frontdoor-listen-saved = The hub will listen on { $addr } from its next start (`agora down`, then `agora`).
doctor-remote-password-set = A way in from outside this machine is on, and a screen password protects it.
doctor-remote-password-missing = A way in from outside this machine is on and no screen password is set. Run `agora password set` now, or `agora remote off`.
doctor-remote-password-missing-tailnet = The hub can be reached over Tailscale and no screen password is set. Run `agora password set`.
doctor-remote-public-listen = The hub listens on a public address: only the screen password and the certificate pin protect it. Prefer Tailscale (`agora remote tailscale --serve`) and listen on a local address.
doctor-remote-tailnet = Tailscale works; this machine is { $name } on your tailnet.
doctor-remote-tailnet-unreachable = Tailscale is not reachable from here ({ $reason }), so the tailnet address will not work. Check `tailscale status`.
doctor-remote-names-missing = The hub's certificate does not name { $names } yet. Restart the hub, or wait a few seconds if `agora remote tailscale` was just run.
remote-password_in_use_listen = The hub listens on a public address, and the screen password protects it, so it stays. Stop the hub (`agora down`), start it on a local address (`agora --listen 127.0.0.1:7443`), then clear the password.
remote-hub_not_running = Agora could not find the address of a running hub in { $dir }. Start the hub with `agora`, then run this again.
remote-status-public-funnel = Public internet: reachable through Tailscale Funnel.
doctor-remote-funnel = Tailscale Funnel is on for this machine, so something is reachable from the whole internet. Only the screen password and the sign-in code protect it. Turn it off with `tailscale funnel reset` unless you meant it.

# ADR-0067 rung 4: a trusted reverse proxy or tunnel in front of the hub (`agora remote proxy`).
remote-proxy_header_invalid = "{ $header }" cannot be the header that carries the client's address: use a plain header name (letters, digits and hyphens) that the proxy always sets itself, such as the one your tunnel provider documents. Nothing was changed.
remote-proxy_host_invalid = "{ $host }" is not a name the hub can answer to: use a plain public host name such as `agora.example.com` (no address, no wildcard, no port; at most 4). Nothing was changed.
remote-proxy_addr_invalid = "{ $addr }" is not an address a proxy can connect from (at most 4). A proxy on this machine needs none. Nothing was changed.
remote-proxy-on = The hub now trusts a reverse proxy on this machine: it answers only { $hosts } (and this machine's own names) and takes a visitor's real address only from the "{ $header }" header. The running hub picks this up within a few seconds. If your proxy forwards to 127.0.0.1 with that address as Host and sends no address header, visitors look like the owner at this machine: set the public Host name and the header, and check from outside (mobile data) that the password is asked.
remote-proxy-access-hint = The hub is now reachable from the internet through that proxy. Strongly recommended: put an access layer with single sign-on in front of the hostname (for a Cloudflare Tunnel, Cloudflare Access), so that only people you name reach the sign-in page. The screen password and the sign-in code are the second lock, not the first.
remote-proxy-listen-hint = The hub also listens beyond this machine, so it can be reached without the proxy. Start it on a local address (`agora --listen 127.0.0.1:7443`) so that the proxy is the only way in.
remote-status-proxy = Trusted proxy: on, answers { $hosts }, real address from the "{ $header }" header. If your proxy forwards to 127.0.0.1 with that address as Host and sends no address header, visitors look like the owner at this machine: set the public Host name and the header, and check from outside (mobile data) that the password is asked.
remote-proxy-warning = Reachable from the internet through the trusted proxy. Only the proxy's own access rules, the screen password and the sign-in code protect it. Turn it off with `agora remote off`. If your proxy forwards to 127.0.0.1 with that address as Host and sends no address header, visitors look like the owner at this machine: set the public Host name and the header, and check from outside (mobile data) that the password is asked.
remote-status-proxy-damaged = The file that turns on the trusted proxy is damaged. A hub that is running keeps the last good settings (its Host and Origin checks too); one that starts now trusts no proxy and checks no Host or Origin. Run `agora remote off`, then `agora remote proxy` again.
remote-off-proxy = The hub no longer trusts a reverse proxy (the running hub notices within a few seconds).
doctor-remote-proxy-on = The hub is reachable from the internet through a trusted proxy ({ $hosts }, address from "{ $header }"). Put an access layer with single sign-on in front of that hostname, and turn it off with `agora remote off` unless you meant it. If your proxy forwards to 127.0.0.1 with that address as Host and sends no address header, visitors look like the owner at this machine: set the public Host name and the header, and check from outside (mobile data) that the password is asked.
doctor-remote-proxy-listen = A trusted proxy is on, but the hub listens on { $addr }, beyond this machine. Start it on a local address so that the proxy is the only way in.
doctor-remote-proxy-damaged = The file that turns on the trusted proxy is damaged. A hub that is running keeps the last good settings (its Host and Origin checks too); one that starts now trusts no proxy and checks no Host or Origin. Run `agora remote off`, then `agora remote proxy` again.
cli-hub-proxy-on = Trusted proxy: on. The hub answers { $hosts } and is reachable from the internet through it.
hold-failed = The terminal holder could not start: { $message }
cli-agent-install-about =
    About to install { $display } ({ $name }) { $version } for { $platform }, on this machine only:
      Download:   { $url }
      SHA-256:    { $sha256 }
      Signature:  { $signature }
      Publisher:  { $publisher } (key { $key })
      Folder:     { $dir }
      Then runs:  the new program with "{ $verify }" to check its version.
    The program runs as you, with your files. Nothing outside the Agora folder is changed and nothing runs as administrator.
cli-agent-install-signature-none = none (the checksum only)
cli-agent-install-signature-by = detached signature { $url } checked with your key "{ $key }"
cli-agent-install-ask-name = Type the agent name ({ $name }) to install it, or anything else to stop:
cli-agent-install-ask-go-back = { $installed } is installed and this is an older version. Type the version ({ $version }) to go back to it:
cli-agent-install-done = Installed { $name } { $version }. The program is at { $launcher }. Agora does not edit your shell profile; agents it starts find the program there once a later release looks in that folder. Running agents are not touched.
cli-agent-install-current = { $name } { $version } is already installed.
cli-agent-install-no-list = Give the signed list and its root key: --list FILE and --root-key HEX or FILE (or save the key as agent-list-root-key in the Agora folder). Nothing was changed.
cli-agent-install-err-index-invalid = The agent list is not acceptable ({ $code }: { $detail }). Nothing was changed.
cli-agent-install-err-list-state = The record of the newest agent list this machine accepted, { $path }, cannot be read ({ $code }: { $detail }), so no list is accepted: an older list could otherwise be put back. If you did not expect this, find out what changed that file first. To start again, remove { $path } and run the install again with the newest list from its publisher: that list is then accepted as the first one and its version recorded. Nothing was changed.
cli-agent-install-err-not-approved = Not installed: the agent name was not typed here ({ $code }: { $detail }). Nothing was changed.
cli-agent-install-err-download-failed = The download did not happen ({ $code }: { $detail }). Nothing was changed.
cli-agent-install-err-checksum-mismatch = The download does not match the checksum in the list ({ $code }). Nothing was changed.
cli-agent-install-err-signature-invalid = The download's signature could not be accepted ({ $code }: { $detail }). Save the vendor's key as agent-keys/NAME.pub in the Agora folder. Nothing was changed.
cli-agent-install-err-disk-full = The disk is full ({ $code }). Nothing was changed.
cli-agent-install-err-verify-failed = The new program did not report the expected version ({ $code }: { $detail }). The new version was removed; the version installed before, if any, is unchanged.
cli-agent-install-err-timeout = It took too long ({ $code }: { $detail }). Nothing was changed.
cli-agent-install-err-unsupported-platform = The list has no download for this system ({ $code }: { $detail }). Nothing was changed.
cli-agent-install-err-in-progress = Another install is running on this machine ({ $code }). Nothing was changed.
cli-agent-install-err-revoked = The list's publisher { $detail } has been revoked ({ $code }). Nothing was changed.
cli-agent-install-err-downgrade = Version { $detail } is installed and the list names an older one ({ $code }). Add --go-back to go to the older version. Nothing was changed.
cli-agent-install-err-unsupported-format = This entry cannot be installed here yet ({ $code }: { $detail }): archives are not supported. Nothing was changed.
cli-agent-install-err-not-offered = The list does not offer that agent as a download ({ $code }: { $detail }). Nothing was changed.
cli-agent-install-err-tool-missing = A program this needs is not installed: { $detail } ({ $code }). Nothing was changed.
cli-agent-install-err-io-failed = A file could not be written ({ $code }: { $detail }). Nothing was changed.
cli-agent-install-err-conflict = That name is not available ({ $code }: { $detail }): another agent already owns that program name, or the agent is named bin. Nothing was changed.

## agora agent explain: which screen rule decides a captured screen. Rule ids, regions and states are the manifest's own words.
cli-explain-header = Agent kind { $kind } ({ $source ->
        [built-in] built in
        [registry] from the ACP registry
       *[folder] from the manifests folder
    }), screen replayed at { $cols }x{ $rows }.
cli-explain-region = What the rules read in { $region }:
cli-explain-region-empty = (nothing)
cli-explain-region-cut = ({ $count } earlier lines not shown)
cli-explain-rule-matched = MATCH   { $id } → { $state } (priority { $priority }, { $region })
cli-explain-rule-own-missing = no      { $id } → { $state } (priority { $priority }, { $region }): its own text is not there
cli-explain-rule-all-missing = no      { $id } → { $state } (priority { $priority }, { $region }): all[{ $index }] is not there
cli-explain-rule-any-missing = no      { $id } → { $state } (priority { $priority }, { $region }): none of its "any" conditions is there
cli-explain-rule-not-present = no      { $id } → { $state } (priority { $priority }, { $region }): not[{ $index }] is there
cli-explain-decided = Decision: { $state }, by rule { $id } (the first match, highest priority first).
cli-explain-undecided = Decision: none. No rule matched, so the node keeps the agent's previous state.
cli-explain-note = These are the screen rules only. A state the agent reports itself (through its hooks, ACP or an adapter) wins over them.
cli-explain-no-rules = { $kind } has no screen rules, so no state is read from its screen and there is nothing to explain here. A state the agent reports, or the node's own (such as done when it exits), is outside this report.
cli-explain-unknown-kind = There is no agent kind "{ $kind }" here. "agora agent kinds" lists the kinds a running node offers.
cli-explain-bad-file = Could not read the screen file { $path }: { $reason }
cli-explain-file-too-large = The screen file { $path } is larger than { $max } bytes, so it is not a captured screen.
cli-explain-manifest-problem = Skipped the manifest file { $path }, as the node does: { $reason }
cli-explain-bad-size = --size takes the terminal size as COLSxROWS, for example 120x40 (each from 2 to 1000).
