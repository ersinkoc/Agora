# Keys: write a keybindings file

Every action of the web window is a **command** with a stable id. A **keybindings file** gives commands the person's own keys. It
holds only their changes; every command it does not name keeps its default keys.

The terminal client has no keybindings file yet: its keys are fixed.

## Where it goes

The web window keeps the file in the browser, not in a folder. The person opens **Settings > Keyboard** (or **Keyboard settings**
in the command palette, Ctrl K), chooses **Import…**, and pastes the file or picks it. **Export** there downloads `keybindings.json`.
A problem refuses the whole file and is listed; nothing changes.

## Format, version 1

```json
{
  "version": 1,
  "bindings": [
    { "command": "palette.open", "keys": ["ctrl+p", "meta+p"] },
    { "command": "layout.preset-2x2", "keys": ["ctrl+k 2"] },
    { "command": "attention.next", "keys": [] }
  ]
}
```

- `version`: `1`. A higher number is refused as "made by a newer version".
- `bindings`: at most 200 entries of `{ "command", "keys" }`; no other field is allowed, at the top or in an entry.
- An entry **replaces** that command's keys. `[]` takes them all away. At most 4 keys per command; a command may be named once.
- The file is at most 64 KiB.

## Keys

A key is one stroke, or a chord of two strokes separated by one space (`ctrl+k 2`):

```
Stroke   = Modifier "+" ... Key
Modifier = ctrl | alt | shift | meta              (also control, option, cmd, command)
Key      = a ... z | 0 ... 9 | - = [ ] \ ; ' , . / `
         | f1 ... f12 | enter | escape | tab | space | backspace | delete | insert
         | home | end | pageup | pagedown | up | down | left | right
```

- Case and the order of modifiers do not matter. Modifiers must match exactly: `ctrl+k` is not Ctrl+Alt+K.
- `meta` is ⌘ on a Mac and the Windows or Super key elsewhere. For Ctrl elsewhere and ⌘ on a Mac, list both (`ctrl+p`, `meta+p`).
- Letters and digits are matched by their place on the keyboard, so the keyboard layout and Shift do not change which key it is.
- A chord waits 1.5 s for its second key.
- While a terminal has the keyboard, its program gets every key except ⌘ keys and Ctrl+Shift with a letter. No chord starts there.

## What a file can never do

- Answer a request: a command named `request.*`, `approval.*`, `approve`, `allow` or `deny` refuses the file.
- Change `window.safe-mode` (the safe-mode key, Ctrl+Alt+Shift+S and ⌘⌥⇧S).
- Use a fixed key in any stroke: Tab, Shift+Tab, Esc, Enter, Space, the arrows; a terminal's copy, find and paste (Ctrl+Shift+C,
  Ctrl+Insert, Ctrl+Shift+F, Ctrl+Shift+V, Shift+Insert, ⌘C, ⌘F, ⌘V); editing keys (Ctrl or ⌘ with A, X, Z, and Ctrl+C, Ctrl+V,
  Ctrl+Y, Ctrl+Shift+Z, ⌘⇧Z); the safe-mode key.

A command id the window does not have (a newer window's) is kept in the file and binds nothing. Two commands with the same keys, or
one whose key starts the other's chord, are accepted and shown as a conflict in Settings > Keyboard.

## Commands

| Command | Default keys |
| --- | --- |
| `palette.open` | Ctrl+K, ⌘K, Ctrl+Shift+K |
| `attention.next` (the next agent that needs the person) | N |
| `window.agents` (show or hide the agent list) | Ctrl+B, ⌘B, Ctrl+Shift+B |
| `pane.tab-1` … `pane.tab-9` | Alt+1 … Alt+9 |
| `side.place-1` … `side.place-5` | Alt+Shift+1 … Alt+Shift+5 |
| `window.safe-mode` (fixed) | Ctrl+Alt+Shift+S, ⌘⌥⇧S |
| `go.home`, `go.machines`, `go.settings`, `go.keyboard`, `go.audit` | none |
| `agent.start`, `terminal.new` | none |
| `tool.terminal`, `tool.changes`, `tool.files`, `tool.manager` | none |
| `machines.allow`, `window.lock` | none |
| `language.system`, `language.en`, `language.tr` | none |
| `theme.editor`, `plugins.open` | none |
| `sounds.toggle`, `agent.mute-sounds` | none |
| `layout.preset-1x1`, `layout.preset-2x1`, `layout.preset-1x2`, `layout.preset-2x2`, `layout.preset-3x2`, `layout.preset-focus` | none |
| `layout.maximize`, `layout.split-right`, `layout.split-down`, `layout.close-pane`, `layout.reset` | none |

Settings > Keyboard lists every command with its id and its keys now.

## Example

Ctrl+Shift+P for the command palette, Ctrl+K then 2 for the 2 × 2 layout, and N no longer jumps to the next agent.

```json agora-keys
{
  "version": 1,
  "bindings": [
    { "command": "palette.open", "keys": ["ctrl+shift+p", "meta+shift+p"] },
    { "command": "layout.preset-2x2", "keys": ["ctrl+k 2"] },
    { "command": "attention.next", "keys": [] }
  ]
}
```
