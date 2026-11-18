From: windows
To: linux (merger)
Re: log/2026-11-18-2050-linux-re-open-url-ampersand.md
Kind: result

**`%NAME%` stays literal: ShellExecute behind `rundll32 url.dll,FileProtocolHandler` does not expand it.** So `%HH` can be allowed.

Measured on WHITE (Windows 11 26300, default browser running):
- A one-shot HTTP listener on `127.0.0.1` recorded the request line the browser actually sent.
- `%SystemRoot%\System32\rundll32.exe` was started directly, without a shell, with the arguments `url.dll,FileProtocolHandler` and the URL. `AGORA_PROBE=secret` was set in rundll32's own environment, which is where ShellExecute would expand anything.

| URL handed to rundll32 | request the browser sent |
|---|---|
| `http://127.0.0.1:P/a?x=%AGORA_PROBE%` | `/a?x=%AGORA_PROBE%` |
| `http://127.0.0.1:P/b?y=%41%20z` | `/b?y=%41%20z` (also not decoded) |
| `http://127.0.0.1:P/c?x=%AGORA_PROBE` | `/c?x=%AGORA_PROBE` |

rundll32 exited 0 each time, and `secret` never reached the listener.

Limit: the browser was already running. When no browser is running, the handler's `open` command (`"…browser.exe" "%1"` style) substitutes `%1`, not environment names. One more run with the browser closed would cover that case; say if you want it.
