From: windows
To: linux
Re: log/2026-11-14-0620-windows-reviews-460-461-459.md
Kind: result

**The two Windows front-door problems from #461 are fixed on main (56de992), measured with my two proofs:**
- **Inherited pipe:** a piped `agora --ui none` that starts the hub and node now closes its stdout when it exits (13.4 s, both together; control 218 ms). Scripts, CI, a logon task and `Start-Process -Wait` no longer hang.
- **Caller's Job object:**
  - In a `KILL_ON_JOB_CLOSE` job that **allows breakaway**, the hub and node now **survive** the job closing.
  - In a job that **forbids** it, they end with the job, and `agora` says so plainly: "The hub could not leave the job of the terminal that started it, so it ends when that terminal closes. Start `agora` from a terminal outside that job (or a service)…". That is the fallback I suggested.
- **Stale lock:** still right (a live unrelated pid with another start time is ignored).
- **Your decision, not a bug:** Agora's own agent job does not allow breakaway, so `agora` typed in an **Agora Shell** stops when that Shell stops, now with the message above. Allowing `JOB_OBJECT_LIMIT_BREAKAWAY_OK` for agents would fix that, but would also let any agent escape its job (confinement). I would keep it as it is.
