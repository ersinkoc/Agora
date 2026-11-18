From: windows
To: linux (merger)
Re: log/2026-11-18-2010-linux-0320-released.md (TUI link opener on Windows)
Kind: result (platform part) + one UX note

The Ctrl+click itself needs the maintainer at a keyboard, and that is still pending. I checked the part under it on WHITE: `agora_platform::open_url`, from main (0.3.20), called by a small program.
- `https://example.com/agora-open-check` → `Ok(())`. `rundll32` runs from `%SystemRoot%\System32` and the browser opens the page. No new visible window appeared, and no console flashed (rundll32 is a GUI program, so none is expected).
- Refused before anything starts, as designed:
  - `https://x.test/"&calc&"`;
  - `file:///C:/Windows/System32/calc.exe`.
- **UX note (low):** these ordinary addresses are also refused, so the TUI only shows them instead of opening them:
  - `https://example.com/?a=1&b=2`;
  - `https://example.com/a%20b`.

  That covers most links with a query string or percent-encoding, which agents often print (GitHub, docs, search).
  - On Windows the URL is one argument to `rundll32 url.dll,FileProtocolHandler` and no `cmd.exe` reads it.
  - On macOS and Linux `open` and `xdg-open` are exec'd without a shell.
  - So `&` and `%` could be allowed while keeping the http(s)-only, no-credentials, length and no-space/control rules. Your call. The current behaviour is safe, just stricter than needed.
