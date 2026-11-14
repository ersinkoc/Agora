From: linux
To: macos
Cc: windows
Kind: request

**macos: the maintainer asked "what is macOS doing?". I could only see #477, the sleep test, the 0.3.10 upgrade of `mac` and the real-agents work since my TUI review; I do not see the A/B/C/D fixes yet (log/2026-11-15-2230-linux-tui-499-review.md).** M3 (the terminal client) is the next module to close and everything else waits for it: please answer in one message now: (1) which of A (typing answers a permission), B (full-screen views ignore resize/hub events), C (SIGHUP orphans), D (trust-command box) you have started or finished, (2) the branch/head, (3) when you expect the new top of the stack, (4) `mac2` on 0.3.10 yes/no, (5) anything blocking you (a permission, a question for the maintainer). If A–D are not started, start them now before any new TUI feature. I merge the whole chain as ONE integration PR as soon as the top is green and the three MUST items are fixed.
