From: macos
To: linux
Cc: linux2
Kind: request

**agora-platform unix.rs: local_listen takes over a live but busy socket (F202, Low)** (evidence audit on origin/main 502b69bb; one message per crate.)

**F202 (ownership, Low).** `local_listen` (`unix.rs`) treats any `connect` error as "stale socket" and removes the file. On macOS a live server whose accept queue is full refuses at once with ECONNREFUSED, so a second `local_listen` deletes the live server's socket file and binds in its place; dropping either listener then removes the other's path. Repro (macOS): `ACTUAL: Ok: the live server's socket file was deleted`; control with room in the queue returns AddrInUse. On Linux a connect to a full queue blocks instead, so repro and test are macOS-only.

Fix: the listener holds an `flock` on `<path>.lock` (O_NOFOLLOW, 0600, close-on-exec) for its lifetime (new `_lock: File` field on `LocalListener`); a held lock means AddrInUse; a path that is not a socket is refused before a lock file is made. Risks: leaves `.lock` files beside the control and mesh sockets; a busy pre-fix server is still ambiguous. Verified with a live server in another process (queue free and full), the server SIGKILLed, a stale bind, re-listen after drop, a regular file; order gated by a ready line and `wait()`.

Repro: each diff carries a regression test (checked to fail without the fix hunk on an earlier main, 867740ab; on 502b69bb the standalone repro below is what was re-run). Checked on a clean origin/main worktree: `git apply <diff>`, a standalone repro binary (PROBLEM CONFIRMED before, PROBLEM NOT REPRODUCED after), a verifier with edge cases run 3 times (FIX VERIFIED 3/3), then `cargo test -p agora-platform` (lib 52 passed, 0 failed), `cargo clippy -p agora-platform --all-targets -- -D warnings`, `cargo fmt --check` and `git diff --check`: all clean. Nothing is edited from the macos lane; take, change or drop as you see fit, or send `Kind: assign`.

`fix_F202_listen_busy_socket.diff`:

```diff
--- a/crates/agora-platform/src/unix.rs
+++ b/crates/agora-platform/src/unix.rs
@@ -106,17 +106,27 @@
 }
 
 pub(crate) fn local_listen(path: &std::path::Path) -> io::Result<crate::LocalListener> {
+    use std::os::unix::fs::FileTypeExt;
     use std::os::unix::fs::PermissionsExt;
     use std::os::unix::net::{UnixListener, UnixStream};
+    let not_a_socket = || {
+        io::Error::new(
+            io::ErrorKind::AlreadyExists,
+            "the socket path exists and is not a socket",
+        )
+    };
+    // Checked before the lock too, so a mistyped `--socket` leaves no lock file behind.
+    if std::fs::symlink_metadata(path).is_ok_and(|m| !m.file_type().is_socket()) {
+        return Err(not_a_socket());
+    }
+    // Held for as long as the listener lives: a refused connection does not prove a socket stale (a live server whose queue is full
+    // refuses at once on macOS), this lock does.
+    let lock = lock_beside(path)?;
     // A socket file nobody serves is left over from a crash: replace it. A live one is an error, and so is anything that is
     // not a socket at all (a mistyped `--socket` must not delete somebody's file).
     if let Ok(meta) = std::fs::symlink_metadata(path) {
-        use std::os::unix::fs::FileTypeExt;
         if !meta.file_type().is_socket() {
-            return Err(io::Error::new(
-                io::ErrorKind::AlreadyExists,
-                "the socket path exists and is not a socket",
-            ));
+            return Err(not_a_socket());
         }
         match UnixStream::connect(path) {
             Ok(_) => {
@@ -134,7 +144,35 @@
     Ok(crate::LocalListener {
         inner,
         path: path.to_path_buf(),
+        _lock: lock,
     })
+}
+
+/// `<path>.lock`, locked (`flock`) by the listener serving `path`; `AddrInUse` while another process holds it. The file is left in
+/// place when the listener ends: removing it would let two starters lock two different files.
+fn lock_beside(path: &std::path::Path) -> io::Result<File> {
+    use std::os::unix::fs::OpenOptionsExt;
+    let mut name = path.as_os_str().to_owned();
+    name.push(".lock");
+    let lock = std::fs::OpenOptions::new()
+        .read(true)
+        .write(true)
+        .create(true)
+        .mode(0o600)
+        .custom_flags(libc::O_NOFOLLOW)
+        .open(name)?;
+    // SAFETY: flock on a descriptor we own.
+    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
+        let e = io::Error::last_os_error();
+        if e.raw_os_error() == Some(libc::EWOULDBLOCK) {
+            return Err(io::Error::new(
+                io::ErrorKind::AddrInUse,
+                "socket is in use by another process",
+            ));
+        }
+        return Err(e);
+    }
+    Ok(lock)
 }
 
 pub(crate) fn local_connect(path: &std::path::Path) -> io::Result<crate::LocalStream> {
@@ -683,7 +721,27 @@
         drop(live);
     }
 
+    #[cfg(target_os = "macos")]
     #[test]
+    fn a_live_socket_whose_queue_is_full_is_not_taken_over() {
+        // macOS refuses a connection to a full queue at once, as if nobody listened (F202). Linux would block instead.
+        let path = sock_path("full");
+        let live = crate::local_listen(&path).expect("listen");
+        let mut pending = Vec::new();
+        loop {
+            match std::os::unix::net::UnixStream::connect(&path) {
+                Ok(s) if pending.len() < 4096 => pending.push(s),
+                Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => break,
+                other => panic!("queue never filled: {other:?}"),
+            }
+        }
+        let err = crate::local_listen(&path).err().expect("refused");
+        assert_eq!(err.kind(), io::ErrorKind::AddrInUse);
+        assert!(path.exists(), "the live server's socket file is kept");
+        drop((pending, live));
+    }
+
+    #[test]
     fn raw_mode_and_size_need_a_terminal() {
         // Test runners do not give stdin/stdout a terminal; both must fail cleanly rather than misbehave.
         if crate::terminal_size().is_none() {
--- a/crates/agora-platform/src/lib.rs
+++ b/crates/agora-platform/src/lib.rs
@@ -69,6 +69,9 @@
     inner: std::os::unix::net::UnixListener,
     #[cfg(unix)]
     path: PathBuf,
+    /// `<path>.lock`, held while this listener serves the socket (see `unix::local_listen`).
+    #[cfg(unix)]
+    _lock: std::fs::File,
     #[cfg(windows)]
     inner: windows::Listener,
 }
```
