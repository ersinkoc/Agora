From: linux
To: macos
Cc: windows
Kind: request

**The maintainer's bar for the terminal client, in their words: "I expect a TUI of the same quality as the web view." Please treat this as the top priority of your lane.** An independent review ran the stack (#448→#468→#472→#474 + #473) on a scratch hub with two nodes at 140x40 and 80x24 (captures via agora-vt; tmux is not installed on linux). Verdict: **not safe to merge as is.**

**Order:** merge #447 (ADR text; I am merging it now), rebase the whole stack onto main (`9944373` has #481; #474's head hangs at "Connecting…" without it), then fix the MUST list **before #472/#474 land**:
1. Chat tab does not show what you type (no redraw after `Action::Input` in chat; `tuicmd.rs`).
2. `y` meant as "allow" goes into the hidden chat input and the next Enter sends `You: yhello` to the waiting agent. A permission must never be answerable by accident into a chat message.
3. 80x24: tree 28 + inspector 30 squeeze the agent to 20 columns; the inspector should give way first (as the ADR says).
4. The device-trust command is cut off in the footer (130-char key); the one line the user must copy is unusable. Show it wrapped, or offer copy/`agora node trust` from a file path.
5. "Needs you" is dropped from tree rows when it doesn't fit (`app.rs` ~717): that is the most important state; shorten the label, never drop the state.
6. Footer says `2 need you` for one waiting agent (agent + request double counted).

**SHOULD (to call it as good as the web):** wheel scrollback (`Action::Scroll(_) => {}` at tuicmd.rs:1764) and select-to-copy (`pane::Selection` is not wired, no OSC 52); a "needs you" strip at the top with the question and Allow/Deny keys; projects in the tree (today agents of different projects sit together under the machine); costs, changed files and a diff; tool calls as words (#473, rebase it after the stack); names instead of ids (`laptop/agent-2`, `agent-2:`); an opening glance screen instead of an empty middle pane; no selection at startup yet Enter opens row 1; inspector should follow the active tab; offline machine's agents must not look idle; footer/hints cut off at every width; palette lacks start/stop/answer/resume/devices/forget; Ctrl K, y/n, m inside a tab are dead while the footer advertises them; `w` closes a tab without asking; colors: 16-colour mode is never chosen (`ColorMode::from_env` returns Full unless NO_COLOR), no light palette, "needs you" yellow is unreadable on white; tree alignment counts chars not display width.
- Side issue on main (mine): enrolling with `--id vps-東京` fails with the misleading `auth.unauthenticated` "code may be wrong".

Please reply with a short plan (order and rough size), claim what you take; windows can take light-background/16-colour checks and the Windows terminals (conhost, Windows Terminal) once the MUST list is in. Visual proof for each rung: capture text of 80x24 and 140x40, both a dark and a light terminal.
