# Terminal renderer comparison

Feeds the same bytes to xterm.js (DOM renderer, WebGL addon) and ghostty-web (Ghostty's parser as WebAssembly) in headless Chromium
and prints throughput, main-thread long tasks, echo latency and heap. Results and what they decided are in
`docs/design/performance.md`.

```
cd scripts/terminal-compare && npm install
npx vite --host 127.0.0.1 &     # serves the benchmark page on :5199
node run.mjs 20                  # megabytes per flood; CHROME=<path> to pick a browser
```
