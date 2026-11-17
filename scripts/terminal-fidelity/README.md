# Terminal fidelity checks

Real-browser checks of the window's terminal (`apps/web/src/terminal/`), the evidence behind `docs/ux/terminal-fidelity.md`. The
real `TerminalView` runs on a fake source in headless Chromium; one group of checks puts a real program on a real PTY
(`pty-run.py` and `mouse-echo.py`, which switches on mouse reporting and prints what it receives).

```
cd apps/web && npm ci && npm run gen
LAB_PORT=5231 npx vite --config ../../scripts/terminal-fidelity/vite.config.mjs &     # serves http://127.0.0.1:5231/lab
cd ../.. && PW_DIR=<dir with node_modules/playwright-core> LAB_PORT=5231 node scripts/terminal-fidelity/run.mjs [screenshots-dir]
```

`CHROME=<path>` picks the browser; it needs `python3` for the PTY. The script exits non-zero when a check fails. It is not part of
CI (it needs a browser).
