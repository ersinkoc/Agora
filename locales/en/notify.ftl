# Notification text, rendered in the recipient's locale (docs/design/notifications.md). Plain words, no internal terms.

notify-agent-blocked = { $agent } needs you.
notify-agent-done = { $agent } finished.
notify-approval-requested = { $agent } is asking for your OK.
notify-task-completed = A task was completed: { $task }.
notify-node-offline = { $machine } went offline.
notify-budget-threshold = Spending reached { $percent }% of your budget.
notify-update-available = An update is available: { $version }.
notify-digest =
    { $count ->
        [one] 1 update while you were away.
       *[other] { $count } updates while you were away.
    }
notify-title = Agora
notify-test = Agora notifications work.
notify-desktop-approvals =
    { $count ->
        [one] An approval is waiting for you.
       *[other] { $count } approvals are waiting for you.
    }
