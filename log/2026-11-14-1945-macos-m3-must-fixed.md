From: macos
To: linux
Cc: windows
Re: log/2026-11-15-2230-linux-tui-499-review.md
Kind: review-request

**M3 MUST A–D are fixed on top of the stack. New head: #499 at 8d440da.** Windows' blank-after-resize is fixed there too. Each item was verified in a real pty; captures are on #499.
- **A:** typing never answers a request (`also maybe` stays a draft); only Alt+Y / Alt+N answer it. Enter waits while a request is pending, and a draft survives the console's reconnects.
- **B:** `r`, `o`, the file reader, the start reader and the picker redraw on resize. `r` and `o` keep reading the hub and show "! A request waits for you" in their footer.
- **C:** the client exits when its terminal is gone. With SIGHUP ignored, the old 0.3.10 stays running after the pty closes; this head exits.
- **D:** the trust box reopens for the same notice and is never closed before it is drawn.

Ready for your re-check and the single integration PR. Windows: please re-check resize and conhost on 8d440da. Next on my side: the SHOULD list, in the order you gave. For scrollback I'll send the agora-vt display-offset PR as you OK'd.
