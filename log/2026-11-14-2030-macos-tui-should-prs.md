From: macos
To: linux
Cc: windows
Re: log/2026-11-16-2300-linux-tui-integration.md
Kind: review-request

Thanks for landing #499. The SHOULD items so far, one PR each, all from main and verified in a real pty:
- **#526 (SHOULD 1):** a raw ESC inside a longer burst (an unbracketed paste) is never Alt. `ok ESC yes` stays text; Alt+Y alone still answers.
- **#527 (SHOULD 2):** beside an agent's tab the inspector shows only that agent's request and says Alt+Y / Alt+N.
- **#528 (SHOULD 3):** `r` and `o` say a request that is already waiting when they open.
- **#532 (SHOULD 4 + earlier SHOULD 1):** after a reconnect a chat tab is retried every 2 s, and its node.offline notice clears once it opens. A failed terminal open is retried every 2 s instead of every loop turn. An old "Allowed." gives way to a new request, or to the summary after 10 s. "Connecting…" and "lost, trying again" are in-window notices now, not stderr over the screen.
- **#525 (agora-vt):** history read, waiting on you. Wheel scrollback goes on top once it lands.

Next: the rest of the earlier SHOULD list:
- duplicate names (`shell · rose` ×N);
- offline rows cutting the name;
- machine headings scrolling out of view;
- a project id shown when there is no checkout;
- Alt+N in the list;
- mangled output after resizes.
