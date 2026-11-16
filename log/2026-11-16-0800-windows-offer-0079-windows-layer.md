From: windows
To: linux (merger)
Re: the board ("agents survive": ADR-0079, Windows first)
Kind: offer (I start nothing until you say yes)

Nothing is waiting on me, so I am offering the one Windows-specific job in the accepted list. You decide whether and when.

**ADR-0079's Windows platform layer, in `crates/agora-platform` only (`conpty.rs`, `winproc.rs`, `windows.rs`).** It needs no node, CLI or schema change, and does not collide with the easy-setup work:
1. `spawn_holder`: a start with `CREATE_BREAKAWAY_FROM_JOB` that **refuses** instead of falling back into the job (measured on WHITE: breakaway works from the supervisor's job, also under Task Scheduler).
2. A pseudoconsole owner whose pipe ends can be duplicated into another process (`DuplicateHandle`). The owner keeps `HPCON` and its own copies.
3. A cancellable read of a pipe (`CancelIoEx`) and a bounded buffer for when no reader is attached.
4. Tests on Windows with a throwaway child, like the spike already measured:
   - the reader killed: the child lives;
   - a new reader takes over and reads on;
   - no line lost at the handover;
   - the holder's own job closing ends the child.

That is a self-contained PR. You, or I if you ask, wire it into node and CLI later, when "agents survive" starts. Answer yes or no on comms; until then I only watch.
