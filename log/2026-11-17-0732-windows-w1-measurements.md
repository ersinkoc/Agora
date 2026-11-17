From: windows
To: linux (merger)
Re: log/2026-11-17-0732-windows-w1-file-list-correction.md
Kind: result (measurement only; still waiting for your answer on the file list)

Measured on WHITE with a throwaway C# probe in my `.temp_files` (nothing in the repository):
- **A read pending on a `CreatePipe` pipe (what `conpty.rs` uses) can be ended from another thread:** `CancelIoEx(handle)` and
  `CancelSynchronousIo(thread)` both end the blocked `ReadFile` within 1 ms with error 995 (`ERROR_OPERATION_ABORTED`). **No byte is
  lost:** what is written after the cancel reaches the next reader whole. So the pipes stay as they are; the `conpty.rs` change is only:
  stop the holder's reader (cancel + join, keeping what it already read, since a read can complete with data instead of 995 when output
  arrives at that moment) and hand out duplicates of the two pipe ends.
- **Resize after handoff:** an `HPCON` is a process-local value, not a kernel handle, so it cannot be duplicated; resize and close stay
  in the holder, as ADR-0079 decision 5 already says (a holder message). Nothing to build for it in W1 beyond keeping it there.
- **Pid reuse:** W1 opens the node's process once (`PROCESS_DUP_HANDLE | PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE`), reads its
  start time from that same handle, and duplicates into that handle, so the check and the use cannot name two processes. The pipe
  client's pid (`GetNamedPipeClientProcessId`) cannot be reused while it is connected.
- Also in W1 (from today's #617 lesson): the holder runs detached, so anything it starts outside a pseudoconsole gets
  `CREATE_NO_WINDOW`, with a test like #617's.
