# Main window

![Window, dark](screens/window-ide-dark.png)
![Window, light](screens/window-ide-light.png)
![Terminals side by side](screens/window-ide-grid.png)
![Full size](screens/window-full-dark.png)
![Start an agent](screens/window-start-chosen-dark.png)

## Browsing folders

![Folder picker](screens/window-folder-picker-dark.png)

**Browse folders** under the working-folder field opens a list of the machine's sub-folders (`directory.list`, ADR-0053; it starts in the home folder, or the typed path when there is one). Click a folder to step into it, the arrow goes up one, a Git project is marked, and **Use this folder** continues as if the path had been typed (the machine still resolves and shows it before anything starts). A machine without the command keeps the typed path.

## Closed agents

![A closed agent's summary](screens/window-closed-dark.png)

An agent whose process is gone moves from its folder to **Closed**, below the machines, with how long ago it ended; a failed one is marked. Opening it shows a summary (how it ended, when, for how long, folder and branch, files left behind with Review/History/Files, tokens and cost), **Start an agent** on this machine and **Remove from the list**. **Clear closed** removes them all. **Remove from this list** is remembered in this browser only; **Remove for everyone** asks the hub to forget the agent (`agent.forget`, ADR-0053): every client stops listing it, while its earlier events and its cost stay in the hub's record. A hub without the command answers with an error and the agent is only hidden here.

## Hub and machines

![Hub and machines](screens/network-dark.png)

`#/machines` draws the hub with a live link to every machine it knows (animated when connected, dotted when not), a card per machine (state, system, node version, agents, terminals, memory, cost, when it was last heard, or for an offline one "last seen", ADR-0070) and the agents running there as chips. An offline machine also shows "last seen" on its row in the agent list and in its tooltip.

**Old machines** (ADR-0073): a machine not seen for more than a day is listed apart, folded, under **n old machines**, and is not
offered for new agents. An offline machine has **Forget…**, and the old ones have **Clean up…**: one dialog lists them, ticked,
with when each was last seen and how many agents were running then, and one box also refuses them if they connect again (a new join
code is needed then). The hub forgets them for every window (`node.forget`, owner only); what they did stays in the history and the
costs, and a machine that connects again comes back with the agents it still runs. `agora hub machines` does the same from the hub's
machine.

![Old machines](screens/old-machines-dark.png)
![Cleaning up old machines](screens/old-machines-cleanup-light.png)

## Layout (editor-style, 2026-11-12)

The window is laid out like an editor, with the terminal as the content. At most two side panels are visible (visual-language.md); each hides with its button.

- **Title bar:** product mark, the search and command field (Ctrl+K), connection state, language, device.
- **Left, Agents:** a tree of machine, folder, agent. A machine row has a start button (visible on hover and focus). Agents show their state shape, shared-folder label and changed-file count. The project filter and search narrow the tree. Clicking an agent opens it as a tab; clicking the one in front closes it.
- **Middle, panes (ADR-0068, [layout.md](layout.md)):** a layout of panes and tab groups: the Fleet tab and one tab per watched agent by default, or 1x2, 2x1, 2x2, 3x2 and one large with three small from the **Layout** menu in the title bar, saved by name, exported and imported. Panes are split, resized by their dividers, dragged onto each other, maximized and restored; a pane can follow what needs you or the selected agent. Above each terminal: machine, folder and branch, changed files, cost, state, History, Open beside (Changes, Files, File manager), Full size, and the copyable agent address.
- **Tool panes (ADR-0068):** Terminal, Changes, Files and File manager are panes of the layout, not only dialogs and side panels: they are placed in splits and tab groups, dragged, maximized, closed and remembered on this device with the rest of the layout. Open one from the **+** of a tab group (also **New terminal**), from the command palette (**Open Changes beside**, **Open Files beside**, **Open the File manager beside**, **Open the terminal beside**, **Start a new terminal**), from the **Open beside** menu in an agent's header, from the changed-files chip there, and from the icon beside each workspace in the rail's Changes list. Opened from an agent, a pane shows that agent's folder; opened from nowhere it shows a short list of the folders (or terminals) it could show. A pane's bar names the folder and switches to another (another agent's, the same project on another machine) without closing the pane; the pane's own name leads in its tab (**Changes · claude · amber**). A tool already open for that agent comes to the front instead of a second one; a terminal for an agent already on screen brings that agent forward. Phones keep their single column and the dialogs.
  - **Changes:** the files list (grouped by folder, filter by status and name) beside the diff, as in the review screen: side by side or unified, unchanged lines folded and shown on request, the file as it is now. Not available without a link to the machine (it says so).
  - **Files:** the tree, a file with syntax colours and the places it changed in the gutter, open files as tabs, search by name (`/`) and in the file (Ctrl F).
  - **File manager:** a breadcrumb, a folder tree (one Tab stop; arrows, Right and Left), and a list of name, kind (an icon and a word, by extension) and size that sorts on a click on the column and filters by name (`/`). Arrows move between rows, Enter or a click opens a folder or previews a file (syntax colours; text only, a private file is never read), Backspace or Left goes up, Escape closes the preview, then clears the filter. **Copy path** (relative to the folder), **Download** (a chosen, non-private file, as the machine returns it), **Agent terminal** (the agent's terminal beside, at the folder's top only). **Rename, Move, Delete and Upload are shown switched off** with the reason as their tooltip and description: changing files needs your approval on the machine and a policy the machine does not offer this window yet. Not shown, because the machine does not report them: a modified time, a full path, listing a folder outside an agent's own.
  - **Terminal:** one agent's terminal as a tile. A plain terminal on a machine and folder is a normal start of the `shell` kind (**New terminal** opens the start window on it, then the new terminal appears as a pane); a machine without a `shell` manifest says so and offers the agents instead.
  - Every pane has an empty state (nothing to show yet, with what makes it appear), a loading state, and an error with Try again; all are announced, keyboard-operable and still under reduced motion.
- **Right, Details:** an icon rail that is always visible (also on a narrow window), with Needs you (badge with the count; opens first whenever something needs you), Changes (count), Board, Messages and Usage (Board and Usage appear only when there is something; Usage is a bars icon, and shows tokens, context and time, never money, with the open subject's own tooltip left off so it cannot cover the panel's values). Each icon has a tooltip with its name and shortcut (Alt+Shift+1 to 5; the same key closes the panel). The subject opens beside the rail and is part of the layout on a wide window, a drawer over the content on a narrow one (Esc closes it); whether it is open and which subject is remembered on this device. The Needs you icon is always there, on Home too: it is never hidden, even though the Fleet console lists the same requests.
- **Tab rows that do not fit** (the pane tabs): they scroll sideways, the edge with more fades, the tab in front is kept in view, and a **More tabs** menu lists the tabs that are cut off.
- **Group by** (the agent list and the Fleet console share one choice, remembered on this device): Machine, Project, Agent (kind) and State. A project is the hub's machine-independent project id, so the same project open on several machines is one group, marked **Same project · n machines** (on tiles, in the list and on the group), with a gentle hint when agents on two or more machines have changes in it. Folders that merely share a name are never merged; an agent without a project id is grouped only with the same folder on the same machine.
- **Agent header** (agents with a chat): one row with a Model and a Mode menu (only what the agent reported over ACP, or its own settings including an Effort menu when it offers them, ADR-0090), context fill, elapsed time (when this window saw it start) and the tokens used. No money is shown here.
- **Acts freely badge:** an agent started so that it acts without asking shows a warning sign and the short label **Acts freely** (the sign alone in a tab); the whole explanation is the tooltip, its accessible name, and is shown on focus or tap where the badge is not inside another control.
- **Command palette extras:** **Machine: ⟨name⟩** (the machines page, at that machine), **Project: ⟨name⟩** (shows that project's agents; not on a phone), **Allow a new machine to join** (brings the existing window for new machines into view on Home; its own button still opens it and each join is still approved with the number), **Lock** (only when a screen password is set; the same action as the title bar's Lock) and **Settings** (This device). Entries are names and navigation: none carries a secret and none answers a request by itself.
- **Status bar:** machines online, agents, how many need you, cost so far.
- **Home, the fleet console ([fleet-console.md](fleet-console.md)):** a window nobody has arranged opens on a **Fleet** tab in front of the agents' tabs: five live counts (needs you, working, finished, failed, machines online) that filter the tiles, what needs you first with Allow and Deny in the row for a plain yes or no, then every agent as a tile grouped by machine or by project. With no machine, no agent or no link it teaches the next step instead.
- **One terminal in several windows:** a terminal open in more than one window (two machines, or a phone and a laptop) has the size of the smallest of them, so it fits every one; a bigger window shows it at its top left with a thin line where it ends, and gets its own size back as soon as the smaller window closes it. A terminal client on the machine itself still sets the size it wants (ADR-0072; docs/specs/protocol.md, "Shared terminal size").
- **Phones:** a single column instead of this window: the same five counts, what needs you, machines and agents, then one agent (ADR-0067 rung 2, [phone.md](phone.md)). A tablet keeps this window; on a touch screen its terminals get the key bar.

The older three-column description below stays valid for what each area holds.

Agreed layout (2026-11-06). Three columns plus a top bar, shown in the **Focus** layer.

Default state follows `visual-language.md`: the window opens on **Home** (Glance: attention strip + fleet of
live agent tiles). Selecting an agent opens this three-column layout. The right column shows one panel at a
time (Files, Changes, Team chat, Usage as tabs) and can be collapsed; the Machines list is a filter, not a
permanent section. Labels below use internal terms; the UI uses the vocabulary in `visual-language.md`.

## Needs you, loudly

A blocked agent is never invisible. The Needs you tab, the fleet console, the status bar count, the tab title and (on a phone) the bar at the
bottom always show it; nothing below can hide them. On top of that, **This device** chooses how loud the window is (kept in this browser
only, default **A card over the window**): a card, a small banner only, or nothing extra.

- **The card** (`AttentionLayer`) opens for a request: for a plain yes or no that is shown whole (the fleet console's `decidableInline`) it has
  **Allow** and **Deny**; for anything else (a request to start an agent, settings, long, several-line or structured text, a hidden
  character, a machine asking to join) it says what and has only **Review**, which goes to the full card. It never cuts what is asked, and it
  is never offered for an agent that waits in its terminal with no request open (the banner and the list carry that). On a phone the bar
  at the bottom does this job ([phone.md](phone.md)), so there is no card.
- **It never takes the keyboard.** It opens only while the window is the one in front and the person is not typing: not with the cursor in
  a field or a terminal, and not within 1.5 s of the last key typed into one. Otherwise it waits, the banner (bottom right, not modal)
  shows how many wait, and it opens when the person stops. A card that is open stays what it is; a newer request waits behind it
  ("1 more needs you after this").
- **It cannot be clicked by accident.** Allow and Deny do nothing for 600 ms after the card appeared or anything it shows changed (the
  request, the agent's name, the machine's name; the same dwell as the phone bar, restarted by every change), after the person comes
  back to the window (focus, or the tab shown again) and after the link comes back. Nothing is focused when it opens (Enter answers
  nothing), and the link must be up. A request is answered once: after Allow or Deny its buttons stay dead (the card says the answer was
  sent and offers Review). The card is an alert dialog and says "An agent needs you" in an assertive live region when it opens; a long
  name is cut and wraps, so the buttons are never pushed out of view.
  **Esc, a click outside the card and Later all mean Later**: nothing is sent, and it comes back only through the banner. Nothing about a
  request is stored anywhere; only the two choices of This device are.
- **Notifications** (only after the person presses **Turn on notifications** in This device; the window never asks on its own): while the
  window is in the background, one system notification says "An agent needs you"; an option adds the agent's and the machine's name, only when it is a plain name
  (letters, digits, spaces, `. _ - @`, at most 24 characters, one request) and otherwise the generic text. A request that arrived while the
  window was in front and is still waiting when it goes to the background is told about then. It
  never carries what is asked (a request's text, a command, a path, a prompt). A burst is one notification, plus one more with the final
  count when 15 s have passed; a new one replaces the old. Pressing it brings the window forward and opens the card (or, with Banner
  only, the list). **Nothing extra** sends none. The tab title shows `(n)` and the app badge the count, whatever the choice.

Screenshots in both themes: `docs/screenshots/needs-you-modal/`. Tests: `apps/web/src/components/AttentionLayer.test.tsx`.

## Top bar
- Scope selector (all machines or one project); connected node count.
- Approvals indicator with count; opens the approvals queue (`approvals.md`).
- Global kill switch (stop all agents) behind a confirmation.

## Left column: navigation
- **Projects** tree. Each project expands to its agents and non-agent terminals (servers, tests, logs).
  - Agent row: kind, instance number when several of the same kind exist, state badge
    (`working`, `blocked`, `done`, `idle`, `unknown`).
  - Project row shows a rollup of the most urgent agent state.
- **Machines** list: node name, OS, connection status.

## Center: selected item
- Header: agent name, machine, selected folder/branch.
- Mode toggle per agent: **Terminal** (native agent UI) and **Chat** (structured view, L2/L3 only).
- Inline approval banners for blocked agents (above the content, so they never scroll away).
- **Chat agent (structured, ACP):** a log that reads like a coding tool, in a centred column (780 px) under a header row: the state as a pill with its running time (a dot that pulses while it works), the context bar and the tokens used. The person's requests are accent-tinted bubbles on the right with their time; everything the agent says and does between two requests is one **turn** with an avatar tile, its name, the model and the time (chunks of one reply are joined and grow in place while it streams). Tool calls in a row are **one bordered group** with a header ("3 steps", plus " · 4.1 s" only when the entries carry times) and the kinds and the last step; the newest group is open, older ones are folded, and a group that waits on you or failed is always open. Each step is a row with a kind icon, its title, the file or command as a mono chip and a fold with its input; an edit whose diff the agent sent shows the diff inside its row (line numbers, +/- glyphs and bars, a hunk header), and a diff no step claimed is its own card. A **Files changed** card lists each file once with totals, jumps to its change, and opens the review screen. With a node that speaks the rich form (ADR-0090, capability `chat.rich`) a tool call is ONE row that changes in place (never a second row): a status mark (a spinner while it runs, a hollow ring while it waits, a tick, a cross), how long it took, the files it touched as chips, and folded what it printed (the last 12 lines, the rest on request); a group with a failed call, or right before a request that waits for you, is open and says so, wherever it sits. A thinking block (collapsed, dashed, "Thought for N s", "Thinking…" while it goes on, plain text) is drawn from the node's thought entries. A request for permission is a read-only record in the turn ("Asked for permission", "Permission given", "Permission refused", "Not answered") with a way to the pinned bar; it never answers anything. An older node's conversation is drawn exactly as before. The pending approval is pinned above the composer. Markdown: headings, lists, tables, inline code, code blocks with a language label, a copy button and syntax highlighting (the file viewer's highlighter), and **links only for http(s)** (`javascript:`, `data:`, `file:` and addresses with credentials stay as text); raw HTML is never drawn. Full timestamps are on hover; a date separator marks a new day; errors are notices with a warning icon. The plan is a card with a numbered list; with the node's step status it ticks the done steps, marks the step in hand (a dot, weight and `aria-current`) and draws a progress bar; an older node's plan is the plain numbered list. The log follows new entries only while the reader is at the bottom; scrolled up, a **Jump to latest** pill counts what arrived. At most 120 items are drawn at once, **Show earlier** adds 120 more and keeps the reading position. The composer is multiline (Enter sends, Shift+Enter a new line, input-method confirmation is not a send) with the Mode and Model menus the agent reported and, read-only, what this device chose at start (an effort). With the agent's own settings (`AgentSessionConfig`) the menus are those settings (mode, model, **Effort**, the rest under **More settings**); choosing one sends `AgentSetConfig`, also while the agent works. A choice the node marks as acting without asking (a mode such as Bypass Permissions, marked "does not ask") is an ordinary choice that is sent like any other (ADR-0090 Amendment A): the machine answers that it needs the owner's approval, the control says **Waiting for your approval** (it shows no new value meanwhile) and the machine's own card appears above the composer, written from its rendering with the real option and value ids beside the agent's names, and **Allow** or **Deny**; Allow signs the decision on this device (`agora-approval-skip-v1`). Allow stays off for a short dwell (a visible bar and "Read it first"), one click sends one decision (both answers wait a few seconds after it), Deny is always available, nothing is focused for you, and the attention dialog and the phone bar never answer it inline: it is read whole in its card on the phone too. Applied: the controls show an **Acts freely** badge (its text says "with your approval" only when this window saw that exact value approved; after a restart, a resume or the agent's own switch it says only that the agent acts without asking) for as long as the agent's current mode or setting is one that skips, and it goes when the machine reports a safe value (switching back never asks). Denied, expired or withdrawn (also when the agent ends or restarts, which removes the card at once): the control returns to its previous value and says so. A policy that forbids it (`agent.option_forbidden`) is said in words and the choice stays marked "not allowed on this machine". The window cannot read a node's capabilities, so it answers such a card only for a machine that has answered a switch with `agent.approval_needed` in this session (any other card is read-only with a note), and a card of a shape it does not know (including one without the node's `skips: true`) says "update your window", signs nothing, and can only be dismissed locally. What was heard and answered is forgotten on every new connection. An approved switch whose report comes late is not called declined: the control says it waits for the agent, and after 20 seconds that the agent has not confirmed yet. Typing `/` opens the agent's slash commands (arrows, Enter or Tab to choose, Escape to close); a choice only puts `/name ` in the box. While the agent works **Stop turn** ends the turn only (the agent stays; it is not Stop agent). A prompt sent during a turn waits: its bubble says "Waiting" and a line above the composer says "1 queued" with its text and **Drop the queue**, which asks first because it also stops the turn; a prompt the node dropped reads "Not sent". The window cannot read a node's capabilities (the snapshot has none), so it goes by what the node sent: rich entries, settings or commands, or a prompt answered in the rich form. History: the node replays the last 500 entries or 256 KiB of the conversation from its memory on every (re)connect of a window; it is gone when the node restarts. Not available: editing the last prompt, and (for an older node) everything that needs ADR-0090.
- **Terminal agent:** a **live summary** strip above the terminal with state, the last line on its screen, time running, tokens and the changed-files count (one click opens the Changes panel). Nothing else is shown: a terminal agent has no structured stream.

## Right column: project context
- **Files**: lazy tree of the project checkout on the selected agent's node. Badges for added/modified;
  lock icon for files under a lease.
  Built today (web, read-only; `apps/web/src/components/FilesView.tsx`), in the Files window and in the Files pane (the pane's **Show…** menu or **Open beside**, beside its terminal):
  - The tree marks each changed file with its status letter, and each folder that holds one with a dot. "N changed" lists only
    the changed files, and a name filter searches every file listed so far.
  - Open files are tabs. The path bar's folders open the tree down to the file.
  - The file shows with line numbers and marks in the gutter (added, changed, removed above), read again whenever the node's
    count of changed files moves. The text is drawn a screenful at a time.
  - Ctrl F finds in the file (a query with a capital letter matches case exactly; otherwise every case form matches, Turkish `i`/`İ`
    and `ı`/`I` included). `/` filters the tree. The whole file can be copied.
  - The tree is one Tab stop: arrows move between the items shown, Right opens a folder and goes into it, Left goes back up and
    closes it, Home and End go to the ends.
  - When the changes are larger than 512 KiB, the files after the cut carry no mark, and the tree says so with the machine's count.
  - In a narrow pane the tree sits above the file.
  - Syntax colours (highlight.js, loaded on demand) for files up to 128 KiB in about 25 languages, chosen by file name.
  - Not built yet: a lock mark on a file in the file browser (the task board lists the leases an agent holds), editing.
- **Changes**: workspace diff summary (+/−); shared folders appear once and have a shared-folder label. No per-agent attribution is implied. Opens the review screen (`review.md`).
- **Mesh**: project channels with unread counts; task board progress (`task-board.md`).
- **Usage**: tokens, context and active time per machine, project and agent. No money: most people pay a subscription, so a dollar figure is not shown in headers, cards or summaries; what an agent itself reports about cost sits behind a disclosure inside Usage ("may not reflect your plan"). **Plan limits** (ADR-0091) are what the tool itself reported, never worked out: one progress bar per tool and window ("Five hours", "Seven days", "N minutes") with the percentage, the reset time (relative and absolute, in the window's timezone) and "Observed N minutes ago"; amber from 80 percent, stronger from 95. A window past its reset time says "Reset" (the old percentage is not shown); a reading older than the window itself says "Out of date"; kinds known to have plan limits that reported nothing get one quiet line ("No limit information from this tool: Claude Code"), never a block per machine, and other kinds show nothing Machines that report the same tool are shown apart (each machine's own plan state). An agent's card and row carry a small "Limit 82 %" chip from 80 percent (a fresh reading only), and the start window shows a hint, never a block, when the chosen kind's window is at 95 percent or more. One clock for all of it, ticking at most every 30 seconds. No money.

## Other screens
`review.md`, `approvals.md`, `task-board.md`, `onboarding.md`, `settings.md`.

## Principles
See `experience.md`. Command palette, attention flow, and fleet view are part of this window.

## Start an agent (ADR-0052)

One window: **Machine** chips, then **Working folder** (a known folder in one click, or a typed path), then **Agent** chips and options. Choosing the folder shows the resolved folder, branch and the rule that nothing is changed before anything starts.

Choose an online **Machine**, a known **Project** or another **Working folder** on that machine, then **Use this folder**.
The canonical folder is shown before choosing an agent and starting it. Default launch uses exactly this folder; it does not
create a branch or working copy. A Git folder offers **Create an isolated working copy** as an unchecked option. Non-Git
folders offer terminals/chat and files without Git history or diffs. Newly selected folders appear without restarting the machine.

**Create a shared project ID** explicitly writes `.agent-project.json`. Explain that committing/copying this file groups copies
on other machines; it does not sync source files. A changed previously confirmed ID requires an explicit association checkbox.
The project filter groups the same identity across machines, retaining each machine/path and its sessions. Several agents in
one folder display **Shared folder**. If a machine lacks directory support, show an update message and keep the form open.
Directory actions require a browser device trusted on the machine; the node verifies its exact signed selection.

## Addresses and machine identity

Machine host names are display labels; enrollment IDs remain the identity used for navigation, project-machine counts and
online status. Two machines with the same host name stay separate. Their IDs appear beside the name and in the launch selector
and machine details. A project shared by two enrolled IDs counts as two machines even when both host names match.

Each live agent pane and focused view shows its hub-qualified **Agent address** (`<node>/<agent>`) with an explicit copy action.
Copy the actual routing identifier, never construct it from the host name. The address remains selectable when clipboard access
is unavailable and the UI explains manual copying. Demo tiles without an enrolled identity offer no fictitious address.
