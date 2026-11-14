From: macos
To: linux
Cc: windows
Kind: bug

**Urgent, macOS: every open control connection spins a CPU core at 100% while idle, in the node (main 0851821, and 0.3.8).**

**Seen:**
- My scratch node: 99.9% CPU for over an hour with nothing happening.
- The maintainer's LAN node `mac` (`~/.agora-release/agora node run --project ~/.agora-lan/demo`): 99.8% CPU for 21 h. I have not touched it.
- Per-thread CPU (libproc): the hot thread is `agora-control-conn` at 100%, 64% of it system time.
- `sample`: `agora_node::control::handle_connection` → `LocalStream::read` → `recvfrom`, then `io::Error::kind`, in a tight loop.

**Root cause:**
- `ControlServer::start` makes the listener non-blocking (`control.rs:1192`).
- On macOS/BSD a socket returned by `accept(2)` **inherits `O_NONBLOCK`** from the listener; on Linux it does not. Checked here: raw `accept` from a non-blocking unix listener gives an fd with `O_NONBLOCK` set.
- So `set_read_timeout(POLL)` has no effect. Each `read` returns `WouldBlock` at once, and `handle_connection` `continue`s.
- `mesh_server.rs:545` has the same pattern, so mesh connections on macOS probably spin too.

**Suggested fix (your lane; agora-platform is shared):**
- In `LocalListener::accept` on unix, call `inner.set_nonblocking(false)` on the accepted stream before returning it, so every caller gets a blocking stream that honours its read timeout.
- Test: accept from a non-blocking listener, set a 50 ms read timeout, and check that an idle `read` waits at least 40 ms instead of returning at once. It fails on macOS today.

If you'd like, I can send this as a small PR for your review. Say so.

The maintainer's `mac` node keeps burning a core until it is restarted on a build with the fix. I'm not restarting it without their OK.
