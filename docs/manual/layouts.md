# Layouts: arrange panes in a file

A **layout** is a tree of splits, tab groups and panes, as plain JSON. The web window and the terminal client read and write the same
file, so a layout made in one opens in the other.

## Where it goes

- **Terminal client:** each saved layout is `layouts/NAME.json` in the Agora folder (`$AGORA_HOME`, else `~/.agora`). The name is
  a file name: not hidden, at most 60 characters, none of `/ \ : * ? " < > |`. At most 50 are listed. In `agora tui`, open the
  commands (Alt+K, or Ctrl K in the agent list) and choose **Use layout “NAME”**. The arrangement in use is kept in `tui-layout.json` there;
  the client writes it, so leave it alone.
- **Web window:** the person chooses **Import…** in the **Layout** menu of the title bar and pastes the file or picks it. **Export**
  there downloads `layout.json`, the same format.

## Format, version 1

```
Layout = { "version": 1, "root": Node }
Node   = Split | Tabs | Pane
Split  = { "type": "split", "direction": "row" | "column", "ratios": [number, ...], "children": [Node, ...] }
Tabs   = { "type": "tabs", "active": number, "children": [Pane, ...] }
Pane   = { "type": "pane", "id": string, "panel": string, "bind": Bind }      ("id" and "bind" are optional)
Bind   = { "agent": "MACHINE/AGENT_ID" } | { "follow": "attention" } | { "follow": "selection" }
```

- A `row` split puts its children side by side; a `column` split stacks them. `ratios[i]` is child `i`'s share.
- A tab group shows its pane number `active` (from 0) in front.
- A pane shows one **panel**. `id` is a short name unique in the layout; a missing or repeated one is replaced when read.
- `bind.agent` is an agent's key on the hub: the machine id, `/`, the agent id (`laptop/agent-1`). `follow: "attention"` shows
  the next agent that needs the person; `follow: "selection"` shows the agent selected in the agent list.

Panels:

| Panel | Shows |
| --- | --- |
| `fleet` | Home: what needs the person, then every agent |
| `agent` | the bound agent's terminal or chat |
| `terminal` | the bound agent's terminal |
| `conversation` | the bound agent's chat |
| `approvals` | the requests that wait for the person |
| `changes` | the bound agent's changed files and their diff |
| `files` | the bound agent's folder as a tree, read-only |
| `manager` | a file manager of the bound agent's folder, read-only |
| `tasks` | the shared tasks |
| `messages` | messages between agents |
| `usage` | tokens, context and time |
| `theme` | the theme editor (web window) |
| `empty` | a chooser |

A panel this client does not know is kept and shown as a placeholder; it never fails the layout. The terminal client draws
`terminal`, `agent`, `approvals`, `fleet` and `empty`, and shows the others as "open in the window to see it".

## Rules when it is read

- Refused: not JSON; no integer `version` or no `root` of a known `type`; a split with another direction; a pane without a
  `panel`; a split inside a tab group; a `version` above 1 ("made by a newer version").
- At most 32 panes, and at most 6 levels from the root to a pane (the pane counted).
- Fixed quietly: ratios that are missing, of the wrong count or not positive are shared out equally, and ratios are scaled to sum
  to 1; an empty split or tab group is dropped; a split with one child becomes that child; a split inside a split of the same
  direction is merged into it; `active` is kept in range. No child gets less than 10 % when resized.
- The terminal client reads a file of at most 256 KiB; a larger one is refused before it is read.

## Example

Two agents side by side, the requests that wait under them.

```json agora-layout
{
  "version": 1,
  "root": {
    "type": "split",
    "direction": "column",
    "ratios": [0.7, 0.3],
    "children": [
      {
        "type": "split",
        "direction": "row",
        "ratios": [0.5, 0.5],
        "children": [
          { "type": "pane", "id": "p1", "panel": "agent", "bind": { "agent": "laptop/agent-1" } },
          { "type": "pane", "id": "p2", "panel": "agent", "bind": { "follow": "attention" } }
        ]
      },
      { "type": "pane", "id": "p3", "panel": "approvals" }
    ]
  }
}
```
