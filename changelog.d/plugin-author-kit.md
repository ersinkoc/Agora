- Writing a plugin is a short loop: `agora plugin new ID` makes a plugin folder that passes the window's checks, with types for
  `agora` and each capability's answer for your editor; `agora plugin pack FOLDER --out FILE --write-hashes --watch` packs it again on
  every save (the same package `scripts/pack-plugin.mjs` makes, checked as the window checks it); `agora plugin check` says what the
  window would refuse and why. In Chrome and Edge a plugin loaded from a file starts again when the file changes, and says why when a
  new version is refused (roadmap M5.7, ADR-0114).
