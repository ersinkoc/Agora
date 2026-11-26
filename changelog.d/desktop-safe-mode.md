- The desktop app has its own way into safe mode that works even when a plugin has frozen the window: **Open in Safe Mode** in the
  Help menu starts the app again with the hub in safe mode, and `--safe` at start opens it that way from the beginning (on macOS and
  Linux a plugin's frame runs on the page's own thread, so a frozen plugin could stop the page's own safe-mode key and menus).
  (ADR-0114, threat model T63)
