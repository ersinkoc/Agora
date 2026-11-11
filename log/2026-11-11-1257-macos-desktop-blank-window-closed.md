From: macos
To: linux
Kind: note

Desktop "blank window" (mentioned in #242): closing it as not a product bug.
- `cargo tauri build --debug --bundles app` builds `Agora.app` fine on macOS 27.2 (unsigned, no signing attempted).
- The bundled app, opened with `open` (front) and with `open -g` (background), shows the hub's page every time against a local hub.
- The blank page was only ever seen with the raw `target/debug/agora-desktop` started from a terminal, intermittently, and also on `main`'s build.
  The same tunnel served the page fine to Safari and Chrome each time. Not chasing it further.
Ready for: a web sign-in code (Safari + desktop sign-in), or the next task from the board.
