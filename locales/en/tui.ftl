# The terminal client (ADR-0043). UI vocabulary: machines, "Needs you", "Finished" (docs/ux/visual-language.md).
tui-connecting = Connecting to { $hub }…
tui-lost = The connection to { $hub } was lost. Trying again…
tui-signin-failed-local = Could not sign in to this machine's hub. Run `agora` again; `agora status` shows whether it is running and where its log is.
tui-signin-failed = The hub did not accept the code. Make a new one with "agora hub login".
tui-need-hub = Tell me where your hub is: run this on the hub's machine, or pass --hub HOST:PORT --ca FILE --code CODE.
tui-quit-hint = Ctrl-C to quit
tui-header = Agora · { $online } of { $total } machines online · { $agents } agents running
tui-attention = Needs you
tui-nothing-waiting = Nothing is waiting for you.
tui-asks = { $agent } asks: { $action }
tui-asks-start = { $agent } ({ $agent_kind }) on { $node } asks to start { $kind } in { $folder }{ $worktree ->
        [true] , in its own worktree
       *[other] {""}
    } (depth { $depth }). Its first prompt:
tui-asks-start-outside = { $agent } ({ $agent_kind }) on { $node } asks to start { $kind } in { $folder }, outside its own folder{ $worktree ->
        [true] , in its own worktree
       *[other] {""}
    } (depth { $depth }). Its first prompt:
tui-asks-start-remote = { $agent } ({ $agent_kind }) on another machine, { $origin }, asks to start { $kind } here on { $node } in { $folder }{ $worktree ->
        [true] , in its own worktree
       *[other] {""}
    } (depth { $depth }). Its first prompt:
tui-start-remote-once = Answering here allows this one start. To let it start more without asking, answer from the window.
tui-started-by = started by { $agent }
tui-blocked = { $agent } is waiting for you
tui-machines = Machines
tui-online = online
tui-offline = offline
tui-last-seen = last seen { $unit ->
        [s] { $n } s
        [min] { $n } min
        [h] { $n } h
       *[d] { $n } d
    } ago
tui-no-agents = No agents yet. Start one in the web app or with "agora agent start".
tui-state-working = Working
tui-state-needs-you = Needs you
tui-state-idle = Idle
tui-state-finished = Finished
tui-state-unreachable = offline
tui-agent-on = on { $machine }
tui-agent-on-in = on { $machine } in { $folder }
tui-agent-changes = { $files } { $files ->
        [one] file
       *[other] files
    } changed · +{ $plus } −{ $minus }
tui-agent-tokens = { $input } in · { $output } out
tui-agent-cost = { $amount } { $currency } so far
tui-state-unknown = Unknown
tui-no-project = (no project)
tui-hint-answer = Press y to allow, n to deny the first request above.
tui-allowed = Allowed.
# ADR-0101: an answer the agent remembers. The session words are the ADR's, for a kind known to drop it when its session ends; no kind is
# marked so yet, so the "-kept" words are shown.
tui-allowed-always = Allowed for this session.
tui-allowed-always-kept = Allowed always; the agent remembers it and decides for how long.
tui-allow-always-not-offered = This request can only be allowed once.
tui-allow-once-not-offered = This agent offers no single answer for this request: A in the request box (Alt+A in its tab opens it) allows it for the rest of its session.
tui-allow-once-not-offered-kept = This agent offers no single answer for this request: A in the request box (Alt+A in its tab opens it) allows it always, and the agent remembers it.
tui-denied = Denied.
tui-not-trusted-at = This device is not trusted by that machine yet. On that machine, in the folder of { $binary }, run: ./agora node trust { $id } { $key }   (.\agora.exe on Windows). Or press a to ask another device of yours to approve this one (fingerprint { $fingerprint }).
tui-not-trusted = This device is not trusted by that machine yet. On that machine run: agora node trust { $id } { $key }   (if `agora` is not found there, run it in the folder of the agora program: ./agora, or .\agora.exe on Windows). Or press a to ask another device of yours to approve this one (fingerprint { $fingerprint }).
tui-device-name = Terminal on { $host }
tui-device-asked = Asked. Approve this terminal ({ $fingerprint }) on another device of yours that your machines trust: in the window, or with v in its terminal client.
tui-devices-none-waiting = No device of yours is waiting for approval. This terminal's fingerprint: { $fingerprint }
tui-device-cannot-vouch = No machine trusts this terminal yet, so its approval would count nowhere.
tui-device-card =
    { $name } asks to be trusted on your machines.
    Fingerprint: { $fingerprint } (compare it with the one that device shows)
    It will be trusted where this terminal is trusted: { $machines }
    Press y to approve it with this terminal's key, any other key to cancel.
tui-device-approved = Approved { $name }. Each machine checks the approval itself; the device shows as trusted once it has.
tui-answer-failed = Could not send your answer ({ $code }).
tui-hint-open = Tab: next agent · Enter: open it · x: stop it · s: start one (Ctrl-] comes back)
tui-opening-failed = Could not open that terminal ({ $code }).
tui-usage = { $input } tokens in · { $output } out
tui-chat-you = You
tui-chat-agent = { $agent }
tui-chat-tool = ran { $text }
tui-chat-plan = Plan
tui-chat-diff = changed { $path }
tui-chat-input = ›
tui-chat-hint = Type a request and press Enter · /mode or /model followed by a name switches · Ctrl-] goes back
# ADR-0106: in place of the hint and the line to type on, in a viewer's window (the hub refuses a viewer's request or switch).
tui-chat-watching = You can watch here. Sending requests and switching the mode or model need a member's role · Ctrl-] goes back
tui-chat-mode = mode { $mode }
tui-chat-model = model { $model }
tui-chat-context = context { $percent }%
tui-notice-turn_failed = (the turn failed, error { $rpc_code }; its provider may have refused the request) { $stderr_tail }
tui-notice-turn_stopped = (the agent stopped its turn: { $stop_reason })
tui-notice-turn_empty = (the turn ended without a word; its provider may have refused the request) { $stderr_tail }
tui-notice-agent_exited = (the agent stopped, step { $stage }, exit code { $exit_code }: { $stderr_tail })
tui-notice-agent_refused = (the agent refused to start, step { $stage }, error { $rpc_code }: { $detail })
tui-notice-no_answer = (the agent did not confirm it got this request within { $waited_ms } ms; it may still be working on it)
tui-notice-output_dropped = (the agent kept running while Agora restarted; { $bytes } bytes of its output were lost)
tui-notice-restarting = (the agent stopped unexpectedly; Agora is starting it again, attempt { $attempt })
tui-notice-gave_up = (the agent stopped unexpectedly again after { $restarts } restarts, last exit code { $exit_code }; Agora stopped starting it again)
tui-notice-gave_up-no_code = (the agent stopped unexpectedly again after { $restarts } restarts; Agora stopped starting it again)
tui-notice-history_not_saved = (this conversation could not be saved on its machine; the agent keeps working, but later turns may be missing after a restart)
tui-notice-skip_permissions_refused = (the agent was set to act without asking, which this machine does not allow; Agora stopped it)
tui-notice-other = (the agent reported { $code })
tui-option-switched = Switched.
tui-option-unknown = The agent does not offer { $id }. It offers: { $ids }
tui-option-confirm = In { $id } the agent acts without asking you. Type the same line again to switch anyway.
tui-stopped = Stopped.
tui-stop-failed = Could not stop it ({ $code }).
tui-stop-helpers = It started { $count } helpers that still run. Stop them too? Press n to keep them, any other key stops them.
tui-send-failed = Could not send it ({ $code }).
tui-agent-no-answer = The agent did not confirm it got this. Wait a moment, and send it again only if nothing happens.
tui-answer-no-answer = The agent did not take your answer in time. The request is still open: wait a moment and answer again.
tui-agent-exited = The agent stopped, so it did not get this. Start the agent again.
tui-start-pick = Start which agent on { $machine }? Press its number, any other key cancels.
tui-start-pick-page = Start which agent on { $machine }? Press its number, n for the next page ({ $page } of { $pages }), any other key cancels.
tui-start-item = { $n }  { $name }
tui-start-item-last = { $n }  { $name }  (last time)
tui-start-group-untested = Untested (from the ACP registry, not yet tried in Agora)
tui-start-none = No agent is installed on { $machine }.
tui-start-signed-only = { $name } downloads its program when it starts, so the machine starts it only when this device signs the start: press f and choose its folder.
tui-start-missing-item = { $name } (`{ $program }`)
tui-start-missing = Not found on this machine: { $names }. The machine cannot find their programs in its PATH. Install them there, or start the node from a terminal where they run.
tui-start-nowhere = No machine with a project is online.
tui-started = Started { $agent }.
tui-start-failed = Could not start it ({ $code }).
tui-start-role = Your role does not let you start agents here. Ask the owner if you need to.
tui-resume-role = Your role does not let you start this agent again. Ask the owner.
tui-cost = about { $usd } USD
tui-grant-ask = That machine wants your OK before it lets you do this. Press y to allow it for one hour, any other key to cancel.
tui-grant-ask-typing = Allow this window to type into { $agent } for one hour? That is all it allows. Press y to allow it, any other key to cancel.
tui-grant-ask-prompt = Allow this window to send messages to { $agent } for one hour? That is all it allows. Press y to allow it, any other key to cancel.
tui-grant-ask-stop = Allow this window to stop { $agent } for one hour? That is all it allows. Press y to allow it, any other key to cancel.
tui-grant-ask-mode = Allow this window to change the mode of { $agent } for one hour? That is all it allows. Press y to allow it, any other key to cancel.
tui-grant-ask-model = Allow this window to change the model of { $agent } for one hour? That is all it allows. Press y to allow it, any other key to cancel.
tui-granted = Allowed for one hour.
tui-split-answer = y allows, n denies the first request
tui-tab-answer = Alt+Y allows it, Alt+N denies it
tui-chat-answer = Alt+Y allows it, Alt+N denies it · what you type stays in your draft
tui-tab-answer-always = Alt+Y allows it, Alt+A allow always for this session (asks first), Alt+N denies it
tui-tab-answer-always-kept = Alt+Y allows it, Alt+A allow always (asks first; the agent remembers it), Alt+N denies it
tui-tab-answer-always-only = Alt+A allow for the rest of its session (asks first; it offers no single answer), Alt+N denies it
tui-tab-answer-always-only-kept = Alt+A allow always (asks first; it offers no single answer), Alt+N denies it
tui-chat-answer-always = Alt+Y allows it, Alt+A allow always for this session (asks first), Alt+N denies it · what you type stays in your draft
tui-chat-answer-always-kept = Alt+Y allows it, Alt+A allow always (asks first; the agent remembers it), Alt+N denies it · what you type stays in your draft
tui-chat-answer-always-only = Alt+A allow for the rest of its session (asks first; it offers no single answer), Alt+N denies it · what you type stays in your draft
tui-chat-answer-always-only-kept = Alt+A allow always (asks first; it offers no single answer), Alt+N denies it · what you type stays in your draft
tui-chat-answer-first = Answer the request first (Alt+Y allows, Alt+N denies); your draft is kept.
tui-view-asked = ! A request waits for you: q goes back to answer it
tui-request-answer = y allows it · n denies it · any other key leaves it waiting
tui-request-answer-always = y allows it · A allows it always for this session · n denies it · any other key leaves it waiting
tui-request-answer-always-kept = y allows it · A allows it always (the agent remembers it) · n denies it · any other key leaves it waiting
tui-request-answer-always-only = A allows it for the rest of its session (it offers no single answer) · n denies it · any other key leaves it waiting
tui-request-answer-always-only-kept = A allows it always, the agent remembers it (it offers no single answer) · n denies it · any other key leaves it waiting
tui-always-label = In its own words: “{ $label }”
tui-always-warning = If you allow it always, the agent keeps the answer until its session ends.
tui-always-warning-kept = If you allow it always, the agent decides what that covers and how long it keeps the answer, possibly longer than this session.
tui-always-too-small = The window is too small to show the whole request and what “always” means, so nothing was sent. Make it larger, then answer again.
tui-note-copy = c copies the command · any key closes this
tui-note-close = Any key closes this.
tui-copied = Copied the command to the clipboard.
tui-copied-text = Copied { $count ->
        [one] one character
       *[other] { $count } characters
    } to the clipboard.
tui-split-asks-start = asks to start { $kind }
tui-split-card-more = … { $count } more lines: press Enter to read all
tui-split-read-first = Read the whole request before allowing it: press Enter.
tui-reader-keys = j / k or the arrows scroll · space goes a page down · q goes back
tui-split-pick = Pick an agent on the left
tui-split-no-screen = Opening its terminal…
tui-split-status = { $online }/{ $total } { $total ->
        [one] machine
       *[other] machines
    } · { $agents } { $agents ->
        [one] agent
       *[other] agents
    } · { $waiting } need you
tui-split-keys = ↑↓ choose · Enter open · s start · f folder · x stop · u resume · d remove · a ask · v devices · q quit
# ADR-0106: a viewer's list shows only the keys that read.
tui-split-keys-viewer = ↑↓ choose · Enter open · a ask · q quit
tui-split-chat = No terminal: Enter opens its conversation
tui-folder-title = Start an agent in a folder on { $machine }
tui-folder-keys = ↑↓ choose · Enter open · ⌫ up · Space start here · m machine · Esc cancel
tui-folder-projects = Your projects on this machine
tui-folder-projects-keys = ↑↓ choose · Enter open · m machine · Esc cancel
tui-folder-git = git
tui-folder-empty = No folders here.
tui-folder-more = … more folders than shown (the first { $n })
tui-folder-unsupported = That machine cannot list its folders yet (update it).
tui-folder-failed = Could not open that folder ({ $code }).
tui-folder-role = Your role does not let you choose this folder. Ask the owner.
# ADR-0106: a key that changes something, pressed in a viewer's window. Nothing was sent.
tui-viewer-watching = You can watch here. Starting, stopping and answering requests need a member's role. Ask the owner.
tui-folder-identity = This folder's project changed since an agent last ran here. Press y to start anyway, any other key to cancel.
tui-forgot = Removed from the list.
tui-forget-running = It is still running: stop it first (x).
tui-forget-failed = Could not remove it: { $reason }
tui-forget-disconnected = The connection was lost. Wait for it to reconnect.
tui-forget-timeout = No answer arrived in time. Try again.
tui-forget-action = removing the agent
tui-interrupted = stopped with its machine
tui-interrupted-resume = stopped with its machine · u resume · U start again
tui-interrupted-again = stopped with its machine · U start again
tui-resumed = { $agent } is running again.
tui-resume-not-interrupted = Only an agent that stopped with its machine can be resumed (u) or started again (U).
tui-resume-failed = It did not start: { $reason }
tui-resume-cannot = It cannot continue its own conversation. U starts it again in its folder (a new conversation).
tui-resume-confirm = Press u again within 5 seconds to resume { $agent }.
tui-again-confirm = Press U again within 5 seconds to start { $agent } again in its folder (a new conversation).
tui-interrupted-running = still running from before its machine's restart · x stops it
tui-resume-still-running = Its process from before the restart is still running. Stop it with x first, then resume it.

# ADR-0059: launch options (manifest `[[launch.option]]` labels)
option-skip-permissions = Don't ask
setting-model = Model
setting-effort = Effort
tui-start-setting = { $name }: { $value }
tui-start-how-it-acts = How it acts
tui-start-does-not-ask = If you allow it, this helper runs commands and changes files without asking you first, for as long as it runs.
tui-start-approval-confirm-skip = Let the helper act without asking? With "{ $name }" it runs commands and changes files without asking you first, for as long as it runs. Press s to start it without asking; any other key, y included, keeps asking.
option-ask = Ask before acting
tui-start-acts = How should it act? Press its number; Enter or any other key asks before acting.
tui-start-confirm-skip = Let the agent act without asking? With "{ $name }" it runs commands and changes files without asking you first, for as long as it runs. Press y to start without asking; any other key keeps asking.
tui-badge-no-ask = does not ask
tui-badge-asks-less = asks less
# ADR-0094: a person runs an agent by hand in this Shell.
tui-runs-inside = { $name } in Shell

## A chat agent's tool call in words (as the web window says it)
tui-tool-read = Read
tui-tool-edit = Edit
tui-tool-run = Command
tui-tool-search = Search
tui-tool-fetch = Web page
tui-tool-delete = Delete
tui-tool-move = Move
tui-tool-think = Thinking
tui-tool-other = Step

## The app shell (ADR-0063 stage 2)
tui-app-keys-tree = Ctrl K commands · Enter open · N next that needs you · y / n answer · s start · f folder · x stop · u resume · r changes · o files · b board · d remove · a ask · v devices · Tab next part · i details · q quit
tui-app-keys-tree-viewer = Ctrl K commands · Enter open · N next that needs you · r changes · o files · b board · a ask · Tab next part · i details · q quit
tui-app-keys-terminal = Ctrl-] or F12 back to the list · Your keys go to the agent · Alt+K commands · Alt+1…9 tabs · Alt+M mouse
tui-app-keys-terminal-viewer = Ctrl-] or F12 back to the list · You are watching: your keys are not sent · Alt+K commands · Alt+1…9 tabs · Alt+M mouse
tui-app-keys-terminal-waiting = It needs you: Alt+Y allows · Alt+N denies · Ctrl-] or F12 back to the list · Alt+K commands
tui-app-keys-inspector = Tab next part · Esc back to the list · Ctrl K commands
tui-app-keys-palette = Type to filter · ↑↓ choose · Enter run · Esc close
tui-app-mouse-on = mouse on (m releases it to select text)
tui-app-mouse-off = mouse off (m takes it back)
tui-app-no-agent = Choose an agent on the left and press Enter to see its terminal here.
tui-app-welcome-title = Pick up where you left off
tui-app-welcome-start = Start an agent
tui-app-welcome-running = Running now
tui-app-welcome-none = No agent is running yet.
tui-app-welcome-search = Search and commands
tui-app-welcome-next = Jump to the next agent that needs you
tui-app-tabs-waiting = { $count } waiting · Ctrl-] N
tui-app-scrolled = ↑ { $count ->
        [one] one line back
       *[other] { $count } lines back
    } · type to go back down
tui-app-self-terminal = This tab is the terminal this client runs in, so its size is left alone (sizing it would shrink this window each time).
tui-app-no-screen = This agent's terminal is opening.
tui-app-tree-empty = No agents yet.
tui-app-palette-title = Commands
tui-app-palette-none = Nothing matches.
tui-app-cmd-open = Open { $name }
tui-app-cmd-next = Go to the next agent that needs you
tui-app-cmd-allow = Allow the request that waits (y)
tui-app-cmd-allow-always = Allow the request that waits, always for this session… (asks first)
tui-app-cmd-allow-always-kept = Allow the request that waits, always… (asks first; the agent remembers it)
tui-app-cmd-deny = Deny the request that waits (n)
tui-app-cmd-start = Start an agent (s)
tui-app-cmd-folder = Start an agent in a folder (f)
tui-app-cmd-stop = Stop the selected agent (x)
tui-app-cmd-resume = Resume the selected agent (u)
tui-app-cmd-again = Start the selected agent again (U)
tui-app-cmd-remove = Remove the selected agent (d)
tui-app-cmd-review = See what the selected agent changed (r)
tui-app-cmd-files = Browse the selected agent's files (o)
tui-files-empty = This folder is empty.
tui-files-keys = ↑↓ choose · Enter open · ⌫ up · q or Esc back
tui-files-file-keys = ↑↓ scroll · Space / b a page · g / G top / end · ⌫, q or Esc back to the folder
tui-files-binary = A binary file ({ $size }): not shown.
tui-files-failed = Could not read it ({ $code }).
tui-files-private = private
tui-files-secret = Kept private: it may hold a secret
tui-review-files = { $count ->
        [one] one file changed
       *[other] { $count } files changed
    }
tui-review-none = Nothing changed yet.
tui-review-hidden = Changed, content hidden (+{ $added } -{ $removed }). Kept private: it may hold a secret
tui-review-keys = ↑↓ scroll · Space / b a page · g / G top / end · q or Esc back
tui-review-nowhere = This agent has no folder or worktree to show changes for.
tui-review-failed = Could not read its changes ({ $code }).
tui-app-cmd-ask = Ask your other devices to approve this terminal (a)
tui-app-cmd-devices = Approve a device that waits (v)
tui-app-cmd-grant = Allow typing into the agent in front for one hour, after its machine refused it (g)
tui-app-cmd-mouse = Turn the mouse on or off
tui-app-cmd-inspector = Show or hide the details
tui-app-cmd-close = Close this tab
tui-tab-closed = Closed the tab. { $agent } keeps running; x stops it.
tui-app-cmd-quit = Quit
tui-app-old-machines = { $count ->
        [one] One old machine
       *[other] { $count } old machines
    } (not seen for a day)
tui-app-cmd-cleanup = Clean up old machines
tui-cleanup-none = No old machines: every machine was seen within a day.
tui-cleanup-title = { $count ->
        [one] Clean up one old machine?
       *[other] Clean up { $count } old machines?
    }
tui-cleanup-explain = Forgotten machines and their finished agents leave every window. What they did stays in the history and the costs. A machine that connects again comes back.
tui-cleanup-keys = y forgets them · r forgets them and refuses them if they connect again · any other key keeps them
tui-cleanup-done = { $count ->
        [one] Forgot one machine.
       *[other] Forgot { $count } machines.
    }
tui-cleanup-failed = Could not forget a machine ({ $code }); { $count } forgotten before it.
# ADR-0106: "Clean up old machines" chosen by a member or a viewer. Nothing was sent.
tui-cleanup-role = Only the owner can clean up old machines. Nothing was sent.
tui-app-start-read = You read the whole request. Press y to allow it or n to deny it.

# ADR-0065: the owner's card for a machine that asks to join (the same words as the web's pair-*).
tui-pair-machine-title = A new machine wants to join
tui-pair-browser-title = A browser wants to sign in
tui-pair-name = Name
tui-pair-system = System
tui-pair-address = Address
tui-pair-number = Number
tui-pair-machine-compare = Compare with the number on the new machine. Approve only if they are the same.
tui-pair-browser-compare = Compare with the number on that browser. Approve only if they are the same.
tui-pair-expires = Runs out in { $time }
tui-pair-ran-out = This request has run out. The new machine can ask again.
tui-pair-offline = Not connected right now. You can answer once the connection is back.
tui-pair-replace-off = Replace the existing machine: no. A machine named "{ $name }" has already joined.
tui-pair-replace-on = Replace the existing machine: yes. "{ $name }" has already joined, and the old one loses its access.
tui-pair-confirm = Do the two numbers match exactly?
tui-pair-keys = a approves, d denies, Esc leaves it for later
tui-pair-keys-taken = a approves, r changes Replace, d denies, Esc leaves it for later
tui-pair-keys-confirm = y: yes, they match. Any other key goes back.
tui-pair-asked = ! A machine wants to join: press p to read it
tui-pair-asked-more = ! { $count } machines want to join: press p to read the first
tui-pair-none = No machine is asking to join.
tui-pair-answer-sent = Answer sent.
tui-pair-denied = Denied. Nothing was given to that machine.
tui-pair-gone = That request has already ended.
tui-pair-failed = The answer could not be sent ({ $code }).
tui-pair-result-approved = { $name } was approved and is joining.
tui-pair-result-denied = { $name } was denied.
tui-pair-result-expired = The request from { $name } ran out.
tui-pair-result-closed = The request from { $name } was cancelled.
tui-app-cmd-pair = Read a machine's request to join (p)
tui-link-confirm = Link: { $url } (Ctrl+click it again to open)
tui-link-opened = Opened { $url }
tui-link-failed = Could not open { $url }. Copy it from the terminal instead.
tui-typing-needs-ok = Ctrl-] then g: allow typing into { $agent } for one hour. It needs your OK first.
# ADR-0095: a project's board and "Give to an agent" in the terminal client.
tui-app-cmd-board = Open the board of the selected agent's project (b)
tui-board-no-project = The selected agent has no project, so there is no board to show.
tui-board-title = Board: { $project }
tui-board-empty = No tasks yet. Tasks that agents add for this project appear here.
tui-board-col-backlog = Backlog ({ $count })
tui-board-col-doing = Doing ({ $count })
tui-board-col-done = Done ({ $count })
tui-board-col-backlog-empty = Nothing waiting.
tui-board-col-doing-empty = Nothing in progress.
tui-board-col-done-empty = Nothing finished yet.
tui-board-untitled = Untitled task
tui-board-more = and { $count } more
tui-board-keys = ↑↓ choose · Enter or g give a Backlog task to an agent · q or Esc back
tui-board-keys-viewer = ↑↓ choose · q or Esc back
tui-board-backlog-only = Only a task in Backlog can be given to an agent.
tui-machine-name-id = { $name } · { $id }
tui-give-title = Give to an agent
tui-give-pick-hint = Running agents of { $project }. Helpers are listed under the agent that started them.
tui-give-none = No agent of { $project } is running on a connected machine that can take tasks. Start one first.
tui-give-pick-keys = ↑↓ choose · Enter choose this agent · Esc back
tui-give-none-keys = Esc back
tui-give-row-detail = { $kind } on { $machine }
tui-give-row-helper = Helper · { $kind } on { $machine }
tui-give-confirm-title = Give this task to { $agent }?
tui-give-field-task = Task: { $value }
tui-give-field-agent = Agent: { $value }
tui-give-field-kind = Kind: { $value }
tui-give-field-machine = Machine: { $value }
tui-give-field-project = Project: { $value }
tui-give-confirm-no-ask = { $agent } acts without asking: it works on this task without asking you before it changes files or runs commands.
tui-give-confirm-body = The agent gets the task as a message. If it is in the middle of a turn, the task waits until that turn ends. The agent keeps its own settings, and anything that needs your OK still asks you.
tui-give-confirm-keys = Press g to give the task, n or Esc to go back.
tui-give-sent = Sent to { $agent }.
tui-give-queued = Sent to { $agent }. It waits until the agent's current turn ends.
tui-give-failed = The task was not sent ({ $code }). Try again.
tui-give-refused-paste = This agent's terminal program cannot take a pasted task. Choose another agent, or type the task into it yourself.
tui-give-refused-waiting = This agent already has a task waiting for its turn to end. Wait for it, or choose another agent.
tui-give-refused-limited = You gave { $limit } tasks to agents in the last minute. Wait a moment, then try again.
tui-give-refused-grant = This machine still needs your permission first. Nothing was sent.
tui-give-refused-offline = That machine is not connected right now.
tui-give-refused-forbidden = You are not allowed to give tasks to this agent.
tui-give-refused-other-project = That agent does not belong to this task's project.
tui-give-target-gone = That agent stopped, was closed or left this project. Nothing was sent; choose another agent.
tui-give-machine-unsupported = That agent's machine cannot take tasks yet; it may need an update. Nothing was sent.
tui-give-grant-declined = The machine's permission was not given, so nothing was sent. You can try again.
tui-grant-ask-give = Allow this window to give board tasks to { $agent } for one hour? That is all it allows. Press y to allow it, any other key to cancel.
tui-give-too-small = This terminal is too small to show the whole confirmation, so the task cannot be given here. Make the window larger, or press n or Esc to go back.

# ADR-0113 step 3: themes in the terminal client. The theme list (t, or "Theme…" in the commands) and its problems; the problems use
# the web window's wording (web.ftl theme-error-*), so both clients say the same thing about the same file.
tui-app-cmd-theme = Theme… (t)
tui-theme-title = Theme
tui-theme-folder = Theme files go in { $folder }. A theme file in use is read again when it changes.
tui-theme-keys = ↑↓ choose · Enter use this theme · q or Esc back
tui-theme-terminal = Your terminal's colours
tui-theme-terminal-hint = No theme: the window is drawn in the colours your terminal shows.
tui-theme-name-dark = Dark
tui-theme-name-light = Light
tui-theme-name-high-contrast = High contrast
tui-theme-name-large-text = Large text
tui-theme-name-slate = Slate
tui-theme-name-sand = Sand
tui-theme-scheme-dark = dark
tui-theme-scheme-light = light
tui-theme-with-scheme = { $name } · { $scheme }
tui-theme-refused-row = { $file } · can't be used
tui-theme-in-file = File: { $file }
tui-theme-starter = A starter theme: the colours of { $project }, ported ({ $licence } licence).
tui-theme-shows = Shown in its { $scheme } colours.
tui-theme-unused = Not used in the terminal client: { $list }.
tui-theme-unused-sizes = text size, spacing and corners
tui-theme-unused-text = fonts and text style
tui-theme-unused-region = colours of { $region }
tui-theme-unused-cursor = the cursor colour
tui-theme-unused-syntax = code colours
tui-theme-chrome-tab-bar = tab bar position
tui-theme-chrome-tab-style = tab style
tui-theme-chrome-state-display = how an agent's state is shown
tui-theme-chrome-icons = icons
tui-theme-chrome-shadow = shadows
tui-theme-chrome-motion = motion
tui-theme-warnings = Shown, but some colours would be hard to see in places:
tui-theme-refused = This file can't be used:
tui-theme-cannot-use = This file can't be used; the reasons are listed below.
tui-theme-no-colour = Colours are off in this terminal (NO_COLOR), so no theme is shown. The choice is kept for a terminal with colours.
tui-theme-chosen = Theme: { $name }.
tui-theme-not-kept = Theme: { $name }, for now: it could not be kept for the next start ({ $code }).
tui-theme-fell-back = The theme file { $file } can't be used now, so your terminal's colours are shown. Press t to see why.
tui-theme-in-pack = In the pack "{ $pack }" (file { $file }): theme { $n } of { $count }.
tui-theme-pack-unused = Also in the pack, not used in the terminal client: { $list }.
tui-theme-pack-layouts = { $count ->
    [one] 1 saved layout
   *[other] { $count } saved layouts
}
tui-theme-pack-keys = { $count ->
    [one] keys for 1 command
   *[other] keys for { $count } commands
}
tui-theme-error-pack-field = "{ $field }" is not part of a theme pack.
tui-theme-error-pack-name = A theme pack needs a name of 1 to { $max } characters, without ";", "<" or "url(".
tui-theme-error-pack-part = "{ $part }" is not in the form a theme pack keeps it in.
tui-theme-error-pack-too-many-themes = A theme pack holds at most { $max } themes.
tui-theme-error-pack-too-many-layouts = A theme pack holds at most { $max } layouts.
tui-theme-error-pack-theme-format = Theme { $n } of the pack is not a theme of the current format.
tui-theme-error-in-pack-theme = Theme { $n } of the pack: { $text }
tui-theme-error-pack-no-themes = This theme pack has no theme. Its { $layouts ->
    [one] 1 layout
   *[other] { $layouts } layouts
} and keys for { $keys ->
    [one] 1 command
   *[other] { $keys } commands
} are for the web window; the terminal client does not use them.
tui-theme-token-surface-0 = Background
tui-theme-token-surface-1 = Panels
tui-theme-token-surface-2 = Raised surfaces
tui-theme-token-border = Lines
tui-theme-token-text = Text
tui-theme-token-text-muted = Secondary text
tui-theme-token-accent = Accent
tui-theme-token-on-accent = Text on accent
tui-theme-token-state-working = Working
tui-theme-token-state-needs-you = Needs you
tui-theme-token-state-finished = Finished
tui-theme-token-state-idle = Idle
tui-theme-token-state-unknown = Unknown
tui-theme-token-font-ui = Interface font
tui-theme-token-font-mono = Terminal and code font
tui-theme-token-text-16 = Text size
tui-theme-token-space-1 = Spacing
tui-theme-token-radius-xs = Corners: tiny
tui-theme-token-radius-s = Corners: small
tui-theme-token-radius-m = Corners: medium
tui-theme-token-radius-l = Corners: large
tui-theme-region-sidebar = the agents list
tui-theme-region-tab-bar = the tabs
tui-theme-region-pane-bar = pane bars
tui-theme-region-status-line = the status line
tui-theme-region-cards = cards
tui-theme-region-inputs = text fields
tui-theme-region-buttons = buttons
tui-theme-region-agent-entries = agent replies
tui-theme-region-person-entries = your messages
tui-theme-region-code-blocks = code blocks
tui-theme-region-part-surface = Background of { $region }
tui-theme-region-part-text = Text of { $region }
tui-theme-terminal-background = Terminal background
tui-theme-terminal-foreground = Terminal text
tui-theme-terminal-selection = Selection
tui-theme-terminal-black = Black
tui-theme-terminal-red = Red
tui-theme-terminal-green = Green
tui-theme-terminal-yellow = Yellow
tui-theme-terminal-blue = Blue
tui-theme-terminal-magenta = Magenta
tui-theme-terminal-cyan = Cyan
tui-theme-terminal-white = White
tui-theme-terminal-bright-black = Bright black
tui-theme-terminal-bright-red = Bright red
tui-theme-terminal-bright-green = Bright green
tui-theme-terminal-bright-yellow = Bright yellow
tui-theme-terminal-bright-blue = Bright blue
tui-theme-terminal-bright-magenta = Bright magenta
tui-theme-terminal-bright-cyan = Bright cyan
tui-theme-terminal-bright-white = Bright white
tui-theme-floor-danger = The red for errors and danger is always the same, so it can't be hidden or copied.
tui-theme-floor-diff-removed = Removed lines are always the same red.
tui-theme-floor-approval-card = A request from an agent always looks the same, so no theme can hide or fake what it asks.
tui-theme-floor-focus-ring = The focus ring always shows where the keyboard is.
tui-theme-floor-needs-you = "Needs you" always keeps its shape, its word and its place.
tui-theme-floor-plugin-marker = What a plugin adds is always marked.
tui-theme-error-json = This file is not JSON.
tui-theme-error-invalid = This is not an Agora theme file.
tui-theme-error-newer = This theme was made by a newer version of Agora.
tui-theme-error-name = A theme needs a name of 1 to { $max } characters.
tui-theme-error-scheme = The theme must say whether it is "dark" or "light".
tui-theme-error-unknown-field = "{ $field }" is not part of a theme file.
tui-theme-error-unknown-token = "{ $token }" is not a theme token.
tui-theme-error-derived-token = "{ $token }" follows from "{ $base }": set "{ $base }" instead.
tui-theme-error-bad-color = { $label } ("{ $token }") must be a colour like #1a2b3c.
tui-theme-error-bad-size = { $label } ("{ $token }") must be between { $min } and { $max } px.
tui-theme-error-bad-font = { $label } ("{ $token }") must be a list of font names separated by commas.
tui-theme-error-contrast = { $fg } on { $bg } is { $ratio }:1; it needs at least { $min }:1 to be readable.
tui-theme-error-seen = { $fg } on { $bg } is { $ratio }:1; it needs at least { $min }:1 to be seen.
tui-theme-error-no-look = A theme needs a "dark" or "light" part, or a built-in theme to start from ("inherits").
tui-theme-error-moved-token = "{ $field }": "{ $token }" goes in "{ $to }".
tui-theme-error-field-color = "{ $field }" must be a colour like #1a2b3c.
tui-theme-error-field-size = "{ $field }" must be a whole number of pixels from { $min } to { $max }.
tui-theme-error-field-list = "{ $field }" must be one of: { $values }.
tui-theme-error-field-line-height = "{ $field }" must be "normal" or a number from { $min } to { $max }.
tui-theme-error-field-switch = "{ $field }" must be true or false.
tui-theme-error-field-group = "{ $field }" must be a group of named values.
tui-theme-error-unsafe = "{ $field }" has text a theme can't hold (like "url(", ";" or "<"): a theme holds only colours, sizes, words from a list and font names.
tui-theme-error-floor = "{ $field }" can't be set. { $reason }
tui-theme-error-text-floor = "{ $field }": no text is smaller than 12 px, so the text size is at least { $min } px.
tui-theme-warning-text-lifted = The text size { $from } would make some text smaller than 12 px: it is read as { $to }.
tui-theme-error-inherits = "{ $id }" is not a built-in theme to start from.
tui-theme-error-too-large = This file is larger than { $max } KiB; a theme is far smaller.
tui-theme-error-unreadable = This file could not be read.
tui-theme-error-more = { $count ->
    [one] …and one more problem.
   *[other] …and { $count } more problems.
}
# A problem in one scheme of a theme that has both; $scheme is tui-theme-scheme-dark or -light.
tui-theme-issue-in-scheme = In the { $scheme } colours: { $text }
# ADR-0068 rung 5: layouts in the terminal client (docs/ux/layout.md, "The terminal client"). The panel names and the presets use the
# web window's words (web.ftl layout-*), so both clients name the same thing the same way.
tui-app-keys-tree-tiled = Ctrl K commands · Enter show here · h / l pane · z maximize · 1…6 layout · w close pane · N next that needs you · y / n answer · s start · f folder · x stop · u resume · r changes · o files · b board · d remove · a ask · v devices · Tab next part · i details · q quit
tui-app-keys-tree-tiled-viewer = Ctrl K commands · Enter show here · h / l pane · z maximize · 1…6 layout · w close pane · N next that needs you · r changes · o files · b board · a ask · Tab next part · i details · q quit
tui-layout-panel-approvals = Needs you
tui-layout-panel-fleet = Fleet
tui-layout-panel-empty = Empty pane
tui-layout-panel-conversation = Conversation
tui-layout-panel-changes = Changes
tui-layout-panel-files = Files
tui-layout-panel-manager = File manager
tui-layout-panel-tasks = Shared tasks and files
tui-layout-panel-messages = Team chat
tui-layout-panel-usage = Usage
tui-layout-panel-theme = Theme
tui-layout-panel-plugin = Plugin
tui-layout-follows-attention = Follows what needs you
tui-layout-follows-selection = Follows the selection
tui-layout-maximized = maximized · z restores
tui-layout-nothing-waiting = Nothing needs you now. This pane shows the next agent that does.
tui-layout-nothing-selected = Select an agent in the list to see it here.
tui-layout-empty-hint = Empty pane. Choose an agent in the list and press Enter to show it here.
tui-layout-chat-focus = This agent is a chat. Move here (h / l, or a click) to read and write.
tui-layout-no-terminal = This agent is a chat; it has no terminal.
tui-layout-approvals-none = Nothing needs you.
# A panel of the web window the terminal client does not draw; $panel is its name.
tui-layout-elsewhere = { $panel }: open in the window to see it.
# A panel id no client here knows (a newer version's); $panel is the id as the layout names it.
tui-layout-unknown = Unknown panel “{ $panel }”: open in the window to see it.
tui-layout-cmd-1x1 = Layout: one pane with tabs (1)
tui-layout-cmd-2x1 = Layout: two side by side (2)
tui-layout-cmd-1x2 = Layout: two, one above the other (3)
tui-layout-cmd-2x2 = Layout: four, 2 × 2 (4)
tui-layout-cmd-focus = Layout: one large, three small (5)
tui-layout-cmd-3x2 = Layout: six, 3 × 2 (6)
tui-layout-cmd-maximize = Maximize this pane (z)
tui-layout-cmd-restore = Restore the layout (z)
tui-layout-cmd-split-right = Split right
tui-layout-cmd-split-down = Split down
tui-layout-cmd-close-pane = Close pane (w)
tui-layout-cmd-next-pane = Next pane (l)
tui-layout-cmd-swap = Swap with the next pane
tui-layout-cmd-move = Move to the next pane, as a tab
tui-layout-cmd-take-out = Take out of the tabs
tui-layout-cmd-wider = Make this pane wider
tui-layout-cmd-narrower = Make this pane narrower
tui-layout-cmd-taller = Make this pane taller
tui-layout-cmd-shorter = Make this pane shorter
tui-layout-cmd-show-approvals = Show here: Needs you
tui-layout-cmd-show-fleet = Show here: Fleet
tui-layout-cmd-follow-attention = Show here: what needs you next
tui-layout-cmd-follow-selection = Show here: the agent selected in the list
tui-layout-cmd-save = Save layout as “{ $name }”
tui-layout-cmd-save-as = Save layout as…
tui-layout-name-title = Save the layout
tui-layout-name-hint = Type a name for the layout, then Enter. A saved layout of that name is replaced.
tui-layout-cmd-use = Use layout “{ $name }”
tui-layout-cmd-delete = Delete layout “{ $name }”
tui-layout-cmd-reset = Reset layout
# $folder is where the file went (~/.agora/layouts); the web window's Import reads the file as it is.
tui-layout-saved = Saved the layout “{ $name }” in { $folder }. The web window can import that file.
tui-layout-save-failed = Could not save the layout “{ $name }”.
tui-layout-used = Using the layout “{ $name }”.
tui-layout-deleted = Deleted the layout “{ $name }”.
tui-layout-delete-failed = Could not delete the layout “{ $name }”.
# $why is one of tui-layout-error-*.
tui-layout-cannot-use = Could not use the layout “{ $name }”: { $why }
tui-layout-error-missing = the file is not there.
tui-layout-error-too-large = the file is too large to be a layout.
tui-layout-error-unreadable = the file cannot be read.
tui-layout-error-json = this is not valid JSON.
tui-layout-error-invalid = this is not a layout this client can read.
tui-layout-error-newer = it was made by a newer version. Update to use it.
tui-layout-error-too-deep = it is nested too deeply (at most 6 levels).
tui-layout-error-too-many = it has too many panes (at most 32).

## ADR-0090 step 7: a structured agent's conversation (thinking, tool status, plan steps, Stop turn, settings while it runs).
tui-chat-hint-rich = Type a request and press Enter · /mode, /model, /effort or /set change how it runs · /stop ends the turn · /thinking shows the thinking · Ctrl-] goes back
tui-chat-thought = thinking (hidden; /thinking shows it)
tui-chat-thought-plain = thinking (hidden)
tui-chat-thought-shown = thinking:
tui-chat-queued = (waits until this turn ends)
tui-chat-permission = asked for your permission
tui-chat-permission-pending = asks for your permission
tui-chat-permission-approved = was allowed
tui-chat-permission-always = was allowed always
tui-chat-permission-denied = was denied
tui-chat-permission-cancelled = asked, and nobody answered before the turn ended
tui-chat-effort = effort { $effort }
tui-plan-step-done = ✓ { $text }
tui-plan-step-now = ▸ { $text }
tui-plan-step-todo = · { $text }
tui-tool-status-pending = waiting
tui-tool-status-running = running
tui-tool-status-done = done
tui-tool-status-failed = failed
tui-tool-status-stopped = stopped
tui-tool-seconds = in { $seconds } s
tui-turn-stopped = The turn is stopped; the agent keeps running and the requests that wait are sent next.
tui-turn-stopped-dropped = The turn is stopped and the requests that waited are dropped; the agent keeps running.
tui-turn-stop-failed = Could not stop the turn ({ $code }).
tui-config-confirm = With { $option } set to { $value } the agent acts without asking you. Type the same line again to switch; the machine asks for your signed approval first.
tui-config-current = { $option } is { $value } already.
tui-config-unknown = The agent has no setting “{ $id }”. Its settings: { $ids }
tui-config-values = { $option } takes: { $values }
tui-config-no-effort = This agent does not offer its effort while it runs; it is chosen when the agent starts.
tui-config-approval-needed = The machine waits for your signed approval before switching: answer the request when it shows.

## ADR-0066: the start's settings (model, effort, ...).
tui-start-settings-step = Settings for the start ({ $n } of { $total })
tui-start-settings-pick = { $setting }: press its number. Enter keeps the default, Esc cancels the start.
tui-start-settings-pick-last = { $setting }: press its number. Enter keeps { $value } (your last choice here), 1 the default, Esc cancels the start.
tui-start-settings-type = { $setting }: type a value and press Enter. Enter alone keeps { $default }; Esc cancels the start.
tui-start-settings-type-last = { $setting }: Enter keeps { $value } (your last choice here). Type another value, or clear the field to keep { $default }; Esc cancels the start.
tui-start-settings-default = the default ({ $value })
tui-start-settings-default-tool = the tool's default
tui-start-settings-default-word = default
tui-start-settings-more = n  more ({ $page } of { $pages })
tui-start-settings-invalid = A value starts with a letter or a digit and holds only letters, digits and . _ : / @ + -
tui-start-settings-back = Backspace goes back to the setting before.

## ADR-0068 decision 6: the keybindings file (docs/ux/keybindings.md).
tui-keys-refused = Your keybindings file (keybindings.json in the Agora folder) is not used; the default keys are in force:
tui-keys-error-json = keybindings.json is not valid JSON.
tui-keys-error-too-big = keybindings.json is too large (at most 64 KB).
tui-keys-error-shape = keybindings.json is not a keybindings file this client can read.
tui-keys-error-newer = keybindings.json was made by a newer version. Update to use it.
tui-keys-error-too-many = keybindings.json lists too many commands (at most 200).
tui-keys-error-unreadable = keybindings.json could not be read.
tui-keys-error-field = Unknown field “{ $field }”.
tui-keys-error-command = “{ $command }” is not a command name.
tui-keys-error-request = “{ $command }”: a request is answered on its own, never with a key from a file.
tui-keys-error-fixed-command = “{ $command }” keeps its keys; they cannot be changed.
tui-keys-error-twice = “{ $command }” is listed twice.
tui-keys-error-too-many-keys = “{ $command }” has too many keys (at most 4).
tui-keys-error-key = “{ $key }” for { $command } is not a key.
tui-keys-error-fixed-key = { $key } for { $command } stays as it is ({ $reason }).
tui-keys-error-more = …and { $count } more.
tui-keys-fixed-safe-mode = safe mode
tui-keys-fixed-navigation = moving around the window
tui-keys-fixed-terminal = a terminal's copy, find and paste
tui-keys-fixed-editing = editing text
tui-keys-kept-answers = { $key } for { $command } is not used here: in the terminal client it answers a request.
tui-keys-kept-moving = { $key } for { $command } is not used here: in the terminal client it moves around the window.
tui-keys-unknown = Kept for the web window, not used here: { $commands }

# M6.7: the bell and desktop notification when an agent finishes or needs you, and the finished mark.
tui-alert-title = Agora
tui-alert-finished = { $agent } finished
tui-alert-needs-you = { $agent } needs you
tui-alert-finished-unnamed = An agent finished
tui-alert-needs-you-unnamed = A request needs you
tui-app-cmd-bell-off = Turn the bell off (when an agent finishes or needs you)
tui-app-cmd-bell-on = Turn the bell on (when an agent finishes or needs you)
tui-bell-on = The bell rings when an agent finishes or needs you.
tui-bell-off = The bell is off. The list still marks agents that finish or need you.
tui-settings-not-kept = It could not be kept in { $file }, so it holds until you quit.
tui-finished-unseen = ✓ Finished while you were away. Open it to clear this mark.
