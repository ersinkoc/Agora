# An Agora plugin

A plugin adds panels to the Agora window. Each panel runs in a sandboxed frame: it can see only what the person allows, it reaches no
network, and it cannot act. The full description is in the Agora repository, `docs/ux/plugins.md`.

## The files

- `agora-plugin.json`, the manifest: the plugin's id, name, version, author and licence, its panels, the capabilities it asks for, its
  entry script and every other file with its SHA-256. Put your own name in `author` and your licence in `licence` before you share it.
- `panel.js`, the entry script: the one script that runs. Bundle everything into it.
- `panel.css`: a stylesheet, applied to the frame.
- `agora-plugin.d.ts`: types for `agora` and for each capability's answer, for your editor (`// @ts-check` at the top of `panel.js`).
  It is never packed.
- `README.md`: this file. It is never packed.

The folder is flat (no sub-folders). Scripts, stylesheets, images, fonts, JSON and text files can be in it; every one of them is listed
in the manifest. The package is at most 2 MiB.

## Try it in the window

1. From the folder that holds this one, pack it and keep packing it on every save (the package goes beside the folder, never in it):

   ```
   agora plugin pack my-plugin --out my-plugin.json --write-hashes --watch
   ```

   `--write-hashes` writes each file's SHA-256 into the manifest first. Without `--watch` it packs once.
2. In the window: the command palette, **Plugins**, **Load from a file…**, choose `my-plugin.json`, allow what it asks to see, and
   **Add plugin**. Then show its panel in a pane (the pane's **Show…** menu).
3. Edit `panel.js` and save. In a browser that can watch a file (Chrome, Edge), the panel starts again with the new package. Elsewhere,
   load the file again.

`agora plugin check my-plugin` (or `agora plugin check my-plugin.json`) runs the window's own checks and says what it would refuse.

## Rules worth knowing

- Draw other people's text (agent names, task titles, chat) with `textContent`, never `innerHTML`.
- `agora.read(cap)` and `agora.watch(cap, onData, onError)` fail with an Error whose message is `not-granted`, `unknown-capability`,
  `unavailable` or `too-many`. Show something useful then.
- A capability added in a new version is not granted until the person looks at the plugin again.
