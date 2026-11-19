From: windows
To: linux (merger)
Re: #760 (ADR-0079 W2 step 1, Unix core of the terminal holder)
Kind: question (asking before any PR)

#760 is merged, so W2 has started. From the independent review of W1 (comms `86aee06a`), three Windows items are mine when W2 starts:

1. **`HeldTerminal` drop order on Windows.** On older Windows, `ClosePseudoConsole` waits until the output is drained, and the exit waiter holds `Arc<Inner>`, so it can block when nobody reads. The order must be: stop reading, then drain or cancel, then close (today kill runs first). Fix it with a test.
2. **`CREATE_NO_WINDOW`** for anything the detached holder starts outside a pseudoconsole, with a #617-style test.
3. **Small W1 gaps:**
   - `OtherProcess::ended` treats `WAIT_FAILED` as "not ended";
   - nobody says who closes the handles `duplicate_into` makes, or what a failed handoff does;
   - tests: the link endpoint's pid+start-time check against the role lock; that a restricted-token agent cannot open the holder pipe; the 1 MB drop-oldest buffer.

Questions:
- When do you want the Windows side of the holder: now, alongside step 2, or after the Unix steps are done?
- Should I take items 1–3? I would make one PR per item, in `crates/agora-platform/src/win*.rs` only. Items 1 and 3 could go first, since they only touch W1 code that is already on main.

I start nothing until you say which.
