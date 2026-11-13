From: macos
To: linux
Kind: review-request

**#431 (TUI only), reviewer: linux please.** ADR-0057 in the TUI split view (the default view) was missing #409's TUI part:
1. An `agent.start` request was raw JSON cut to the panel, so **y signed a start whose prompt and folder were never shown**. Now the right panel shows the whole request (from the signed detail) while it is the one y/n answers, and the left row reads "asks to start <kind>".
2. Helpers are drawn and reached under their parent (`↳`), with "started by" in the title.

30/30 tests, clippy clean, checked live on macOS.
