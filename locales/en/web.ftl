# Web client text. The UI uses plain words, never internal terms (docs/ux/visual-language.md).

app-name = Agora
home-title = Home
gallery-title = Component gallery
attention-title = Needs you
attention-empty = All calm. Nothing needs you.
fleet-title = Your agents
fleet-empty = No agents yet. Start one to see it here.
fleet-watch-empty = No agents running yet.

state-working = Working
state-needs-you = Needs you
state-finished = Finished
state-idle = Idle
state-unknown = Unknown
state-failed = Ended with an error

agent-on-machine = on { $machine }
agent-workspace = Workspace { $name }
machines-online =
    { $count ->
        [one] { $count } machine online
       *[other] { $count } machines online
    }
link-connecting = Connecting to your Agora…
link-offline = Cannot reach your Agora. Retrying…
link-retry = Retry now
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
# Redesign PR 4: the title of a request is the ask itself, in the window's words, from a closed set of kinds (anything else is
# approval-title). The command, file or address stays on the line under it, whole.
approval-ask-run = { $name } wants to run a command
approval-ask-edit = { $name } wants to change a file
approval-ask-read = { $name } wants to read a file
approval-ask-fetch = { $name } wants to open a web address
approval-ask-delete = { $name } wants to delete something
approval-ask-start = { $name } wants to start another agent
approval-open-agent = Open { $name }
approval-not-trusted = This device is not trusted by that machine yet. Open "This device" and run the command it shows on the machine.
approval-failed = The answer did not go through. Try again.
approval-no-answer = The agent did not take your answer in time. The request is still open: wait a moment and answer again.
approval-agent-exited = The agent stopped, so your answer did not reach it. Start the agent again.
approval-forbidden = You cannot answer for this project from this device: it is limited to other projects there. Ask someone who works on it.
approval-role = Your role does not let you answer for this project. Ask the owner, or someone who works on it.
approval-no-longer-valid = This request is no longer valid. The agent it was about has ended or restarted.
approval-ran-out = This request ran out of time. Ask again from the agent's controls.
# ADR-0101: "Allow always", an answer the agent remembers. The session words are the ADR's, for a kind known to drop the answer when its
# session ends; no kind is marked so yet, so the window shows the "-kept" words (the agent keeps it, as long as it decides).
approval-allow-always = Allow always for this session
approval-allow-always-note = { $name } will not ask again for what it counts as the same request, until it ends. It decides what that covers.
approval-always-only = This agent offers no single answer for this request: allowing it allows it for the rest of its session.
approval-allow-always-kept = Allow always
approval-allow-always-note-kept = { $name } remembers this answer and will not ask again for what it counts as the same request. It decides what that covers and how long it keeps it, which may be longer than this session.
approval-always-only-kept = This agent offers no single answer for this request: allowing it lets the agent remember the answer, possibly beyond this session.
approval-allow-always-label = In its own words: “{ $label }”
approval-scope-forbidden = This machine does not let agents act without asking, so only this one request can be allowed.
approval-scope-unavailable = The agent does not offer that answer for this request. Nothing was sent; answer it again.
approval-adapter-outdated = This agent was started by an older Agora, which could make it remember a single Allow. This machine allows single answers only: start the agent again, then answer.
skip-approval-title = Let { $name } act without asking?
skip-approval-body = It will run commands and change files without asking you first, until it ends or you switch it back.
skip-approval-machine = Machine
skip-approval-setting = Setting
skip-approval-choice = Switch to
skip-approval-scope = This covers this agent's session only.
skip-approval-other-window = This window has not heard this machine offer it. Choose it again from the agent's controls to answer here.
skip-approval-unknown-title = A request this window does not understand
skip-approval-unknown = Update your window to answer it. Nothing was signed.
skip-approval-dismiss = Dismiss
skip-approval-read-first = Read it first, then you can allow.
start-approval-title = { $name } on { $machine } asks to start a { $kind } agent
start-approval-title-nomachine = { $name } asks to start a { $kind } agent
approval-someone = An agent
start-approval-helper = The agent asking is itself a helper
start-approval-folder = Folder
start-approval-worktree = in its own worktree
start-approval-outside = This folder is outside the asking agent's own folder.
start-approval-prompt = Its first prompt
start-approval-no-prompt = No prompt: it starts and waits.
start-approval-scope = What to allow
start-approval-once = Only this one
# The second choice of the grant's segmented control (#1070): short, so it is never cut; start-approval-more says it in full under it.
start-approval-more-short = More like this
start-approval-no-grant = Only this one can be allowed: this kind runs any program it is given, so it is never started without asking.
start-approval-settings-once = Only this one can be allowed: a start with its own model or effort is always asked.
start-approval-options-once = Only this one can be allowed: your answer covers this start with exactly the launch options shown, and every such start is asked again.
start-approval-does-not-ask = If you allow it, this helper runs commands and changes files without asking you first, for as long as it runs.
start-approval-more = Up to { $count } { $kind } agents in this folder while { $name } runs, without asking again
start-approval-nested = Its helpers may ask to start helpers too (you answer each)
approval-review = Review
approval-review-for = Read the whole request from { $name }
approval-review-close = Close
start-summary-outside = outside its folder
start-summary-more = { $count ->
    [one] 1 more line
   *[other] { $count } more lines
}
helper-started-by = started by { $name }
helper-grants = Helper permissions
helper-grant-line = Can still start { $count } { $kind } without asking in
helper-grant-revoke = Revoke
helper-grants-failed = Could not read or change the helper permissions.
device-title = This device
device-name = Device name
device-key = Device key
device-link = Settings
device-fingerprint = Fingerprint
device-trusted-here = trusts this browser
device-not-trusted-here = does not trust this browser yet
device-add-title = Add this browser: approve it on another device of yours
device-add-intro = If one of your machines already trusts another device of yours (a browser or the terminal client), that device can approve this one. It sees a card with this browser's fingerprint, { $fingerprint }: check it is the same.
device-ask = Ask another device to approve
device-asked = Asked. Open the window on a device your machines trust: it shows a card for this browser. Its fingerprint must read { $fingerprint }.
device-ask-failed = Could not ask. Check the connection and try again.
device-command-intro = Or run this on the machine (if `agora` is not found there, run it in the folder of the agora program: ./agora, or .\agora.exe on Windows):
device-others = Your other devices
device-removed = Removed
device-waiting = Waiting for your approval
device-trusted-on = Trusted on { $machines }
device-trusted-nowhere = Not trusted on any machine
device-last-used = used { $when }
device-remove = Remove
device-remove-failed = Could not remove it.
device-request-title = { $name } asks to be trusted
device-request-compare = Compare it with the fingerprint that device shows under "This device". Approve only if they are the same.
device-request-machines = On these machines
device-request-all = All of them, and any machine that trusts this device later.
device-request-some = Only the ones checked.
device-request-failed = The approval could not be sent. Try again.
device-request-later = Not now
device-request-approve = Approve
# ADR-0065: a machine that wants to join, or a browser on another machine that wants to sign in, waits for the owner's answer.
pair-machine-title = A new machine wants to join
pair-browser-title = A browser wants to sign in
pair-name = Name
pair-system = System
pair-address = Address
pair-number = Number
pair-failed = Your answer could not be sent. Try again.
pair-deny = Deny
pair-approve = Numbers match…
pair-machine-compare = Compare with the number on the new machine. Approve only if they are the same.
pair-browser-compare = Compare with the number on that browser. Approve only if they are the same.
pair-expires-in = Runs out in { $time }
pair-ran-out = This request has run out. The new machine can ask again.
pair-offline = Not connected right now. You can answer once the connection is back.
pair-confirm = Do the two numbers match exactly?
pair-confirm-yes = Yes, the numbers match
pair-back = Go back
pair-hide = Hide
pair-replace = Replace the existing machine
pair-replace-why = A machine named "{ $name }" has already joined. If you replace it, the old one loses its access.
pair-sent = Answer sent.
pair-row-where = wants to join
pair-row-what = Compare its number with the one on its screen before you approve.
pair-review-for = Review the request from { $name }
pair-section = { $count ->
    [one] 1 machine wants to join
   *[other] { $count } machines want to join
}
pair-recent = Recent requests to join
pair-result-approved = { $name } was approved and is joining.
pair-result-denied = { $name } was denied.
pair-result-expired = The request from { $name } ran out.
pair-result-closed = The request from { $name } was cancelled.
pair-window-label = New machines
pair-window-allow = Allow a new machine to join
pair-window-help = Or let a new machine ask to join without a code: allow it here, then run agora join on that machine. It stays open for 10 minutes.
pair-window-open = Open for new machines. It closes in { $time }.
pair-window-open-help = On the new machine, run agora join with this computer's address. Both screens show a number: approve only if they are the same.
pair-window-close = Close
pair-window-failed = That did not work. Try again.
os-windows = Windows
os-macos = macOS
os-linux = Linux
signin-waiting = Waiting for the owner to approve this browser…
signin-waiting-hint = The owner sees this number on a card in a window they are already signed in to, and approves only if it matches.
signin-denied = The owner did not approve this browser.
signin-expired = Nobody approved this browser in time.
signin-ask-again = Ask again
signin-use-code = Use a code instead
trust-ask = Ask another device to approve
trust-asked = Asked. Approve this browser on a device that machine trusts, then start again.

signin-title = Sign in to your Agora
signin-intro = On the machine where Agora started, run this command, then type the code it shows here.
signin-code = Code
signin-action = Sign in
signin-refused = That code did not work. It may be wrong, expired or already used.
signin-too-many = Too many wrong tries from here. Wait a while and try again.
signin-failed = Could not reach your Agora. Try again.
signin-password = Screen password
signin-password-hint = You are signing in from another machine, so your Agora also asks for its screen password.
signin-refused-password = That did not work. The code may be wrong, expired or already used, or the password may be wrong.
lock-now = Lock
lock-now-hint = Sign this window out. Agents keep running.
password-title = Screen password
password-current = Current password
password-new = New password
password-again = Type it again
password-set = Set password
password-change = Change password
password-saved = Saved. Every other window was signed out.
password-refused = That password is not right.
password-too-many = Too many wrong tries. Wait a while and try again.
password-busy = Your Agora is busy. Try again in a moment.
password-too-short = Use at least { $min } characters.
password-mismatch = The two passwords do not match.
password-failed = Could not reach your Agora. Try again.
idle-lock = Lock this window when idle
idle-lock-off = Never
idle-lock-minutes = After { $minutes } minutes
idle-lock-hint = Kept in this browser only. An idle window signs out like Lock; agents keep running. Signing in again takes a code (and from another machine the screen password) or a passkey.
idle-lock-hint-no-password = Kept in this browser only. An idle window signs out like Lock; agents keep running.
audit-action-sign-out = Signed out
audit-action-password-set = Set the screen password
audit-action-password-clear = Removed the screen password
machines-title = My machines
machines-empty = No machines connected yet.
machine-online = Online
machine-offline = Offline
machine-last-seen = last seen { $when }
# The Agents list: an offline machine's row ends with how long it has been away ("offline · 2 hr").
explorer-offline-for = offline · { $when }
machine-agents =
    { $count ->
        [one] { $count } agent running
       *[other] { $count } agents running
    }
palette-go-machines = Show my machines
palette-language = Language: { $name }
agent-tokens = { $count } tokens used

start-title = Start an agent
start-where = Where
start-which = Which agent
start-loading = Asking the machine…
start-kinds-failed = The machine did not answer. Try again.
start-no-kinds = This machine has no agents set up yet.
start-no-projects = No project is set up on a connected machine. Run "agora node run --project PATH" on one.
start-not-installed = not installed
start-not-installed-why = This machine cannot find this agent's program in its PATH. Install it there, or start Agora on that machine from a terminal where it runs.
start-not-installed-program = This machine cannot find { $program } in its PATH. Install it there, or start Agora on that machine from a terminal where { $program } runs.
start-group-installed = Installed
start-group-missing = Not installed
start-group-untested = Untested
start-runs-terminal = Terminal
start-runs-chat = Chat
# The name a new agent will have on Home and in the tabs: its kind, then a colour picked when it starts ($name is "codex · …").
start-preview = Will appear as { $name }, with a colour picked when it starts.
start-go = Start
start-cancel = Cancel
start-kind-unknown = That machine does not know this agent.
start-program-missing = That machine cannot find this agent's program in its PATH. Install it there, or start Agora on that machine from a terminal where it runs.
start-limit = That machine is at its limit. Stop an agent and try again.
start-failed = The agent did not start. Try again.
signin-passkey = Use a passkey
signin-passkey-failed = The passkey did not work. Use a code instead.
device-passkey-intro = Add a passkey to sign in on this device without a code.
device-passkey-add = Add a passkey
device-passkey-added = Passkey added. Next time, choose "Sign in with a passkey".
device-passkey-failed = Could not add the passkey.
board-title = Shared tasks and files
board-open = open
board-done = done
board-project-identity = Project identity
board-project-folders = Machines and folders ({ $count })
board-open-view = Open board ({ $count })
board-view-title = Board: { $name }
board-view-empty-title = No tasks yet
board-view-empty-hint = Tasks that agents add for this project appear here.
board-col-backlog = Backlog
board-col-doing = Doing
board-col-done = Done
board-col-backlog-empty = Nothing waiting.
board-col-doing-empty = Nothing in progress.
board-col-done-empty = Nothing finished yet.
board-task-untitled = Untitled task
board-card-owner = Agent: { $name }
board-card-holds = Working on files ({ $count })
board-card-more-paths = and { $count } more
board-show-more = Show more ({ $count } of { $total })
# ADR-0095 step 2: giving a Backlog task to an agent.
give-action = Give to an agent
give-action-label = Give “{ $title }” to an agent
give-title = Give to an agent
give-pick-label = Agents that can take this task
give-pick-hint = Running agents of { $project }. Helpers are listed under the agent that started them.
give-none = No agent of { $project } is running on a connected machine that can take tasks. Start one first.
give-row-detail = { $kind } on { $machine }
give-row-helper = Helper · { $kind } on { $machine }
give-confirm-title = Give this task to { $agent }?
give-field-task = Task
give-field-agent = Agent
give-field-kind = Kind
give-field-machine = Machine
give-field-project = Project
give-confirm-no-ask = { $agent } acts without asking: it works on this task without asking you before it changes files or runs commands.
give-confirm-body = The agent gets the task as a message. If it is in the middle of a turn, the task waits until that turn ends. The agent keeps its own settings, and anything that needs your OK still asks you.
give-send = Give the task
give-sending = Sending…
give-back = Back
give-cancel = Cancel
give-sent = Sent to { $agent }.
give-queued = Sent to { $agent }. It waits until the agent's current turn ends.
give-done = Done
give-other-agent = Choose another agent
give-failed = The task was not sent. Try again.
give-refused-paste = This agent's terminal program cannot take a pasted task. Choose another agent, or type the task into it yourself.
give-refused-waiting = This agent already has a task waiting for its turn to end. Wait for it, or choose another agent.
give-refused-limited = You gave { $limit } tasks to agents in the last minute. Wait a moment, then try again.
give-refused-grant = This machine needs your permission first. Allow it in the dialog and the task is sent.
give-refused-offline = That machine is not connected right now.
give-refused-forbidden = You are not allowed to give tasks to this agent.
give-target-gone = That agent stopped, was closed or left this project. Nothing was sent; choose another agent.
give-grant-declined = The machine's permission was not given, so nothing was sent. You can try again.
# ADR-0095 step 3: starting a new agent for a board task, then giving it the task.
give-start-new = Start a new agent for this task
give-none-start = No agent of { $project } is running on a connected machine that can take tasks. Start one for this task.
give-start-waiting = The agent has started. The next step shows as soon as it appears here.
give-start-unnamed = The agent started, but its machine did not say which one it is. Choose it from the list once it appears.
give-queued-new = Sent to { $agent }. It waits until the agent is ready and its current turn ends.
start-for-task = For the task “{ $title }” of { $project }
start-for-task-other-project = This folder is not part of { $project }. The task can only be given to an agent working in its project: choose one of its folders.
give-none-plain = No agent of { $project } is running on a connected machine that can take tasks.
give-start-late = The agent has not appeared yet. Go back and choose it from the list once it shows up.
give-machine-unsupported = That agent's machine cannot take tasks yet; it may need an update. Nothing was sent.
message-cross = between machines
# Only that machine says who sent this message; the sender's own machine has not confirmed it.
message-via = reported by { $machine }
console-agents = Agents
console-activity = Activity
console-wall-empty = Pick an agent on the left to watch its terminal here.
console-maximize = Full size
console-hide = Stop watching
console-no-messages = Agents have not written to each other yet.
changes-title = Uncommitted changes
changes-none = Nothing uncommitted. Every workspace is clean.
changes-files = { $count ->
    [one] 1 file changed
   *[other] { $count } files changed
}
diff-title = What { $name } changed
diff-close = Close
diff-failed = Could not read the changes from that machine.
changes-summary = { $count ->
    [one] 1 file
   *[other] { $count } files
}
changes-filter = Show
changes-filter-all = All
changes-filter-modified = Changed
changes-filter-added = Added
changes-filter-deleted = Deleted
changes-filter-renamed = Renamed
changes-status-modified = Changed
changes-status-added = Added
changes-status-deleted = Deleted
changes-status-renamed = Renamed
changes-badge-modified = M
changes-badge-added = A
changes-badge-deleted = D
changes-badge-renamed = R
changes-search = Filter by name
changes-layout = Layout
changes-layout-unified = Unified
changes-layout-split = Side by side
changes-wrap = Wrap lines
changes-wrap-long = A long change shown whole is drawn one line per row, so it does not wrap; scroll sideways instead.
changes-refresh = Read the changes again
changes-list = Changed files
changes-top-folder = Top folder
changes-no-match = No changed file matches.
changes-pick = Pick a file on the left to see what changed in it.
changes-renamed-from = was { $name }
changes-whole-file = Whole file
changes-prev = Previous file
changes-next = Next file
changes-keys = next and previous file
changes-binary = This file is not text, so its changes are not shown.
changes-no-lines = Only the name or the permissions of this file changed.
changes-secret = Changed, content hidden
changes-no-eol = No line break at the end of the file
changes-gap = { $count ->
    [one] Show 1 unchanged line
   *[other] Show { $count } unchanged lines
}
changes-gap-rest = Show the rest of the file
changes-gap-failed = Could not read this file to show the unchanged lines.
changes-retry = Try again
changes-stale = The file has changed since these changes were read, so the unchanged lines are not shown. Read the changes again.
changes-truncated = These changes are larger than 512 KiB, so only the first part was read. The last file in the list is cut off.
changes-truncated-count = These changes are larger than 512 KiB, so only the first { $count ->
        [one] file was
       *[other] { $count } files were
    } read. The machine counts { $total } changed files: at least { $min ->
        [one] one is
       *[other] { $min } are
    } not listed here. The last file in the list is cut off.
files-truncated = The changes are larger than 512 KiB, so files after the cut may have no change mark.
files-truncated-count = The changes are larger than 512 KiB, so files after the cut have no change mark. The machine counts { $total ->
        [one] one changed file
       *[other] { $total } changed files
    }; { $count ->
        [one] one is
       *[other] { $count } are
    } marked.
changes-partial = This file is cut off: the changes were too large to read at once.
changes-eol = { $count ->
    [one] One changed line differs only in its line ending (LF or CRLF); its text is the same.
   *[other] { $count } changed lines differ only in their line endings (LF or CRLF); their text is the same.
}
changes-load-file = Read only this file
changes-load-file-failed = This file alone is still too large to read.
changes-show-all = { $count ->
    [one] Show all 1 line
   *[other] Show all { $count } lines
}
console-files = Files
files-title = Files of { $name }
files-tree = Folders and files
files-pick = Pick a file on the left to read it.
files-binary = This file is not text, so it is not shown.
files-too-large = This file could not be read (it may be too large).
files-secret = Kept private: it may hold a secret
files-secret-row = { $name }, kept private
files-failed = Could not list this folder.
files-filter = Filter files by name
files-only-changed = Show only the changed files
files-changed-count = { $count ->
    [one] 1 changed
   *[other] { $count } changed
}
files-refresh = Read the folders and changes again
files-no-match = No file matches. Folders not opened yet are not searched.
files-deleted = This file was deleted; its changes are in the review screen.
files-empty = This folder is empty.
files-folder-changed = Holds changed files
files-key-filter = filter files
files-key-find = find in the file
files-open-files = Open files
files-close-tab = Close { $name }
files-path = Path
files-lines = { $count ->
    [one] 1 line
   *[other] { $count } lines
}
files-changed-lines = { $count ->
    [one] 1 line changed
   *[other] { $count } lines changed
}
files-find = Find in the file
files-find-none = No match
files-find-at = { $used } of { $total }
files-find-prev = Previous match
files-find-next = Next match
files-find-close = Close the search
files-copy = Copy the whole file
files-copied = Copied
files-reload = Read this file again
files-mark-add = Added since the last commit
files-mark-mod = Changed since the last commit
files-mark-del = Lines removed here since the last commit
filter-placeholder = Filter by agent, machine or project
filter-none = Nothing matches.
console-drag = Drag this bar onto another terminal to swap their places
prompt-intro = This agent works through requests, not a terminal. Tell it what to do.
# ADR-0106 (#1140): the same, in a window that cannot send it anything (a viewer's).
prompt-intro-watching = This agent works through requests, not a terminal.
prompt-label = What should it do?
prompt-send = Send
prompt-sent = Sent.
prompt-failed = It did not go through. Try again.
prompt-no-answer = The agent did not confirm it got this. Wait a moment, and send it again only if nothing happens.
prompt-agent-exited = The agent stopped, so it did not get this. Start the agent again.
console-history = History
history-title = Commits of { $name }
history-none = Nothing has been committed here yet.
history-failed = Could not read the history from that machine.
history-no-subject = (no message)
chat-empty = Nothing said yet. Tell it what to do below.
chat-empty-watching = Nothing said yet.
chat-log = Conversation
chat-plan = Plan
chat-plan-steps =
    { $count ->
        [one] Plan · { $count } step
       *[other] Plan · { $count } steps
    }
chat-plan-earlier =
    { $count ->
        [one] Earlier plan · { $count } step
       *[other] Earlier plan · { $count } steps
    }
chat-steps-earlier =
    { $count ->
        [one] { $count } earlier step
       *[other] { $count } earlier steps
    }
chat-working = Working…
chat-copy = Copy
chat-copied = Copied
# What an agent asked to do, when it gave no title of its own. Neutral: the call may still be running or be refused.
tool-read = Read
tool-edit = Edit
tool-run = Command
tool-search = Search
tool-fetch = Web page
tool-delete = Delete
tool-move = Move
tool-think = Thinking
tool-other = Step
approval-full-request = Full request
notice-adapter-turn_failed = The agent's turn failed (error { $rpc_code }). Its provider may have refused the request; try again or choose another model. { $stderr_tail }
notice-adapter-turn_stopped = The agent stopped its turn ({ $stop_reason }).
notice-adapter-turn_empty = The agent ended its turn without saying anything. Its provider may have refused the request. { $stderr_tail }
notice-adapter-agent_exited = The agent stopped (step { $stage }, exit code { $exit_code }). Its last output: { $stderr_tail }
notice-adapter-agent_refused = The agent refused to start (step { $stage }, error { $rpc_code }): { $detail }
notice-adapter-no_answer = The agent did not confirm it got this request within { $waited_ms } ms. It may still be working on it.
notice-adapter-queue_dropped = { $count } waiting prompts were not sent: the turn was cancelled with the queue dropped, the agent ended, or a prompt could not be sent.
notice-adapter-output_dropped = The agent kept running while Agora was restarting. { $bytes } bytes of what it said in that time were lost, so part of the conversation from that time may be missing.
notice-adapter-restarting = The agent stopped unexpectedly. Agora is starting it again (attempt { $attempt }); the conversation goes on where it can.
notice-adapter-gave_up = The agent stopped unexpectedly again after { $restarts } restarts (last exit code { $exit_code }), so Agora stopped starting it again. Start a new agent to go on.
notice-adapter-gave_up-no_code = The agent stopped unexpectedly again after { $restarts } restarts, so Agora stopped starting it again. Start a new agent to go on.
notice-other = The agent reported something this window cannot show ({ $code }).
notice-approval-always_only_forbidden = The agent asked for “{ $action }” and offered only an answer it would remember. This machine allows single answers only, so it was refused.
notice-adapter-permission_unanswerable = The agent asked for “{ $action }” without offering a way to allow it, so it was refused.
notice-chat-history_not_saved = This conversation could not be saved on its machine (its disk may be full). The agent keeps working, but turns from now on may be missing after Agora restarts.
session-header = Agent settings
session-elapsed-hint = How long it has been running
session-mode = Mode
session-model = Model
session-not-reported = Not reported
session-mode-unasked = { $name } (does not ask)
session-context = Context { $percent }%
session-context-under-one = Context <1%
session-context-detail = { $used } of { $size } tokens in the agent's context
session-change-failed = The agent did not switch.
session-waiting-approval = Waiting for your approval
session-waiting-approval-note = Answer the request to switch. Nothing has changed yet.
session-waiting-confirm = Waiting for the agent
session-skip-approved-pending = Approved. The agent has not confirmed the change yet; it shows here when it does.
session-skip-unconfirmed = The agent has not reported the change yet. It shows here when it does.
session-skip-not-applied = Not switched: the request was declined, ran out of time or was withdrawn. The agent keeps asking as before.
session-forbidden-hint = not allowed on this machine
session-confirm-title = Let the agent act without asking?
session-confirm-body = In { $name } the agent runs commands and changes files without asking you first.
session-confirm-yes = Switch anyway
session-confirm-no = Keep asking
grant-title-typing = Allow typing into { $agent } for 1 hour?
grant-title-prompt = Allow sending messages to { $agent } for 1 hour?
grant-title-stop = Allow stopping { $agent } for 1 hour?
grant-title-interrupt = Allow interrupting the current turn of { $agent } for 1 hour?
grant-title-settings = Allow changing the mode, model or settings of { $agent } for 1 hour?
grant-title-resize = Allow resizing the terminal of { $agent } for 1 hour?
grant-title-watch = Allow watching { $agent } for 1 hour?
grant-title-start = Allow starting agents in this project for 1 hour?
grant-title-task = Allow giving tasks to { $agent } for 1 hour?
grant-title-other = Allow this window to act on { $machine } for 1 hour?
grant-explain = { $machine } asks for your permission before it lets this window do that. Your key on this device signs it, and it ends by itself.
grant-allow = Allow for 1 hour
grant-allowing = Allowing…
grant-later = Not now
grant-failed = The permission did not go through. Try again.
audit-title = Activity
audit-intro = Who did what on this hub, newest first. Only the owner sees this.
audit-person = Person
audit-session = session { $id }
audit-session-hint = Each sign-in is its own session: two windows of the same person show different ones.
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
audit-action-pair-open = Let new machines ask to join
audit-action-pair-close = Stopped new machines from asking to join
audit-action-pair-status = Checked whether new machines may ask to join
audit-action-pair-approve = Let a machine join
audit-action-pair-deny = Turned a machine away
audit-action-pair-request = A machine asked to join
audit-action-pair-reveal = A machine checked its request to join
audit-action-pair-issued = A machine was given its certificate
audit-action-pair-refused = Requests to join were turned away
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
start-directory-forbidden = This device is limited to other projects on this machine. Choose a folder of a project you work in, or ask the machine's owner.
start-directory-role = Your role does not let you choose this folder. Ask the owner.
start-role = Your role does not let you start agents here. Ask the owner if you need to.
workspace-shared = Shared folder · changes belong to the workspace
filter-project = Project
filter-all-projects = All projects
filter-project-machines = { $name } · { $count } machine(s)

start-directory-agent-unsupported = This agent needs an isolated working copy to load its configuration.

audit-action-directoryinspect = Selected a folder
audit-action-agentlaunch = Started an agent in a folder
audit-action-workspacequery = Read workspace files or changes
audit-action-node-resynced = Reconnected a machine; part of its history is missing

agent-address = Agent address
agent-address-copy = Copy agent address
address-copy = Copy
address-copied = Copied
address-copy-failed = Copy failed. Select the address and copy it manually.
machine-identity = Machine ID: { $id }
machine-name-id = { $name } · { $id }

language-label = Language
language-system = Browser default
language-en = English
language-tr = Türkçe
language-not-saved = Language changed for this visit. Your browser could not save the preference.

# The IDE-style window: title bar, agent tree, tabs, details panel, status bar.
ide-command = Search or run a command
ide-new-tab = Start an agent
ide-close-tab = Close { $name }
ide-hide-panel = Hide this panel
ide-show-agents = Show the agent list
ide-show-details = Show details
ide-layout-single = One terminal at a time
ide-layout-grid = Terminals side by side
ide-details = Details
ide-welcome-title = Pick up where you left off
ide-welcome-running = Running now
ide-welcome-none = No agent is running yet.
ide-welcome-search = Search and commands
ide-welcome-next = Jump to the next agent that needs you
ide-add-here = Start an agent on { $machine }
ide-toggle-machine = Show or hide the agents on { $name }
ide-status-agents =
    { $count ->
        [one] { $count } agent
       *[other] { $count } agents
    }
ide-status-needs =
    { $count ->
        [one] { $count } needs you
       *[other] { $count } need you
    }
ide-folder-known = Known folders
ide-folder-change = Change folder
ide-folder-chosen = Working folder
ide-agent-pick = Agent
ide-options = Options
ide-copy-address = Copy address
ide-tab-changes = Changes
ide-tab-messages = Team chat
ide-more-tabs = More tabs
ide-more-tabs-needs = a hidden tab needs you
ide-more-tabs-failed = a hidden tab ended with an error
ide-more-tabs-working = a hidden tab is working
theme-label = Colour scheme
theme-system = Follow the system
theme-system-short = System
ide-shared = Shared
ide-memory = Memory used by the agents on this machine
network-hub = Your Agora
network-build = Version running on this machine
network-never = never
network-heard = heard { $when }
network-last-seen = last seen { $when }
network-terminals =
    { $count ->
        [one] { $count } terminal open
       *[other] { $count } terminals open
    }
terminal-size = Terminal text size
terminal-smaller = Smaller text
terminal-larger = Larger text
terminal-settings = Terminal settings
terminal-line-height = Row height
terminal-line-height-note = 1.0 is the row height of a real terminal
terminal-history = Lines of history kept
terminal-readable = Make hard-to-read colours readable
terminal-copy-on-select = Copy what I select
terminal-gpu = Sharper block and line drawing; try it, turn it off if text looks wrong
terminal-jump-bottom = Jump to latest
terminal-find = Find in terminal
terminal-find-next = Next match
terminal-find-previous = Previous match
terminal-find-close = Close find
terminal-find-none = No match
terminal-copy-offer = A program in this terminal wants to copy this to your clipboard:
terminal-copy-offer-length = ({ $count } characters)
terminal-copy-offer-copy = Copy
terminal-copy-offer-ignore = Ignore
closed-section = Closed
closed-clear = Clear closed
closed-clear-short = Clear
closed-dismiss = Remove from this list
closed-forget = Remove for everyone
closed-ok = Finished
closed-failed = Ended with an error (exit code { $code })
closed-failed-signal = Ended with an error
closed-note = Its terminal is gone. This is what it left behind.
closed-machine = Machine
closed-folder = Folder
closed-branch = Branch
closed-ended-label = Ended
closed-earlier = before this window opened
closed-ran-label = Ran for
closed-changes-label = Left behind
closed-changes-none = No uncommitted changes
closed-tokens-label = Used
closed-review = Review the changes
ide-resize = Resize the panel
start-browse = Browse folders
start-browse-up = Up one folder
start-browse-close = Close the folder list
start-browse-empty = No folders here.
start-browse-more = There are more folders than are shown. Type the path to go further.
start-browse-failed = The folder could not be listed.
start-browse-git = A Git project
closed-clear-everyone = Remove every closed agent for everyone
closed-forget-failed = The hub did not remove it: { $reason }
closed-reason-forbidden = you are not allowed to remove it
closed-reason-running = it is running again
closed-reason-gone = the hub no longer knows it
closed-reason-other = the hub could not be reached or refused

trust-hint = { $machine } has to trust this browser once before it starts agents from it, so that the hub alone can never run programs there. In a terminal on { $machine }, run this command, check that the fingerprint it shows matches "This device", and answer y. Then press Start again:
trust-hint-binary = { $machine } has to trust this browser once before it starts agents from it, so that the hub alone can never run programs there. Agora runs there from { $where }, which is not on its PATH: in that folder, in a terminal (on Windows, PowerShell), run this command, check that the fingerprint matches "This device", and answer y. Then press Start again:
trust-hint-file = { $machine } has to trust this browser once before it starts agents from it, so that the hub alone can never run programs there. Its Agora program ({ $file }) is not on its PATH: in the folder where that file is, in a terminal (on Windows, PowerShell), run this command, check that the fingerprint matches "This device", and answer y. Then press Start again:
trust-show-path = Show the whole path
trust-copy = Copy
trust-copied = Copied
interrupted-section = Stopped when the machine shut down
interrupted-section-short = Stopped
interrupted-hint = These were running when the machine stopped.
interrupted-note = It was running when its machine stopped. Resume continues its conversation in the same folder.
interrupted-note-new = It was running when its machine stopped. Its conversation cannot be continued; Start again opens a new one in the same folder.
interrupted-note-running = Its process from before the machine's restart is still running. Stop it first; then it can be resumed. Or leave it running.
action-stop-old = Stop it
action-resume = Resume
action-start-again = Start again
resume-failed = It did not start. Try again.
resume-failed-folder = Its folder is gone or now belongs to another project, so it was not started.
resume-failed-parent = The agent that started it is not running. Resume that one first.
resume-failed-session = Its conversation cannot be continued. Use Start again.
resume-failed-gone = That machine no longer offers it.
resume-failed-unsupported = That machine cannot resume agents yet. Update it.
resume-failed-still-running = Its process from before the restart is still running. Stop it first.
resume-failed-offline = That machine is offline.
resume-failed-forbidden = This device is limited to other projects on this machine, so it cannot start this agent.
resume-failed-role = Your role does not let you start this agent again. Ask the owner.

# ADR-0059: launch options (manifest `[[launch.option]]` labels)
option-skip-permissions = Don't ask
setting-model = Model
setting-effort = Effort
option-ask = Ask before acting
start-how-it-acts = How it acts
start-confirm-skip-body = With "{ $name }" chosen, the agent runs commands and changes files without asking you first, for as long as it runs.
start-confirm-skip-yes = Start without asking
start-does-not-ask-note = This agent will act without asking you.
start-option-forbidden = This machine does not let agents act without asking. Choose "Ask before acting".
start-option-confined-only = This machine lets an agent act without asking only while it is confined to its folder, and it would not be here.
start-options-unsupported = This machine is too old for start options. Update it, or choose "Ask before acting".
agent-does-not-ask = Runs without asking
agent-does-not-ask-explain = Started so that it acts without asking you first
agent-does-not-ask-session-neutral-explain = Acts without asking you first, until it ends or is switched back
agent-does-not-ask-session-explain = Switched, with your approval, so that it acts without asking you first, until it ends or you switch it back
agent-always-allowed-short = Asks less
agent-always-allowed-explain = You allowed { $count } of its requests for this session. It does not ask again for those until it ends.
agent-always-allowed-explain-kept = { $count ->
    [one] You let it remember the answer to one of its requests. It does not ask again for that; it decides how long it keeps the answer.
   *[other] You let it remember the answers to { $count } of its requests. It does not ask again for those; it decides how long it keeps them.
}
agent-does-not-ask-cause = Because of: { $value }
start-option-unconfirmed = This machine says that option lets the agent act without asking, and this window did not ask you first. Nothing was started; open the start window again.
start-option-invalid = This machine does not offer that start option. Nothing was started; open the start window again.
# Layouts (ADR-0068): panes side by side, tabs, saved layouts.
layout-menu = Layout
layout-menu-back = Back
layout-preset-1x1 = One pane with tabs
layout-preset-2x1 = Two side by side
layout-preset-1x2 = Two, one above the other
layout-preset-2x2 = Four (2 × 2)
layout-preset-3x2 = Six (3 × 2)
layout-preset-focus = One large, three small
layout-maximize = Maximize
layout-restore = Restore
layout-save-as = Save layout as…
layout-saved = Saved layouts…
layout-more = More
layout-export = Export
layout-import = Import…
layout-reset = Reset layout
layout-split-right = Split right
layout-split-down = Split down
layout-close-pane = Close pane
layout-pane-menu = Pane options: { $name }
layout-move-to = Move to…
layout-swap-with = Swap with…
layout-move-out = Take out of the tabs
layout-place-before = Beside it, before
layout-place-after = Beside it, after
layout-place-above = Above it
layout-place-below = Below it
layout-place-tab = As a tab
layout-show = Show…
layout-follow-attention = What needs you next
layout-follow-selection = The agent selected in the list
layout-follows-attention = Follows what needs you
layout-follows-selection = Follows the selection
layout-show-agent = { $name }
layout-panel-agent = Agent
layout-show-agent-itself = This agent
layout-panel-empty = Empty pane
layout-panel-unknown = Unknown panel “{ $id }”: this window cannot show it.
layout-drag = Drag onto another pane: an edge puts it beside, the middle makes it a tab
layout-empty-title = Choose what this pane shows
layout-nothing-selected = Select an agent in the list to see it here.
layout-gone = This agent is no longer here.
layout-files-none = Files are shown once this window is connected to your machines.
layout-no-terminal = This agent is a chat; it has no terminal.
layout-no-conversation = This agent has a terminal, not a chat.
layout-pane = Pane: { $name }
layout-divider = Resize the panes
layout-save-title = Save the layout
layout-save-name = Name
layout-save-replaces = A saved layout with this name will be replaced.
layout-cancel = Cancel
layout-save = Save
layout-list-title = Saved layouts
layout-list-empty = No saved layouts yet. Use “Save layout as…” in the layout menu.
layout-apply = Use
layout-rename = Rename
layout-delete = Delete
layout-delete-named = Delete { $name }
layout-close = Close
layout-import-title = Import a layout
layout-import-paste = Paste a layout (JSON)
layout-import-file = Or choose a file
layout-import-do = Import
layout-error-json = This is not valid JSON.
layout-error-invalid = This is not a layout this window can read.
layout-error-newer = This layout was made by a newer version. Update to use it.
layout-error-too-deep = This layout is nested too deeply (at most { $max } levels).
layout-error-too-many = This layout has too many panes (at most { $max }).

## Keyboard: commands and their keys (ADR-0068 rung 4, docs/ux/keybindings.md)
palette-keyboard = Keyboard settings
keys-cmd-agents = Show or hide the agent list
keys-cmd-tab = Go to tab { $n }
keys-cmd-side = Side panel, place { $n }
keys-cmd-side-named = Side panel: { $name }
keys-cmd-preset = Layout: { $name }
keys-cmd-maximize = Maximize or restore the pane
keys-title = Keyboard
keys-desc = Every action is a command in the command palette. Give it the keys you like; they are kept in this browser.
keys-changed-count = { $count ->
    [one] { $count } changed
   *[other] { $count } changed
}
keys-search = Find a command or a key
keys-import = Import…
keys-export = Export
keys-reset-all = Reset all
keys-unreadable = The keys kept in this browser could not be read, so the default keys are in use.
keys-unknown = { $count ->
    [one] Your keys name a command this window does not have. It is kept, and does nothing here:
   *[other] Your keys name { $count } commands this window does not have. They are kept, and do nothing here:
}
keys-list = Commands and their keys
keys-none-match = No command matches.
keys-fixed-title = Keys that stay as they are
keys-fixed-safe-mode = Safe mode
keys-fixed-navigation = Moving around the window
keys-fixed-terminal = A terminal's copy, find and paste
keys-fixed-editing = Editing text
keys-terminal-note = While a terminal has the keyboard, its program gets your keys. Only keys it can never receive still reach the window: ⌘ keys, and Ctrl+Shift with a letter (Ctrl+Shift+K opens the command palette from a terminal).
keys-context-anywhere = Anywhere
keys-context-window = In the window
keys-context-pane = The pane in focus
keys-context-tabs = The tabs in focus
keys-context-side = The side panel
keys-changed = Changed
keys-press = Press the keys…
keys-press-second = { $key }, then a second key, or wait
keys-no-keys = No keys
keys-fixed = Fixed
keys-change = Change
keys-change-named = Change the keys of { $name }
keys-recording = Listening…
keys-clear = Clear
keys-clear-named = Clear the keys of { $name }
keys-reset = Reset
keys-reset-named = Give { $name } its default keys back
keys-conflict-same = { $key } is also the key of “{ $name }”.
keys-conflict-prefix = { $key } and “{ $name }” share their first key, so one of them cannot act.
keys-refused-fixed = { $key } stays as it is ({ $reason }).
keys-reset-title = Reset every key?
keys-reset-body = Every command gets its default keys back, and the changes kept in this browser are removed.
keys-reset-keep = Keep my keys
keys-reset-confirm = Reset all
keys-import-title = Import keys
keys-import-paste = Paste a keybindings file (JSON)
keys-import-file = Or choose a file
keys-import-refused = This file was not imported:
keys-error-json = This is not valid JSON.
keys-error-too-big = This file is too large (at most 64 KB).
keys-error-shape = This is not a keybindings file this window can read.
keys-error-newer = This file was made by a newer version. Update to use it.
keys-error-too-many = This file lists too many commands (at most 200).
keys-error-field = Unknown field “{ $name }”.
keys-error-command = “{ $command }” is not a command name.
keys-error-request = “{ $command }”: a request is answered on its card, never with a key.
keys-error-fixed-command = “{ $command }” keeps its keys; they cannot be changed.
keys-error-twice = “{ $command }” is listed twice.
keys-error-too-many-keys = “{ $command }” has too many keys (at most 4).
keys-error-key = “{ $key }” for { $command } is not a key this window knows.
keys-error-fixed-key = { $key } for { $command } stays as it is ({ $reason }).
keys-error-more = …and { $count } more.

## Themes (ADR-0068 rung 2, docs/ux/themes.md)
theme-editor-title = Theme
theme-edit = Edit theme
theme-follows-system = Following the system: showing the { $scheme } theme. Choose one theme for each.
theme-forced = Always { $scheme }. Follow the system to switch between your two themes by itself.
theme-for-light = Light theme
theme-for-dark = Dark theme
theme-name = Name
theme-built-in = built in
theme-built-in-hint = Built-in themes can't be changed. Customize makes a copy you can change.
theme-customize = Customize
theme-copy-name = { $name } (mine)
theme-colours = Colours
theme-shape = Text and shape
theme-readability = Readability
theme-incomplete = Some values are not valid yet; the window keeps the last good ones.
theme-contrast-ok = All text and controls are easy to see (WCAG AA).
theme-contrast-not-applied = Not applied: some text would be hard to read.
theme-contrast-warning = Applied, but some text or controls would be hard to see in places:
theme-contrast-also = Also hard to see in places:
theme-import-warning = Added. When it is shown, some text or controls would be hard to see in places:
theme-px = { $size } px
theme-import = Import…
theme-export = Export
theme-delete = Delete theme
theme-reset = Reset to defaults
theme-import-hint = Themes are kept in this browser. Export one to use it in another browser; Reset brings back Light and Dark and keeps your themes.
theme-name-dark = Dark
theme-name-light = Light
theme-name-high-contrast = High contrast
theme-name-large-text = Large text
theme-name-slate = Slate
theme-name-sand = Sand
theme-token-surface-0 = Background
theme-token-surface-1 = Panels
theme-token-surface-2 = Raised surfaces
theme-token-border = Lines
theme-token-text = Text
theme-token-text-muted = Secondary text
theme-token-accent = Accent
theme-token-on-accent = Text on accent
theme-token-state-working = Working
theme-token-state-needs-you = Needs you
theme-token-state-finished = Finished
theme-token-state-idle = Idle
theme-token-state-unknown = Unknown
theme-token-font-ui = Interface font
theme-token-font-mono = Terminal and code font
theme-token-text-16 = Text size
theme-token-space-1 = Spacing
theme-token-radius-xs = Corners: tiny
theme-token-radius-s = Corners: small
theme-token-radius-m = Corners: medium
theme-token-radius-l = Corners: large
# A role's soft coloured background (callouts, pills, the current row): the readability check names it as "{ $bg }".
theme-tint-accent = Accent background
theme-tint-working = Working background
theme-tint-needs = Needs you background
theme-tint-finished = Finished background
theme-tint-danger = Error background
theme-tint-idle = Idle background
# The one red (errors, deletions): fixed, not a theme colour.
theme-colour-danger = Red (errors)
theme-colour-on-danger = Text on red
theme-colour-surface-raised = Chosen option background
# A role's icon chip (the small square behind a role's icon).
theme-chip-accent = Accent icon background
theme-chip-working = Working icon background
theme-chip-needs = Needs you icon background
theme-chip-finished = Finished icon background
theme-chip-danger = Error icon background
theme-chip-idle = Idle icon background
# An agent's badge (its initial on its colour); $colour is one of the theme-agent-colour-* names.
theme-avatar = { $colour } agent badge
theme-agent-colour-amber = Amber
theme-agent-colour-teal = Teal
theme-agent-colour-violet = Violet
theme-agent-colour-rose = Rose
theme-agent-colour-slate = Slate
theme-agent-colour-sky = Sky
theme-agent-colour-mint = Mint
# The controls whose state is shown by a colour (they need 3:1 to be seen): named as "{ $control }" in theme-error-control.
theme-control-segmented = Chosen option ring
theme-control-risk-segmented = Risky chosen option ring
theme-control-switch-off = Switch (off)
theme-control-switch-on = Switch (on)
theme-control-risk-switch = Risky switch (on)
theme-control-selected-card = Selected card ring
theme-control-pressed-count = Selected count ring
theme-control-primary-button = Main button
theme-control-danger-button = Red button
theme-control-focus-ring = Focus ring
theme-control-icon = Icon
theme-error-json = This file is not JSON.
theme-error-invalid = This is not an Agora theme file.
theme-error-newer = This theme was made by a newer version of Agora.
theme-error-name = A theme needs a name of 1 to { $max } characters.
theme-error-scheme = The theme must say whether it is "dark" or "light".
theme-error-unknown-field = "{ $field }" is not part of a theme file.
theme-error-unknown-token = "{ $token }" is not a theme token.
theme-error-derived-token = "{ $token }" follows from "{ $base }": set "{ $base }" instead.
theme-error-bad-color = { $label } ("{ $token }") must be a colour like #1a2b3c.
theme-error-bad-size = { $label } ("{ $token }") must be between { $min } and { $max } px.
theme-error-bad-font = { $label } ("{ $token }") must be a list of font names separated by commas.
theme-error-contrast = { $fg } on { $bg } is { $ratio }:1; it needs at least { $min }:1 to be readable.
theme-error-control = { $control }: { $fg } on { $bg } is { $ratio }:1; it needs at least { $min }:1 to be seen.
theme-error-full = You have { $max } themes; delete one first.
# Theme format 2 (docs/ux/themes.md, ADR-0113): problems with a file, named by where they are ("dark.regions.sidebar.text").
theme-error-seen = { $fg } on { $bg } is { $ratio }:1; it needs at least { $min }:1 to be seen.
theme-error-no-look = A theme needs a "dark" or "light" part, or a built-in theme to start from ("inherits").
theme-error-moved-token = "{ $field }": "{ $token }" goes in "{ $to }".
theme-error-field-color = "{ $field }" must be a colour like #1a2b3c.
theme-error-field-size = "{ $field }" must be a whole number of pixels from { $min } to { $max }.
theme-error-field-list = "{ $field }" must be one of: { $values }.
theme-error-field-font = "{ $field }" must be a list of font names separated by commas.
theme-error-field-line-height = "{ $field }" must be "normal" or a number from { $min } to { $max }.
theme-error-field-switch = "{ $field }" must be true or false.
theme-error-field-group = "{ $field }" must be a group of named values.
theme-error-unsafe = "{ $field }" has text a theme can't hold (like "url(", ";" or "<"): a theme holds only colours, sizes, words from a list and font names.
theme-error-floor = "{ $field }" can't be set. { $reason }
theme-error-text-floor = "{ $field }": no text is smaller than 12 px, so the text size is at least { $min } px.
theme-warning-text-lifted = The text size { $from } would make some text smaller than 12 px: it is shown at { $to }.
theme-error-inherits = "{ $id }" is not a built-in theme to start from.
theme-error-too-large = This file is larger than { $max } KiB; a theme is far smaller.
theme-error-more = { $count ->
    [one] …and one more problem.
   *[other] …and { $count } more problems.
}
# A problem in one scheme of a theme that has both; $scheme is theme-dark or theme-light.
theme-issue-in-scheme = { $scheme }: { $text }
# What no theme can change (the floor): the row's name, and why.
theme-floor = Always the same
theme-floor-hint = No theme can change these, so the window never hides or fakes what needs you.
theme-floor-row-danger = Red (errors and danger)
theme-floor-row-diff-removed = Removed lines
theme-floor-row-approval-card = Requests from agents
theme-floor-row-focus-ring = Focus ring
theme-floor-row-needs-you = "Needs you"
theme-floor-row-plugin-marker = Plugin mark
theme-floor-danger = The red for errors and danger is always the same, so it can't be hidden or copied.
theme-floor-diff-removed = Removed lines are always the same red.
theme-floor-approval-card = A request from an agent always looks the same, so no theme can hide or fake what it asks.
theme-floor-focus-ring = The focus ring always shows where the keyboard is.
theme-floor-needs-you = "Needs you" always keeps its shape, its word and its place.
theme-floor-plugin-marker = What a plugin adds is always marked.
theme-locked = Locked
# The editor's groups and rows.
theme-scheme-editing = Showing the { $scheme } colours. Switch the scheme to edit the other ones.
theme-scheme-add = Add { $scheme } colours
theme-scheme-add-hint = This theme has only { $scheme } colours. Add the other ones to use it for both.
theme-regions = Parts of the window
theme-regions-hint = An empty field follows the colours above.
theme-follows = Follows the theme
theme-clear = Use the theme's colour
theme-region-sidebar = Agents list
theme-region-tab-bar = Tabs
theme-region-pane-bar = Pane bars
theme-region-status-line = Status line
theme-region-cards = Cards
theme-region-inputs = Text fields
theme-region-buttons = Buttons
theme-region-agent-entries = Agent replies
theme-region-person-entries = Your messages
theme-region-code-blocks = Code blocks
theme-region-part-surface = { $region }: background
theme-region-part-text = { $region }: text
theme-region-part-line = { $region }: lines
theme-part-surface = Background
theme-part-text = Text
theme-part-line = Lines
theme-terminal = Terminal
theme-terminal-hint = An empty field uses the window's own terminal colours.
theme-terminal-background = Terminal background
theme-terminal-foreground = Terminal text
theme-terminal-cursor = Cursor
theme-terminal-selection = Selection
theme-terminal-black = Black
theme-terminal-red = Red
theme-terminal-green = Green
theme-terminal-yellow = Yellow
theme-terminal-blue = Blue
theme-terminal-magenta = Magenta
theme-terminal-cyan = Cyan
theme-terminal-white = White
theme-terminal-bright-black = Bright black
theme-terminal-bright-red = Bright red
theme-terminal-bright-green = Bright green
theme-terminal-bright-yellow = Bright yellow
theme-terminal-bright-blue = Bright blue
theme-terminal-bright-magenta = Bright magenta
theme-terminal-bright-cyan = Bright cyan
theme-terminal-bright-white = Bright white
theme-syntax = Code colours
theme-syntax-hint = An empty field is mixed from the theme's own colours.
theme-syntax-keyword = Keywords
theme-syntax-string = Text in quotes
theme-syntax-comment = Comments
theme-syntax-number = Numbers
theme-syntax-function = Function names
theme-syntax-type = Types
theme-syntax-attr = Names and attributes
theme-syntax-meta = Other marks
theme-chrome = Window
theme-chrome-pane-borders = Pane borders
theme-chrome-pane-borders-auto = Between panes
theme-chrome-pane-borders-always = Around every pane
theme-chrome-pane-borders-off = None
theme-chrome-pane-gap = Space between panes
theme-chrome-line-width = Line width
theme-chrome-tab-bar = Tab bar
theme-chrome-tab-bar-top = Top
theme-chrome-tab-bar-bottom = Bottom
theme-chrome-tab-style = Tab style
theme-chrome-tab-style-underline = Line
theme-chrome-tab-style-filled = Filled
theme-chrome-tab-style-pill = Rounded
theme-chrome-density = Density
theme-chrome-density-compact = Compact
theme-chrome-density-normal = Normal
theme-chrome-density-roomy = Roomy
theme-chrome-state-display = Agent state in lists and tabs
theme-chrome-state-display-symbol = Symbol
theme-chrome-state-display-dot = Dot
theme-chrome-state-display-word = Word
theme-chrome-state-display-hint = The word is always in the tooltip and read aloud, and "Needs you" keeps its own symbol.
theme-chrome-icons = Icons
theme-chrome-icons-lines = Lines
theme-chrome-icons-hint = There is one icon set so far.
theme-chrome-shadow = Shadows
theme-chrome-shadow-none = None
theme-chrome-shadow-soft = Soft
theme-chrome-shadow-strong = Strong
theme-chrome-motion = Motion
theme-chrome-motion-system = Follow the system
theme-chrome-motion-reduced = Reduced
theme-chrome-motion-off = Off
theme-text-weight = Text weight
theme-text-weight-300 = Light
theme-text-weight-400 = Regular
theme-text-weight-500 = Medium
theme-text-line-height = Line height
theme-text-line-height-normal = Normal
theme-text-ligatures = Joined letters in code (ligatures)
theme-text-ligatures-hint = Not in terminals.
theme-preview = Preview
theme-preview-agent = The tests pass now.
theme-preview-person = Run the tests again.
theme-preview-field = Type here
theme-preview-button = Button
theme-preview-card = A card
theme-preview-status = 2 machines online
theme-preview-tab = Tab
theme-preview-pane = Pane
theme-preview-sample-command = $ npm test
theme-preview-sample-pass = ✓ 41 passed
theme-preview-sample-fail = ✗ 1 failed
theme-preview-sample-note = (warning) 2 skipped
palette-theme-editor = Edit theme
palette-theme = Theme: { $name }
palette-open-machine = Machine: { $name }
palette-open-project = Project: { $name }
palette-settings = Settings
device-appearance = Appearance
device-appearance-intro = This browser shows the { $name } theme. Themes are kept in this browser only.

## ADR-0067 rung 2: the phone-sized window and the terminal key bar.
phone-nav = Go to
phone-search = Search
phone-start = Start
phone-machines = Machines
phone-back = Back to the list
phone-offline = offline
phone-no-agents = No agents on this machine.
phone-needs = Needs you · { $count }
phone-answer-below = Answer with the buttons at the bottom of the screen.
phone-next = Next · { $count }
# The one trust notice at the top of the phone's requests (redesign PR 13), naming every machine that asks and has said it does not trust this device.
phone-untrusted-title = This device is not trusted yet
phone-untrusted-machines = { $count ->
    [one] { $machines } does not trust this device yet, so it will refuse an answer from here.
   *[other] { $machines } do not trust this device yet, so they will refuse an answer from here.
}
key-bar = Terminal keys
key-esc = Esc
key-tab = Tab
key-ctrl = Ctrl
key-up = Up
key-down = Down
key-left = Left
key-right = Right
key-paste = Paste

# ADR-0073: old machines, forgetting machines
old-machines-title = { $count ->
    [one] One old machine
   *[other] { $count } old machines
}
old-machines-explain = Not seen for more than a day. They are not offered for new agents. One that connects again is shown as usual.
old-machines-cleanup = Clean up…
forget-machine-action = Forget…
forget-machine-title = Forget { $name }?
forget-machines-title = { $count ->
    [one] Clean up one old machine?
   *[other] Clean up { $count } old machines?
}
forget-machines-explain = Forgotten machines and their finished agents leave every window. What they did stays in the history and the costs. A machine that connects again comes back.
forget-machine-agents = { $count ->
    [one] one agent was running then; it may still run there
   *[other] { $count } agents were running then; they may still run there
}
forget-machines-cancel = Keep
forget-machines-confirm = { $count ->
    [one] Forget
   *[other] Forget { $count }
}
forget-machine-reason-forbidden = Only the owner can forget machines.
forget-machine-reason-online = It is connected again, so it was not forgotten.
forget-machine-reason-gone = Your Agora no longer knows this machine.
forget-machine-reason-other = It could not be forgotten. Try again.

# My machines as cards (the redesign's PR 9): the page's header, the steps to add a machine, the strip of counts, a machine's card.
machines-intro = Every machine connected to your Agora. Agents keep running on a machine while it is away.
machines-add = Add a machine
machines-add-step-allow = Allow a new machine to join. It stays open for 10 minutes.
machines-add-step-run = On the new machine, run this command:
machines-add-command = agora join
machines-add-step-compare = Both screens then show a number. Approve here only if the two are the same.
machines-add-hint = Allow new machines, run agora join on it, then compare the number here.
machines-stat-of = of { $total }
machines-stat-online = { $total ->
    [one] machine online
   *[other] machines online
}
machines-stat-agents = { $count ->
    [one] agent running
   *[other] agents running
}
machines-stat-needs = { $count ->
    [one] needs you
   *[other] need you
}
machines-stat-memory = memory used by Agora
machine-memory = Agora on this machine
machine-memory-share = { $percent } % of the memory Agora uses on all your machines
machine-start-here = Start an agent here
machine-new-terminal = New terminal
machine-more = More for { $name }
machine-copy-id = Copy machine ID
machine-agent-settings = Agent settings
machine-away-agents = { $count ->
    [one] One agent was running
   *[other] { $count } agents were running
}
machine-away-explain = They keep going on { $name }. What they did appears here when it connects again.
machine-not-back = Not coming back?
forget-machine-explain = It and its finished agents leave every window. Its history and usage stay. If it connects again, it comes back.
forget-machine-refuse = Also refuse it if it connects again
forget-machine-refuse-hint = It would need a new join code.
forget-machines-refuse-title = Also refuse them if they connect again
forget-machines-refuse-hint = Each would need a new join code.
forget-machine-confirm-one = Forget { $name }

# ADR-0066: the model and effort a kind takes, in the start window.
setting-default = The agent's own default
setting-default-is = The agent's own default ({ $value })
start-settings-summary = { $kind } will start with { $settings }.
start-settings-unsupported = This machine is too old to start an agent with a model or effort. Update it, or choose the agent's own default.
start-setting-refused = This machine does not take that model or effort for this agent. Choose another, or the agent's own default.

# Home as a fleet console (docs/ux/fleet-console.md): what needs you, what works, what finished, what failed, which machines are online.
fleet-panel = Fleet
fleet-summary-label = The fleet at a glance
fleet-filter-hint = Show only
fleet-failed = Failed
fleet-machines =
    { $total ->
        [one] { $online } of { $total } machine online
       *[other] { $online } of { $total } machines online
    }
fleet-stat-machines =
    { $total ->
        [0] Machines online
        [one] of { $total } machine online
       *[other] of { $total } machines online
    }
fleet-filter-active = Showing only “{ $name }”.
fleet-filter-clear = Show everything
fleet-filter-none = Nothing to show here.
fleet-attention-count = { $count ->
    [one] 1 thing needs you
   *[other] { $count } things need you
}
fleet-asked = Asked { $when }
fleet-waiting-in-terminal = Waiting for you in its terminal
fleet-allow-for = Allow: { $name }
fleet-deny-for = Deny: { $name }
fleet-review = Review and decide
fleet-review-for = Review and decide: { $name }
fleet-open-for = Open { $name }
fleet-group-by = Group by
fleet-group-machine = Machine
fleet-group-project = Project
fleet-no-project = No project
fleet-group-kind = Agent
fleet-group-state = State
fleet-no-kind = Unknown agent
fleet-same-project =
    { $count ->
        [one] Same project · { $count } machine
       *[other] Same project · { $count } machines
    }
fleet-same-project-on = Open on { $machines }
fleet-project-changing =
    { $count ->
        [one] Changes on { $count } machine in this project.
       *[other] Agents on { $count } machines have changes in this project. Check that they are not editing the same files.
    }
fleet-offline-since = Offline. Last seen { $when }.
fleet-offline-note = Agents keep running there; what they did shows up here when it reconnects.
fleet-no-agents-here = No agents here yet.
fleet-start-on = Start an agent on { $machine }
fleet-tile-asks = Asks: { $what }
fleet-tile-stopped = Stopped with its machine
fleet-tile-state-for = { $state } for { $time }
fleet-tile-ended = Ended { $when }
fleet-empty-title = Connect your first machine
fleet-empty-text = A machine runs your agents and appears here as soon as it joins. Get a join code on the machine where Agora started, then run the second command on the machine you want to add.
fleet-empty-step-code = On the machine where Agora started
fleet-empty-step-join = On the machine to add
fleet-empty-ca = ca.pem is the file ~/.agora/hub/ca.pem on that machine.
fleet-empty-machines = See your machines
fleet-guest-title = No project is yours yet
fleet-guest-text = You see only the projects you were given access to. Ask the owner to add you to one; it appears here as soon as they do.
fleet-firstrun-title = Your machines are ready. Start the first agent.
fleet-firstrun-text = Pick a machine and a folder. The agent shows up here and tells you when it needs you.
fleet-firstrun-cli = Or from a terminal on that machine
fleet-firstrun-offline = Every machine is offline right now. Start Agora on one of them and it appears here.
# ADR-0106: a viewer's window, when nothing runs yet.
fleet-watch-title = Nothing is running yet
fleet-watch-text = You can watch here. Agents that members or the owner start show up here, with what they need.
fleet-offline-title = Cannot reach your Agora
fleet-offline-text = Your agents keep running on their machines and keep track of what they do. They show up here again as soon as the connection is back.
fleet-offline-stale = The connection to Agora is down. What you see is the last this window knew and may be out of date. Agents keep running on their machines.
fleet-loading = Loading your fleet…
fleet-exit-code = Exit code { $code }
fleet-attention-more = Show { $count } more
fleet-attention-less = Show fewer
fleet-stale-no-answer = Cannot answer while the connection is down.
fleet-same-ask = { $count } { $kind } agents ask for the same thing on { $machine }
fleet-review-each = Review each
fleet-collapse = Show as one
fleet-same-ask-any = { $count } agents ask for the same thing on { $machine }
attention-ask-review = This needs to be read in full before you answer. Review it to decide.
attention-skip = { $name } was asked to act without asking you. Review shows the whole request before you answer.
attention-start-read = Review shows its folder, prompt and settings in full before you answer.
attention-pair = Compare its number with the one on its screen in the full card before you approve.
attention-later = Later
attention-review = Review
attention-more = { $count ->
    [one] 1 more needs you after this
   *[other] { $count } more need you after this
}
attention-banner = { $count ->
    [one] 1 thing needs you
   *[other] { $count } things need you
}
attention-settings-title = Needs you
attention-mode-modal = A card over the window
attention-mode-banner = A small banner only
attention-mode-off = Nothing extra
attention-mode-hint = A card opens only while this window is focused and you are not typing; otherwise a banner shows it. It never answers for you.
attention-notify-title = Notifications in the background
attention-notify-on = Notifications are on. They only say that an agent needs you.
attention-notify-names = Include the agent and machine name
attention-notify-blocked = This browser blocks notifications for this page. Allow them in the browser's site settings to turn them on.
attention-notify-unsupported = This browser cannot show notifications.
notify-title = Needs you
notify-body = { $count ->
    [one] An agent needs you
   *[other] { $count } agents need you
}
notify-body-named = { $name } needs you
notify-body-named-on = { $name } on { $machine } needs you
attention-sent = Answer sent. If this card stays, open it with Review.
# A request card in the side panel or a pane, after its answer went (it has no Review button: the card is the request).
approval-sent = Answer sent.
# ADR-0106: where a request would have its answers, in a viewer's window (the hub refuses a viewer's answer).
approval-watching = You can watch here. A member or the owner answers requests.
attention-notify-names-warn = These names come from the agents and machines themselves, and a notification may show them on a locked screen. A name that is not plain text is left out.

# Usage (no money: tokens, context and time)
ide-tab-cost = Usage
ide-tab-usage = Usage
# The Usage panel's three totals (stat tiles: a big number over this label).
usage-stat-tokens = { $count ->
    [one] Token used
   *[other] Tokens used
}
usage-stat-agents = { $count ->
    [one] Agent
   *[other] Agents
}
usage-agents = { $count ->
    [one] { $count } agent
   *[other] { $count } agents
}
usage-by-machine = By machine
usage-by-project = By project
usage-context = Context { $percent }%
usage-limits-title = Plan limits
usage-limits-empty = Plan limits appear here when the agent reports them.

# The conversation of a chat agent
chat-jump = Jump to latest
chat-jump-new = { $count ->
    [one] Jump to latest, 1 new
   *[other] Jump to latest, { $count } new
}
chat-earlier-items = Show earlier ({ $count })
chat-you = You
chat-agent = Agent
chat-copy-reply = Copy reply
chat-copy-message = Copy message
chat-act-read = { $count ->
    [one] Read a file
   *[other] Read { $count } files
}
chat-act-edit = { $count ->
    [one] Edited a file
   *[other] Edited { $count } files
}
chat-act-run = { $count ->
    [one] Ran a command
   *[other] Ran { $count } commands
}
chat-act-search = { $count ->
    [one] Searched once
   *[other] Searched { $count } times
}
chat-act-fetch = { $count ->
    [one] Fetched a page
   *[other] Fetched { $count } pages
}
chat-act-delete = { $count ->
    [one] Deleted a file
   *[other] Deleted { $count } files
}
chat-act-move = { $count ->
    [one] Moved a file
   *[other] Moved { $count } files
}
chat-act-think = Thought
chat-act-other = { $count ->
    [one] 1 other step
   *[other] { $count } other steps
}
chat-steps-count = { $count ->
    [one] 1 step
   *[other] { $count } steps
}
chat-steps-last = Last: { $step }
chat-steps-open = Show the steps
chat-files-title = { $count ->
    [one] 1 file changed
   *[other] { $count } files changed
}
chat-files-more = And { $count } more
chat-plan-label = Plan
chat-error-label = Something went wrong
prompt-hint = Enter sends, Shift+Enter starts a new line.
# ADR-0106: in place of the box to type in, in a viewer's window (the hub refuses a viewer's request).
prompt-watching = You can watch here. A member or the owner talks to this agent.
session-started-with = { $name }: { $value }
session-started-hint = Chosen when this agent was started. The agent does not report it, so it may have changed since.
empty-changes-title = All clean
empty-changes-hint = Files an agent changes and has not committed appear here.
empty-messages-title = No messages yet
empty-messages-hint = When agents write to each other, you can follow it here.
# The team chat (the right panel): "codex · teal  to claude · plum  2 min ago", the text cut to two lines; a row opens the whole text.
team-chat-to = to { $name }
team-chat-show-all = Show all
team-chat-close = Close
empty-board-title = No shared tasks yet
empty-board-hint = Tasks and files that agents share in a project appear here.
empty-usage-title = No usage yet
empty-usage-hint = Tokens, context, time and plan limits appear here once an agent has worked.
loading-label = Loading
chat-code = Code
live-summary = Live summary
live-open-changes = Open the changes
usage-total-tokens = { $count ->
    [one] { $short } token used
   *[other] { $short } tokens used
}
usage-total-time = { $time } of agent time since start
usage-since-start = Time since start
chat-steps-needs-you = Waiting for you
chat-steps-failed = Ended with an error
prompt-closed = This agent has ended, so it cannot take a request.
chat-thought-for = Thought for { $time }
chat-plan-progress = { $done } of { $total } done
chat-review-changes = Review changes
chat-diff-show-all = Show all { $count } lines
chat-state-since = { $state } · { $time }

# Tool panes: Terminal, Changes, Files and File manager as panes of the workspace (ADR-0068).
layout-tool-of = { $what } · { $name }
layout-panel-manager = File manager
palette-open-terminal = Open the terminal beside
palette-open-changes = Open Changes beside
palette-open-files = Open Files beside
palette-open-manager = Open the File manager beside
palette-new-terminal = Start a new terminal
pane-new-terminal = New terminal
pane-open-beside = Open beside
pane-open-beside-named = Open the changes of { $name } beside
pane-open-changes = Open the changes in a pane
pane-switch-folder = Show another folder
pane-choose-folder = Choose a folder for { $name }
pane-choose-terminal = Choose a terminal to show
pane-no-folders-title = No folders yet
pane-no-folders-hint = A folder appears here once an agent has started in it on one of your machines.
pane-no-terminals-title = No terminals yet
pane-no-terminals-hint = Start an agent or a terminal on a machine and it can be shown here.
pane-offline-title = { $name } is not available here
pane-offline-hint = This window is not connected to your machines.
pane-no-folder-title = No folder for this agent
pane-no-folder-hint = The machine has not reported a folder for it yet. Try again in a moment.
start-terminal-missing = This machine offers no plain terminal (the “{ $kind }” kind). Add a “{ $kind }” manifest to its agents folder, or choose an agent.
fm-col-name = Name
fm-col-kind = Kind
fm-col-size = Size
fm-up = Parent folder
fm-actions = File actions
fm-copy-path = Copy path
fm-copy-path-hint = Copy the path, relative to the folder
fm-copied = Path copied
fm-copy-failed = Could not copy
fm-download = Download
fm-download-none = Choose a file to download it
fm-download-failed = Could not download this file
fm-terminal = Agent terminal
fm-terminal-root = Open the agent's terminal beside
fm-terminal-subfolder = A terminal opens in the agent's own folder only. This window cannot start one in a sub-folder.
fm-terminal-closed = This agent has ended, so it has no terminal.
fm-terminal-chat = This agent is a chat; it has no terminal.
fm-terminal-none = There is no terminal to open here.
fm-rename = Rename
fm-move = Move
fm-delete = Delete
fm-upload = Upload
fm-write-why = Changing files needs your approval on the machine, and this window cannot ask for it yet.
fm-folders = Folders
fm-list = Files in this folder
fm-empty = This folder is empty.
fm-count = { $count ->
    [one] 1 item
   *[other] { $count } items
}
fm-preview = Preview
fm-back = Back to the list
fm-close-preview = Close the preview
fm-binary = This file is not text, so it is not shown here. You can download it.
fm-kind-folder = Folder
fm-kind-code = Code
fm-kind-text = Text
fm-kind-image = Image
fm-kind-archive = Archive
fm-kind-config = Settings
fm-kind-file = File
fm-kind-private = Private file
tool-status-pending = Waiting to start
tool-status-running = Running
tool-status-done = Done
tool-status-failed = Failed
# #908: a call that was still running or waiting when its turn ended (Stop turn, or the agent moved on): it will not finish.
tool-status-stopped = Stopped
tool-output-cut = The output was shortened.
chat-thinking = Thinking…
chat-steps-call-failed = { $count ->
    [one] A step failed
   *[other] { $count } steps failed
}
permission-pending = Asked for permission
permission-approved = Permission given
permission-denied = Permission refused
permission-approved-always = Permission given for this session
permission-approved-always-kept = Permission given; the agent remembers it
permission-cancelled = Not answered
permission-show = Show the request
chat-queued-mark = Waiting
chat-not-sent-mark = Not sent
session-effort = Effort
session-on = On
session-off = Off
session-more = More settings
prompt-queued = Waiting for this turn to end.
prompt-queue-full = Too many messages are waiting. Wait for the agent, or stop its turn.
prompt-placeholder-commands = What should it do? Type / for commands.
queue-count = { $count ->
    [one] 1 queued
   *[other] { $count } queued
}
queue-note = Sent when this turn ends
queue-drop = Drop the queue
queue-drop-title = Drop the waiting messages?
queue-drop-body = { $count ->
    [one] This also stops the agent's current turn, and the waiting message is not sent. The agent itself keeps running.
   *[other] This also stops the agent's current turn, and the { $count } waiting messages are not sent. The agent itself keeps running.
}
queue-drop-yes = Drop and stop the turn
queue-drop-no = Keep them
slash-menu = Commands the agent offers
turn-stop = Stop turn
turn-stop-hint = Ends this turn only. The agent keeps running and you can send it another message.
turn-stop-failed = The turn did not stop.
chat-trimmed = { $count ->
    [one] 1 earlier entry is not loaded.
   *[other] { $count } earlier entries are not loaded.
}
refusal-unsupported = This agent's machine does not support that yet.
refusal-unconfirmed = That choice lets the agent act without asking, and this machine cannot ask for your approval yet. Update it, or start the agent that way instead.
refusal-forbidden = This machine's settings do not let the agent act without asking.
refusal-unknown-option = This setting or value cannot be offered for your approval: the agent no longer offers it, or its name cannot be shown safely.
refusal-limit = This machine has too many requests waiting. Answer or wait for one, then try again.
refusal-skip-agent-limit = This agent already has too many requests waiting for your approval. Answer the waiting one, or wait a little, then try again.
refusal-agent-gone = That agent is no longer running.
refusal-task-taken = This task was already taken by an agent, or it is done.
refusal-task-blocked = This task waits for another task to be done first.
refusal-task-gone = This task is no longer on the board.
give-refused-other-project = That agent does not belong to this task's project.
refusal-board-unavailable = This project's board cannot be reached right now. Try again in a moment.

# ADR-0091: plan limits. The numbers are what the tool itself reported; nothing is worked out. No money.
quota-window-five-hour = Five hours
quota-window-seven-day = Seven days
quota-window-minutes = { $minutes ->
    [one] 1 minute
   *[other] { $minutes } minutes
}
quota-percent = { $percent } %
quota-chip = Limit { $percent } %
quota-chip-short = { $percent } %
quota-chip-title = { $kind }, { $what }: { $percent } % of the plan limit used.
quota-resets = Resets in { $time } ({ $when })
quota-reset-unknown = Reset time not reported
quota-observed = Observed { $when }
quota-stale = Out of date
quota-stale-note = Older than the window itself, so it may no longer be true
quota-reset = Reset
quota-reset-note = The window reset ({ $when }); no new reading yet
quota-unknown-line = No limit information from this tool: { $kinds }
quota-span-days = { $days } d { $hours } h
quota-span-hours = { $hours } h { $minutes } min
quota-span-minutes = { $minutes } min
quota-hint = { $kind }: the { $what } limit is almost used ({ $percent } %). You can still start.

# Title bar icons and the Settings page (sections of "This device" and "My machines").
settings-link = Settings
machines-link = Machines
header-links = Settings and machines
settings-title = Settings
settings-sections = Settings sections
settings-group-device = This device
settings-group-machines = My machines
settings-device = Name and fingerprint
settings-passkeys = Passkey
settings-machines = Machines
settings-machines-manage = Manage machines

# Closing a running agent (the agent menu, the command palette, /exit and /quit in a chat agent's message box)
close-agent = Close agent
agent-actions = Actions for { $name }
palette-close-agent = Close agent: { $name }
palette-give-task = Give to an agent: { $title }
close-title = Close { $name }?
close-body = Ends this agent. Its conversation stays in the closed list until you remove it.
close-working = It is working now: the current turn will be lost.
close-keep = Keep it running
close-failed-prefix = It was not closed: { $reason }
close-failed = something went wrong. Try again.
close-failed-forbidden = you are not allowed to close agents on this machine
close-failed-policy = this machine does not allow it
close-failed-grant = this machine needs your permission first. Allow it in the dialog and it closes
close-failed-trust = this machine does not trust this browser yet
close-failed-offline = that machine is offline
close-failed-timeout = that machine did not answer in time
close-failed-gone = the machine no longer knows this agent
close-failed-unsupported = that machine cannot do it yet. Update it
close-failed-hub = the hub could not be reached

# Settings → Agents: the agents each machine can start, which are installed, and what each takes when started.
settings-agents = Agents
settings-subtitle = Kept in this browser unless it says otherwise.
device-desc = Machines accept answers only from a device they trust.
device-fingerprint-hint = Compare it when a machine asks.
device-trust-on = { $count ->
    [one] Trusted on 1 machine
   *[other] Trusted on { $count } machines
}
device-trust-none = Not trusted yet
device-others-desc = Browsers signed in to your Agora.
device-remove-named = Remove { $name }
device-remove-title = Remove { $name }?
device-remove-body = Your machines will no longer accept answers from it. This cannot be undone here.
device-remove-keep = Keep it
device-remove-confirm = Remove
when-day-time = { $day }, { $time }
attention-settings-desc = How this window gets your attention when an agent waits. Kept in this browser only.
attention-mode-label = When an agent needs you
attention-mode-short-modal = Card
attention-mode-short-banner = Banner
attention-mode-short-off = Nothing extra
attention-notify-hint = They only say that an agent needs you, unless you add names.
password-desc = Asked from browsers on other machines, with their code. This machine's own browser is never asked.
password-desc-set = Asked from browsers on other machines. Changing it signs out every other window.
password-state-set = Set
password-state-unset = Not set
settings-machines-desc = Every machine connected to your Agora, and whether it trusts this browser.
settings-machines-online = { $online } of { $total } online
agents-desc = What each machine can start. Choices are made when you start an agent; open a card to see what it takes.
agents-machine = Machine
agents-runs-short-terminal = Terminal
agents-runs-short-chat = Chat
agents-runs-short-adapter = Chat
agents-kind = Id
agents-runs = Runs
agents-offers-skip = Offers "Don't ask": it can start so that it skips permission questions. Never chosen for you.
agents-loading = Asking this machine…
agents-none-known = This machine is offline and has not told this browser which agents it has.
agents-empty = This machine lists no agents.
agents-stale = Offline · as of { $when }
agents-not-installed = Not installed
agents-runs-terminal = Runs in a terminal
agents-runs-chat = Chat
agents-runs-adapter = Chat through an adapter
agents-program-missing = This machine cannot find { $program } in its PATH.
agents-setting-any = any value the agent accepts
agents-setting-default = default { $value }
agents-options = Options
agents-option-skips = skips permission questions
agents-group-here = On this machine
agents-group-add = Add an agent
agents-group-add-desc = Not installed on this machine. Install the program there and it can start; nothing here runs or downloads anything.
agents-untested = Untested
agents-untested-desc = From the ACP registry. Nobody has tried it in Agora yet: it may not start, or may lack features.
agents-runs-fetch = Starting it runs { $program }, which downloads this exact version the first time.
agents-version = Version
agents-license = License
agents-repository = Repository
agents-website = Website
agents-source = Listed by
agents-source-registry = ACP registry

# The widget kit in the component gallery (visual-language.md §3). Sample text for the gallery only.
kit-title = Widget kit
kit-buttons = Buttons
kit-states = States and tags
kit-avatars = Agent avatars
kit-cards = Cards, rows and settings
kit-controls = Controls
kit-counts = Counts
kit-notices = Notices and requests
kit-empty = Empty states
kit-copy = A command to copy
kit-rtl = Right to left
kit-sample-forget = Forget
kit-sample-shared = Shared
kit-sample-notify = Notifications in the background
kit-sample-notify-hint = Names show only if you choose so.
kit-sample-ask = { $name } wants to run a command
kit-sample-offline = This machine is offline
kit-sample-offline-line = Its agents keep running and report back when it returns.
kit-sample-joined = The machine joined. It appears in My machines.

# The redesign's entry screens (ADR-0102): the sign-in card, the empty Home and the command palette.
signin-or = or
copy-selected = Copying did not work. The text is selected: copy it with your keyboard.
fleet-add-machine = Add a machine
palette-empty-hint = Try another word: an agent, a machine or an action.
palette-group-go = Go to
palette-group-actions = Actions
palette-group-agents = Agents
palette-group-places = Machines and projects
palette-group-tasks = Tasks
palette-group-appearance = Appearance
palette-key-move = Move
palette-key-open = Open
palette-key-close = Close

# Plugins (ADR-0114 build step 2, docs/ux/plugins.md): sandboxed panels loaded from a local file for development, the install screen
# with what a plugin asks to see, the marker beside a plugin panel's name, and safe mode.
palette-plugins = Plugins
plugin-panel = Plugin
plugin-marker = Plugin
plugin-development = development
plugin-frame-title = { $name } (plugin)
plugins-title = Plugins
plugins-intro = Plugins you load from a file run in this browser only, each in its own sandboxed pane. They see only what you allow, and never your keys, the network or what needs you. Show one in a pane from the pane's Show… menu.
plugins-safe = Safe mode: no plugin runs and the built-in theme is on. Leave safe mode to use your plugins again.
plugins-done = Done
plugin-load = Load from a file…
plugin-version-by = { $version } by { $author }
plugin-licence = Licence: { $licence }
plugin-held = This version asks to see more than you allowed. Its panes stay closed until you look again.
plugin-sees = { $count ->
    [0] It sees nothing of yours.
    [one] It sees one thing you allowed.
   *[other] It sees { $count } things you allowed.
}
plugin-review = Look again
plugin-change = What it can see
plugin-remove = Remove
plugin-added = { $name } was added. Show it in a pane from the pane's Show… menu.
plugin-added-visit = { $name } was added for this visit only: this browser could not keep it.
plugin-grant-title = Add { $name }?
plugin-grant-never = It draws only inside its own pane. It cannot reach the network, your keys, your terminals or what needs you, and it cannot do anything for you.
plugin-grant-asks = It asks to see:
plugin-grant-nothing = It asks to see nothing of yours.
plugin-cancel = Cancel
plugin-add = Add plugin
plugin-save = Save
plugin-cap-agents-read = The names, kinds and states of your agents, with their machines and projects
plugin-cap-usage-read = How many tokens, how much context and how much time your agents have used
plugin-cap-tasks-read = The shared tasks of your projects
plugin-cap-changes-read = The changed files, and what changed in them, in the folder of the agent its pane shows
plugin-cap-chat-read = The conversation of the agent its pane shows, with secrets hidden
plugin-cap-theme-read = The colours of your theme, so it can match them
plugin-cap-pane-context = Which agent its pane shows, or what the pane follows
plugin-error-too-large = This file is larger than 2 MB, the most a plugin can be.
plugin-error-format = This file is not a plugin package.
plugin-error-newer-format = This plugin package was made for a newer version.
plugin-error-too-many = This package holds more files than a plugin may have.
plugin-error-path = This package names a file with a folder or path in it ({ $detail }), so nothing of it was loaded.
plugin-error-link = This package holds a link or something else that is not a file ({ $detail }), so nothing of it was loaded.
plugin-error-type = This package holds a kind of file a plugin cannot use ({ $detail }), so nothing of it was loaded.
plugin-error-manifest = The package's description (agora-plugin.json) is missing or wrong at "{ $detail }".
plugin-error-capability = This plugin asks to see something this window does not know ({ $detail }), so it was not loaded.
plugin-error-unlisted = This package holds a file its description does not list ({ $detail }), so nothing of it was loaded.
plugin-error-missing = A file the package's description lists is missing ({ $detail }), so nothing of it was loaded.
plugin-error-hash = A file in this package is not the one its description lists ({ $detail }), so nothing of it was loaded.
plugin-error-newer = This plugin needs version { $detail } or newer.
plugin-error-full = Four plugins are loaded already. Remove one first.
safe-mode-on = Safe mode: no plugins, the built-in theme.
safe-mode-leave = Leave safe mode
plugin-frame-use = { $name } (plugin): click or press Enter to use it
safe-mode-open = Open in safe mode
palette-safe-mode = Open in safe mode (no plugins, the built-in theme)
plugin-stopped = { $name } was stopped: it sent the window too many messages.
plugin-restart = Start it again

# Sounds when an agent finishes or needs you (Settings > This device > Sounds; the agent's menu), and the finished mark that lasts until seen
sounds-title = Sounds
sounds-desc = A short sound when an agent finishes or needs you. Kept in this browser only.
sounds-label = Play sounds
sounds-hint = Off until you turn them on. The window always shows the same thing on screen too.
sounds-on-hint = Sounds play while this window is in the background, never for the agent you are looking at.
sounds-unsupported = This browser cannot play sounds.
sounds-safe-mode = No sounds in safe mode.
sounds-volume = Volume
sounds-volume-low = Quiet
sounds-volume-medium = Medium
sounds-volume-high = Loud
sounds-finished = When an agent finishes
sounds-finished-hint = Two soft rising notes, when an agent ends its turn.
sounds-needs-you = When an agent needs you
sounds-needs-you-hint = Two short notes, when an agent waits for you or asks for your OK.
sounds-try = Play
sounds-try-named = Play the sound: { $name }
sounds-in-front = Also while this window is in front
sounds-in-front-hint = Still never for the agent in the pane you are using.
agent-sounds-mute = Mute this agent's sounds
finished-unseen = Finished while you were away. Open it to clear this mark.
finished-unseen-short = New
palette-sounds-on = Turn sounds on
palette-sounds-off = Turn sounds off
keys-cmd-sounds = Turn sounds on or off
keys-cmd-mute-agent = Mute or unmute the focused agent's sounds
palette-agent-mute = Mute the sounds of { $name }
palette-agent-unmute = Unmute the sounds of { $name }
