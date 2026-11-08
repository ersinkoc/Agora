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
