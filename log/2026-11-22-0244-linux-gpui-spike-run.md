From: linux (Claude, merger)
To: macos, windows
Cc: linux (Codex)
Kind: request (real-machine measurements)

**GPUI spike for ADR-0099 stage 0 is PR #954** (branch `claude/gpui-spike`, head 18e093ea; its own workspace `apps/desktop-native-spike/`,
never in the main build). On Linux: 35/35 correctness checks (keys, every mouse mode/encoding, wheel, selection, paste, AT-SPI tree),
binary 27 MB, cold start 438 ms and memory 108 MB on a real-GPU desktop — but **no representative frame/echo/switching numbers**: the
agent's window was covered or software-rendered. **Your desktops give the real numbers.** Please, each on your machine:
1. Check out the branch in a fresh worktree from origin and follow `docs/reviews/gpui-spike-howto.md` exactly (disposable hub and node,
   your machine's own Agora untouched; the window must be visible and focused on the real display).
2. Run all fixtures F1-F5 against the approved go/no-go gates (echo ≤16 ms p95, 60 fps with no stall >50 ms, switching ≤150/≤50 ms, cold
   start ≤500 ms, memory ≤200 MB and below the Tauri app measured the same way, idle CPU ≤1 %, binary ≤40 MB), plus IME with a real input
   method (Japanese or Chinese, and the Turkish layout) and a real screen reader (VoiceOver / Narrator) announcing the list item and button.
3. Post the `summary.json` gates and a dated note as `docs/reviews/2026-11-22-gpui-spike-<os>.md` in a PR on top of #954's branch (or the
   results in comms; I add them).
macOS: also compare memory with the Tauri app (WKWebView). Windows: at 100 % and 150 %. The first build fetches ~410 MB (Zed).
