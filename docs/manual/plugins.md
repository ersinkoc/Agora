# Plugins: write a panel for the web window

A **plugin** adds one to four panels to the web window (and the desktop app). Each panel is shown in a pane like a built-in one,
inside a sandboxed frame. A plugin reads what the person allowed and draws in its pane. It never acts: it cannot answer a request,
send a prompt, start, stop or type into anything, change a setting or a theme, or sign anything. It connects to nothing: no fetch,
WebSocket, WebRTC, or file from another origin. Everything it runs is in its package.

## Where it goes

The person opens **Plugins** in the command palette (Ctrl K), chooses **Load from a file…**, picks the package, allows what it may
see, and chooses **Add plugin**. It is kept in that browser only and marked **development**; at most 4 are loaded. Then a pane's
**Show…** menu offers its panels. `?safe` in the address, or Ctrl+Alt+Shift+S (⌘⌥⇧S on a Mac), opens the window without plugins.

## The package

One JSON file of at most 2 MiB: the manifest and every file, each as base64.

```json
{ "agora-plugin-bundle": 1, "files": { "agora-plugin.json": "BASE64", "panel.js": "BASE64" } }
```

- File names are plain names: no folder, no `..`, no leading dot, at most 100 characters. At most 64 files.
- Allowed kinds, by extension: `js`, `css`, `json`, `txt`, `png`, `jpg`, `jpeg`, `gif`, `webp`, `svg`, `woff`, `woff2`.
- Every file but the manifest is listed in the manifest's `files` with its SHA-256 (lower-case hex). A file not listed, a listed
  file missing, or a wrong hash refuses the whole package.

## The manifest, `agora-plugin.json`

| Field | Rule |
| --- | --- |
| `id` | lower-case letters, digits and `-`, at most 40 characters |
| `name` | one line of text, at most 60 characters |
| `version`, `min_agora` | `x.y.z`; a window older than `min_agora` refuses the plugin |
| `author` | one line of text, at most 100 characters |
| `licence` | required, at most 60 characters |
| `panels` | one to four `{ "id", "kind", "name" }`: `id` lower-case, at most 20 characters; `kind` is `global` (the default) or `tool` (bound to an agent's folder); `name` optional |
| `capabilities` | what it asks to read, from the list below; an unknown name refuses the package |
| `entry` | the one script that runs: a `.js` file listed in `files` |
| `files` | every other file with its SHA-256 |

No other field is allowed, anywhere in the manifest.

## Capabilities

Each is read-only. The person allows each one; none is allowed by default.

| Capability | Answer |
| --- | --- |
| `agents.read` | each agent's id, name, kind, state, machine, project, and whether it is closed |
| `usage.read` | each agent's tokens, context percent and time since it started; the total tokens |
| `tasks.read` | each project's shared tasks: id, title, status, owner |
| `changes.read` | the bound agent's changed files with their diff lines |
| `chat.read` | the bound agent's newest 200 entries, with secrets hidden |
| `theme.read` | the scheme and the colour tokens of the window's theme |
| `pane.context` | the pane's id and kind, what it follows, and the bound agent's id and name |

## The script

In the frame, the entry script gets `agora`:

- `agora.read(cap)`: a Promise of one answer; it is rejected with an `Error` whose message is `not-granted`,
  `unknown-capability`, `unavailable` or `too-many`.
- `agora.watch(cap, onData, onError)`: an answer now and again on each change (at most 16 at once). It returns a function that
  stops it.
- `agora.file(name)`: a `blob:` address of one of the package's images or fonts.
- `agora.granted`, `agora.plugin`, `agora.panel`, `agora.scheme`.

Draw with `textContent`, never `innerHTML`: agent names are other people's text. Only the entry script runs, so bundle all code
into it. Messages over 20 a second are dropped, and many of them stop the frame; a malformed message stops it at once.

## Packing

Put the files in one flat folder. Run this there: it writes each file's hash into the manifest, then the package,
`agents-plugin.json`. List every file but the manifest in `names`.

```sh
python3 - <<'EOF'
import base64, hashlib, json, pathlib
names = ["panel.js"]
read = lambda n: pathlib.Path(n).read_bytes()
manifest = json.loads(read("agora-plugin.json"))
manifest["files"] = {n: hashlib.sha256(read(n)).hexdigest() for n in names}
pathlib.Path("agora-plugin.json").write_text(json.dumps(manifest, indent=2) + "\n")
files = {n: base64.b64encode(read(n)).decode() for n in ["agora-plugin.json", *names]}
pathlib.Path("agents-plugin.json").write_text(json.dumps({"agora-plugin-bundle": 1, "files": files}))
EOF
```

In a checkout of Agora's source, `node scripts/pack-plugin.mjs FOLDER --out FILE --write-hashes` does the same and checks the
folder as the window would.

## Example

A panel that lists the agents and their states. `panel.js`:

```js panel.js
var list = document.createElement('ul');
document.body.append(list);
agora.watch('agents.read', function (data) {
  list.replaceChildren();
  data.agents.forEach(function (a) {
    var li = document.createElement('li');
    li.textContent = a.name + ': ' + a.state;
    list.append(li);
  });
}, function (e) {
  list.textContent = 'Not allowed: ' + e.message;
});
```

`agora-plugin.json` (the hash is that of `panel.js` above):

```json agora-plugin
{
  "id": "agents-list",
  "name": "Agents",
  "version": "0.1.0",
  "author": "Me",
  "licence": "MIT",
  "min_agora": "0.3.0",
  "panels": [{ "id": "list", "kind": "global" }],
  "capabilities": ["agents.read"],
  "entry": "panel.js",
  "files": { "panel.js": "408cd8c21c7954f58d19770d608c7848234aebe2769bd9e652f34599e8eef89c" }
}
```
