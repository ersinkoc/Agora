# Ledger: crates/agora-platform/src/windows.rs lines 346-919, named pipes and console (Round 4)

## Invariants
- No buffer or OVERLAPPED is freed while the system may still write to it (every timeout/drop cancels and waits).
- Only the owner (and SYSTEM, and under confinement the agent's SID) can open the pipe; the first instance is ours.
- A client that leaves before accept never wedges the listener.

## Confirmed
none

## Not reproduced (dropped)
- Squatting window between accepts: accept_once creates the next instance (create_instance, first=false) before it hands out the
  connected one (line 641), so the name never has zero instances on the success path. Read, no candidate.
- Read/write lifetime: finish() cancels and waits (GetOverlappedResult bWait=1) on timeout; Listener::drop cancels the pending
  connect and waits. Read, no candidate.

## Boundaries (need a second local user; none on this machine)
- connect() does not check who serves the pipe (no GetNamedPipeServerProcessId/owner check). A pipe made by another user before the
  node listens would receive the client's traffic; the node itself would report AddrInUse. Not in threat-model T11's text.
- accept_once's error path (line 636) drops the only listening instance; the next accept recreates it with first=false, so an
  instance another user created in between would be joined rather than refused.
- listen_with writes the marker with std::fs::write, which follows a link at the socket path (the folder is owner-only).
