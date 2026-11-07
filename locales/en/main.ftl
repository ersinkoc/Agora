# Source locale. Every user-facing string lives here; other locales translate these keys.
# Keys are kebab-case and namespaced by area.

app-name = Agora
agent-state-working = Working
agent-state-blocked = Needs input
agent-state-idle = Idle
agent-state-done = Done
agent-state-unknown = Unknown
approvals-pending = { $count ->
    [one] { $count } approval pending
   *[other] { $count } approvals pending
}
nodes-connected = { $count ->
    [one] { $count } node connected
   *[other] { $count } nodes connected
}
