# ADR-0079 on Windows: the measured premises, measured again

Date: 2026-11-21

Machine: WHITE, Windows 11 Pro 10.0.26300. Asked for by the merger for roadmap M2.6 (the Windows holder). This is a measurement only:
no code changed.

ADR-0079's section "Measured on Windows 11 (WHITE, 2026-11-15)" rests on throwaway programs run on 2026-11-15 and 2026-11-16. The same
programs were run again on the same machine, which has since taken the 0.3.2x and 0.3.3x releases and Windows updates. Every premise
holds unchanged.

## A holder keeps a pseudoconsole while its reader dies (the spike)

A *holder* created the pipes and a pseudoconsole (`CreatePseudoConsole` returned 0) and started a child that prints a counter every
second and echoes what is typed. *Readers*, standing in for the node, got the holder's pipe ends with `DuplicateHandle`.

- Reader 1 attached, typed a line, got its echo, and read ticks 1 to 5. It was then killed (`TerminateProcess`): the child stayed alive.
- With no reader, the holder read the output itself and buffered it: 41 bytes, ticks 6 to 10. The child never blocked on a full pipe.
- Reader 2 attached, typed a line, got its echo, and read on from tick 12. It was then killed too, and the child was still alive at the end.
- Tick 11 went to the holder's own read that was still in flight when reader 2 attached, the same as on 2026-11-15. A handover has to end
  the holder's read first and hand its buffer over (ADR-0079 decision 4).

## A pending read on an anonymous pipe can be ended from another thread

A `ReadFile` waiting on a `CreatePipe` pipe (synchronous, anonymous):

- `CancelIoEx(handle)` ended it in 1 ms (error 995, the operation was aborted). The next read returned the next data; no byte was lost.
- `CancelSynchronousIo(thread)` ended it in 0 ms, with the same result.

## Task Scheduler: breakaway and `Stop-ScheduledTask`

A temporary scheduled task (created for this run and deleted afterwards; the machine's `agora-node` task was not touched) ran a program
that starts two long-lived children and records what Windows allowed.

- The task's process is in a job (`IsProcessInJob`), and that job forbids breakaway: `CREATE_BREAKAWAY_FROM_JOB` from the task's process
  fails with error 5 (access denied).
- A process that first joins a kill-on-close job of its own that allows breakaway (`JOB_OBJECT_LIMIT_BREAKAWAY_OK`, as the supervisor's
  `spawn_bound` does) and then starts a child with `CREATE_BREAKAWAY_FROM_JOB` succeeds.
- `Stop-ScheduledTask` ended only the task's own process. 30 seconds later, the child started plainly in the task's job and the child that
  broke away from the inner job were both alive.

So a holder that the supervisor starts with breakaway outlives both the supervisor's job and a stop of the task, as ADR-0079 says.

## The code the premises rely on (origin/main)

- `spawn_bound` sets `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_BREAKAWAY_OK` (`crates/agora-platform/src/winproc.rs`).
- `spawn_detached_strict` exists. It is the start without the fallback into the job that decision 2 requires for the holder: a refused
  breakaway is an error, never a start inside the supervisor's kill-on-close job.

## Limits

- The holder itself is not built on Windows (`docs/capabilities.md`: primitives only). These runs check the operating system behaviour
  the design depends on, not an Agora holder.
- One machine and one Windows build. Signing out still ends a logon task's processes; this run did not test that.
