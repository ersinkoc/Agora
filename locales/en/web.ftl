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
