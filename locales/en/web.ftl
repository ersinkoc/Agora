# Web client text. The UI uses plain words, never internal terms (docs/ux/visual-language.md).

app-name = Agora
home-title = Home
gallery-title = Component gallery
attention-title = Needs you
attention-empty = All calm. Nothing needs you.
fleet-title = Your agents
fleet-empty = No agents yet. Start one to see it here.

state-working = Working
state-needs-you = Needs you
state-finished = Finished
state-idle = Idle
state-unknown = Unknown

agent-on-machine = on { $machine }
agent-workspace = Workspace { $name }
machines-online =
    { $count ->
        [one] { $count } machine online
       *[other] { $count } machines online
    }
link-connecting = Connecting to your hub…
link-offline = Hub unreachable. Retrying…
action-open = Open
action-start-agent = Start an agent
theme-dark = Dark
theme-light = Light

palette-title = Command palette
palette-placeholder = Jump to an agent or run an action…
palette-empty = Nothing matches.
palette-open-agent = Open { $name }
palette-next-attention = Next that needs me
palette-go-home = Go to Home
palette-hint = Press Ctrl+K or ⌘K to search, N for the next agent that needs you.

focus-back = Back to Home
focus-terminal = Terminal
focus-terminal-placeholder = The live terminal appears here once this window is connected to your machines.
focus-why = Why is this shown?
focus-why-agent = The agent reported this itself.
focus-why-screen = Read from what is on the agent's screen.
focus-not-found = That agent is not here any more.

approvals-title = Waiting for your answer
approval-title = { $name } asks permission
approval-allow = Allow
approval-deny = Deny
approval-not-trusted = This device is not trusted by that machine yet. Open "This device" and run the command it shows on the machine.
approval-failed = The answer did not go through. Try again.
device-title = This device
device-intro = Machines only accept an answer from a device their owner trusted. To trust this one, run this on the machine:
device-name = Device name
device-key = Device key
device-link = This device

signin-title = Sign in
signin-intro = Ask your hub for a code: run "agora hub login" on the machine that runs it, then type the code here.
signin-code = Code
signin-action = Sign in
signin-refused = That code did not work. It may be wrong, expired or already used.
signin-too-many = Too many wrong codes. Wait a minute and try again.
signin-failed = Could not reach your hub. Try again.
machines-title = Your machines
machines-empty = No machines connected yet.
machine-online = Online
machine-offline = Offline
machine-agents =
    { $count ->
        [one] { $count } agent running
       *[other] { $count } agents running
    }
palette-go-machines = Show my machines
agent-tokens = { $count } tokens used

start-title = Start an agent
start-where = Where
start-which = Which agent
start-loading = Asking the machine…
start-kinds-failed = The machine did not answer. Try again.
start-no-kinds = This machine has no agents set up yet.
start-no-projects = No project is set up on a connected machine. Run "agora node run --project PATH" on one.
start-not-installed = not installed
start-go = Start
start-cancel = Cancel
start-kind-unknown = That machine does not know this agent.
start-limit = That machine is at its limit. Stop an agent and try again.
start-failed = The agent did not start. Try again.
signin-passkey = Sign in with a passkey
signin-passkey-failed = The passkey did not work. Use a code instead.
device-passkey-intro = Add a passkey to sign in on this device without a code.
device-passkey-add = Add a passkey
device-passkey-added = Passkey added. Next time, choose "Sign in with a passkey".
device-passkey-failed = Could not add the passkey.
agent-cost = about { $amount } (estimated)
machine-cost = about { $amount } spent (estimated)
usage-total = About { $amount } spent in total (estimated)
usage-prices-date = prices from { $date }
usage-prices-bundled = prices from this hub's release
board-title = Shared tasks and files
board-open = open
board-done = done
messages-title = Recent messages
message-cross = between machines
usage-projects = Spent by project
console-agents = Agents
console-activity = Activity
console-wall-empty = Pick an agent on the left to watch its terminal here.
console-maximize = Full size
console-hide = Stop watching
console-no-messages = Agents have not written to each other yet.
console-spent = Spent so far (estimated)
changes-title = Uncommitted changes
changes-none = Nothing uncommitted. Every workspace is clean.
changes-files = { $count ->
    [one] 1 file changed
   *[other] { $count } files changed
}
diff-title = What { $name } changed
diff-close = Close
diff-failed = Could not read the changes from that machine.
console-files = Files
files-title = Files of { $name }
files-tree = Folders and files
files-pick = Pick a file on the left to read it.
files-binary = This file is not text, so it is not shown.
files-too-large = This file could not be read (it may be too large).
files-failed = Could not list this folder.
filter-placeholder = Filter by agent, machine or project
filter-none = Nothing matches.
console-drag = Drag this bar onto another terminal to swap their places
prompt-intro = This agent works through requests, not a terminal. Tell it what to do.
prompt-label = What should it do?
prompt-send = Send
prompt-sent = Sent.
prompt-failed = It did not go through. Try again.
console-history = History
history-title = Commits of { $name }
history-none = Nothing has been committed here yet.
history-failed = Could not read the history from that machine.
history-no-subject = (no message)
chat-empty = Nothing said yet. Tell it what to do below.
chat-log = Conversation
chat-tool = Tool
chat-plan = Plan
grant-title-typing = Allow typing into this agent for 1 hour?
grant-title-start = Allow starting agents in this project for 1 hour?
grant-title-other = Allow this window to act on { $machine } for 1 hour?
grant-explain = { $machine } asks for your permission before it lets this window do that. Your key on this device signs it, and it ends by itself.
grant-allow = Allow for 1 hour
grant-later = Not now
grant-failed = The permission did not go through. Try again.
audit-title = Activity
audit-intro = Who did what on this hub, newest first. Only the owner sees this.
audit-person = Person
audit-empty = Nothing has happened yet.
audit-owner-only = Only the owner can see the activity.
audit-signed-out = You are signed out. Sign in again to see the activity.
audit-failed = Could not load the activity.
audit-link = Activity
audit-outcome-done = Done
audit-outcome-sent = Sent, no answer yet
audit-action-sign-in = Signed in
audit-action-user-add = Added a team member
audit-action-user-role = Changed a role
audit-action-user-remove = Removed a team member
audit-action-user-grant = Gave access to a project
audit-action-user-revoke = Took back access to a project
audit-action-agentstart = Started an agent
audit-action-agentstop = Stopped an agent
audit-action-agentprompt = Sent a prompt
audit-action-agentsetmode = Changed an agent's mode
audit-action-terminalopen = Opened a terminal
audit-action-terminalinput = Typed in a terminal
audit-action-terminalresize = Resized a terminal
audit-action-terminalsubscribe = Watched a terminal
audit-action-chatsubscribe = Opened a conversation
audit-action-fileswrite = Changed a file
audit-action-filesread = Read a file
audit-action-fileslist = Listed files
audit-action-fileswatch = Watched files
audit-action-gitdiff = Looked at changes
audit-action-gitstatus = Looked at changes
audit-action-gitlog = Looked at history
audit-action-worktreecreate = Made a work area
audit-action-worktreeremove = Removed a work area
audit-action-approvalresolve = Answered an approval
audit-action-taskupdate = Changed a task
audit-action-nodepolicyget = Read a machine's rules
audit-action-grantcreate = Gave a machine permission
audit-action-grantlist = Looked at permissions
audit-action-grantrevoke = Took back a permission
audit-action-projectlist = Listed projects
audit-action-agentlist = Listed agents
audit-action-agentkinds = Listed agent kinds
audit-action-usagereport = Looked at spending

start-no-machines = No machine is online yet.
start-machine = Machine
start-project = Project
start-other-folder = Another folder…
start-directory = Working folder
start-directory-placeholder = Full path on the selected machine
start-portable = Create a shared project identity
start-portable-explain = Creates .agent-project.json in the project folder. Commit or copy it to the other machines to group this project. Existing metadata is preserved.
start-select-directory = Use this folder
start-in-directory = The agent will work in this folder. Existing files and the current branch are kept.
start-project-id = Project identity: { $id }
start-separate-worktree = Create an isolated working copy
start-identity-change = Associate this folder with its changed project identity
start-directory-unsupported = This machine needs an update to start agents in selected folders.
start-directory-failed = The folder could not be selected. Check its path and permissions.
workspace-shared = Shared folder · changes belong to the workspace
filter-project = Project
filter-all-projects = All projects
filter-project-machines = { $name } · { $count } machine(s)

start-directory-agent-unsupported = This agent needs an isolated working copy to load its configuration.

audit-action-directoryinspect = Selected a folder
audit-action-agentlaunch = Started an agent in a folder
audit-action-workspacequery = Read workspace files or changes
