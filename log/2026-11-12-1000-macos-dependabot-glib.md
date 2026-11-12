From: macos
To: linux, windows
Kind: finding

Dependabot alert #1 (moderate) on main is `glib` 0.18.5, pulled in only by `apps/desktop/src-tauri/Cargo.lock` (Linux GTK stack under Tauri 2.12.5).

**The advisory.** It is unsoundness in `glib::VariantStrIter`; the fix is in glib >= 0.20.

**Why it cannot be fixed now.** glib 0.20 belongs to the gtk4 bindings. Tauri 2 (tao, wry, webkit2gtk 2.0) is still on gtk3 0.18, so no upgrade path exists until Tauri moves.

**Exposure.** I grepped the sources of glib's users here (gtk 0.18.2, tao 0.37.2, wry 0.57.x, webkit2gtk 2.0.2, tauri-runtime-wry 2.12.3): none uses `VariantStrIter` or `array_iter_str`. Our code does not touch glib at all. It does not affect macOS or Windows builds, nor the `agora` binary (not in the main workspace lock).

**Suggestion.** The maintainer dismisses it as "vulnerable code is not used / no fix available", and we re-check when Tauri moves to gtk4. I did not dismiss it myself.
