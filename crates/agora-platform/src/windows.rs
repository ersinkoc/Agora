//! Windows backend of the local control channel (ADR-0045): a named pipe with an owner-only security descriptor, behind the same
//! functions and types the Unix socket backend implements. The pipe is opened for overlapped I/O, so a read that waits in one
//! thread never blocks a write from another thread on the same pipe, and a read can time out or be cancelled.
//!
//! Every operation waits for its own completion (or cancels and waits for the cancellation) before it returns, so the buffer and the
//! `OVERLAPPED` it passed to the system are never used after they are gone.

use std::io;
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_ACCESS_DENIED, ERROR_BROKEN_PIPE,
    ERROR_FILE_NOT_FOUND, ERROR_IO_PENDING, ERROR_NO_DATA, ERROR_OPERATION_ABORTED,
    ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, ERROR_PIPE_NOT_CONNECTED, GENERIC_READ, GENERIC_WRITE,
    HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED,
    OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
};
use windows_sys::Win32::System::Console::{
    GetConsoleMode, GetConsoleScreenBufferInfo, GetStdHandle, SetConsoleMode,
    CONSOLE_SCREEN_BUFFER_INFO, ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT,
    ENABLE_PROCESSED_OUTPUT, ENABLE_VIRTUAL_TERMINAL_INPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
    STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, WaitNamedPipeW, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{CreateEventW, WaitForSingleObject, INFINITE};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

/// Only the owner of the pipe and the system may open it; nobody else on the machine, and nobody over the network.
const OWNER_ONLY: &str = "D:P(A;;GA;;;OW)(A;;GA;;;SY)";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn os_error(code: u32) -> io::Error {
    io::Error::from_raw_os_error(code as i32)
}

/// An owned system handle.
struct Handle(HANDLE);

// SAFETY: a handle is an opaque id the system lets any thread use; I/O on it is serialised by the system.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle is owned and closed once.
        unsafe { CloseHandle(self.0) };
    }
}

/// A manual-reset event for one operation.
struct Event(Handle);

impl Event {
    fn new() -> io::Result<Self> {
        // SAFETY: plain call; null attributes and name.
        let h = unsafe { CreateEventW(null(), 1, 0, null()) };
        if h.is_null() {
            return Err(os_error(unsafe { GetLastError() }));
        }
        Ok(Self(Handle(h)))
    }
}

fn overlapped(event: &Event) -> OVERLAPPED {
    // SAFETY: an all-zero OVERLAPPED is the documented initial state.
    let mut ov: OVERLAPPED = unsafe { std::mem::zeroed() };
    ov.hEvent = event.0 .0;
    ov
}

/// The pipe name for a socket path: the file name for people, a hash of the whole path so two sockets of the same name in different
/// folders do not meet.
fn pipe_name(path: &Path) -> Vec<u16> {
    let full = path.to_string_lossy().to_lowercase();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in full.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    let stem: String = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(32)
        .collect();
    wide(&format!(r"\\.\pipe\agora-{stem}-{h:016x}"))
}

/// A security descriptor made from [`OWNER_ONLY`], freed on drop.
struct Descriptor(PSECURITY_DESCRIPTOR);

unsafe impl Send for Descriptor {}
unsafe impl Sync for Descriptor {}

impl Descriptor {
    fn owner_only() -> io::Result<Self> {
        Self::from_sddl(OWNER_ONLY)
    }

    fn from_sddl(sddl: &str) -> io::Result<Self> {
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: a valid NUL-terminated string and a valid out pointer.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide(sddl).as_ptr(),
                SDDL_REVISION_1,
                &mut sd,
                null_mut(),
            )
        };
        if ok == 0 {
            return Err(os_error(unsafe { GetLastError() }));
        }
        Ok(Self(sd))
    }
}

impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: allocated by the call above with LocalAlloc.
        unsafe { LocalFree(self.0 as _) };
    }
}

fn create_instance(name: &[u16], sd: &Descriptor, first: bool) -> io::Result<Handle> {
    let attrs = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.0,
        bInheritHandle: 0,
    };
    let mut open = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED;
    if first {
        // The first instance must be ours: nobody else may already be serving (or squatting on) this name.
        open |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    // SAFETY: valid name and attributes for the duration of the call.
    let h = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            open,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            64 * 1024,
            64 * 1024,
            0,
            &attrs,
        )
    };
    if h == INVALID_HANDLE_VALUE {
        let e = unsafe { GetLastError() };
        return Err(if e == ERROR_ACCESS_DENIED || e == ERROR_PIPE_BUSY {
            io::Error::new(
                io::ErrorKind::AddrInUse,
                "another process serves this control channel",
            )
        } else {
            os_error(e)
        });
    }
    Ok(Handle(h))
}

struct Pending {
    event: Event,
    ov: Box<OVERLAPPED>,
}

// SAFETY: the pending operation is only touched under the listener's mutex; the event handle is usable from any thread.
unsafe impl Send for Pending {}

struct ListenState {
    instance: Option<Handle>,
    pending: Option<Pending>,
}

pub struct Listener {
    /// An empty file at the socket path while this listens, as the Unix backend has a socket file: callers and tests look at the path
    /// to see that a node is up. The pipe is what is served; a file left by a crash is harmless.
    marker: std::path::PathBuf,
    name: Vec<u16>,
    sd: Descriptor,
    state: Mutex<ListenState>,
    nonblocking: AtomicBool,
}

pub fn listen(path: &Path) -> io::Result<Listener> {
    listen_with(path, Descriptor::owner_only()?)
}

/// The listening end of one agent's own channel (its mesh endpoint). Under confinement (ADR-0050 Amendment B) the agent runs with a
/// restricted token at Low integrity: besides the owner and SYSTEM, the pipe names exactly that agent's SID (`identity`), which only
/// its token holds among the restricting SIDs, so no other confined agent passes the second access check; and it carries a Low label
/// so that no-write-up lets the Low agent write to it. Without an identity it is owner-only like [`listen`].
pub fn listen_agent(path: &Path, identity: Option<&str>) -> io::Result<Listener> {
    match identity {
        Some(sid)
            if sid.starts_with("S-1-")
                && sid[4..].chars().all(|c| c.is_ascii_digit() || c == '-') =>
        {
            // A restricted token passes two checks. The user's own entry carries the first one (an `OWNER RIGHTS` entry does not
            // for a restricted token: measured), the agent's entry the second; any other confined agent fails the second.
            let user = crate::winconfine::current_user_sid_text()?;
            listen_with(
                path,
                Descriptor::from_sddl(&format!(
                    "{OWNER_ONLY}(A;;GRGW;;;{user})(A;;GRGW;;;{sid})S:(ML;;NW;;;LW)"
                ))?,
            )
        }
        Some(_) => Err(io::Error::from(io::ErrorKind::InvalidInput)),
        None => listen(path),
    }
}

/// `\\.\pipe\...` for a socket path, as text (see [`crate::local_endpoint_name`]).
pub(crate) fn pipe_name_text(path: &Path) -> String {
    let w = pipe_name(path);
    String::from_utf16_lossy(&w[..w.len().saturating_sub(1)])
}

fn listen_with(path: &Path, sd: Descriptor) -> io::Result<Listener> {
    let name = pipe_name(path);
    let first = create_instance(&name, &sd, true)?;
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(path, b"")?;
    Ok(Listener {
        marker: path.to_path_buf(),
        name,
        sd,
        state: Mutex::new(ListenState {
            instance: Some(first),
            pending: None,
        }),
        nonblocking: AtomicBool::new(false),
    })
}

impl Listener {
    pub fn set_nonblocking(&self, nonblocking: bool) {
        self.nonblocking.store(nonblocking, Ordering::Release);
    }

    pub fn accept(&self) -> io::Result<Stream> {
        let mut st = self
            .state
            .lock()
            .map_err(|_| io::Error::other("poisoned"))?;
        if st.instance.is_none() {
            st.instance = Some(create_instance(&self.name, &self.sd, false)?);
        }
        let inst = st.instance.as_ref().map_or(null_mut(), |h| h.0);
        let mut connected = false;
        if st.pending.is_none() {
            let event = Event::new()?;
            let mut ov = Box::new(overlapped(&event));
            // SAFETY: `inst` is a live pipe instance opened for overlapped I/O; `ov` lives in `pending` until it completes.
            let ok = unsafe { ConnectNamedPipe(inst, &mut *ov) };
            if ok != 0 {
                connected = true;
            } else {
                match unsafe { GetLastError() } {
                    ERROR_PIPE_CONNECTED => connected = true,
                    ERROR_IO_PENDING => st.pending = Some(Pending { event, ov }),
                    e => return Err(os_error(e)),
                }
            }
        }
        if !connected {
            let Some(p) = st.pending.as_ref() else {
                return Err(io::Error::other("no pending connection"));
            };
            let ms = if self.nonblocking.load(Ordering::Acquire) {
                0
            } else {
                INFINITE
            };
            // SAFETY: a live event handle.
            let w = unsafe { WaitForSingleObject(p.event.0 .0, ms) };
            if w == WAIT_TIMEOUT {
                return Err(io::Error::from(io::ErrorKind::WouldBlock));
            }
            if w != WAIT_OBJECT_0 {
                return Err(os_error(unsafe { GetLastError() }));
            }
            let mut n = 0u32;
            // SAFETY: the operation completed (the event is signalled); `ov` is the one it was started with.
            let ok = unsafe { GetOverlappedResult(inst, &*p.ov, &mut n, 0) };
            let err = (ok == 0).then(|| unsafe { GetLastError() });
            st.pending = None;
            if let Some(e) = err {
                // This instance is unusable; the next accept makes a new one.
                st.instance = None;
                return Err(os_error(e));
            }
        }
        // A fresh instance is ready before the connected one is handed out, so the next client finds one.
        let next = create_instance(&self.name, &self.sd, false)?;
        let conn = st.instance.replace(next);
        conn.map(Stream::from_handle)
            .ok_or_else(|| io::Error::other("no instance"))
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.marker);
        if let Ok(mut st) = self.state.lock() {
            if let (Some(p), Some(inst)) = (st.pending.take(), st.instance.as_ref()) {
                let mut n = 0u32;
                // SAFETY: cancel the connect and wait until it is over, so `ov` is not used after it is freed.
                unsafe {
                    CancelIoEx(inst.0, &*p.ov);
                    GetOverlappedResult(inst.0, &*p.ov, &mut n, 1);
                }
            }
        }
    }
}

pub fn connect(path: &Path) -> io::Result<Stream> {
    let name = pipe_name(path);
    let mut waited = false;
    loop {
        // SAFETY: valid NUL-terminated name; no attributes or template.
        let h = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                null_mut(),
            )
        };
        if h != INVALID_HANDLE_VALUE {
            return Ok(Stream::from_handle(Handle(h)));
        }
        match unsafe { GetLastError() } {
            // Every instance is serving someone: wait (once) for a free one.
            ERROR_PIPE_BUSY if !waited => {
                waited = true;
                // SAFETY: valid name.
                unsafe { WaitNamedPipeW(name.as_ptr(), 2000) };
            }
            ERROR_FILE_NOT_FOUND => return Err(io::Error::from(io::ErrorKind::NotFound)),
            e => return Err(os_error(e)),
        }
    }
}

#[derive(Clone)]
pub struct Stream {
    handle: Arc<Handle>,
    timeout: Arc<Mutex<Option<Duration>>>,
    closed: Arc<AtomicBool>,
}

impl Stream {
    fn from_handle(h: Handle) -> Self {
        Self {
            handle: Arc::new(h),
            timeout: Arc::new(Mutex::new(None)),
            closed: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(self.clone())
    }

    pub fn set_read_timeout(&self, t: Option<Duration>) -> io::Result<()> {
        *self
            .timeout
            .lock()
            .map_err(|_| io::Error::other("poisoned"))? = t;
        Ok(())
    }

    /// Wakes a thread waiting in `read` (it then sees the end of the stream) and makes later reads and writes fail.
    pub fn shutdown(&self) -> io::Result<()> {
        self.closed.store(true, Ordering::Release);
        // SAFETY: cancels every pending operation on the handle; a null `OVERLAPPED` means all of them.
        unsafe { CancelIoEx(self.handle.0, null()) };
        Ok(())
    }

    /// Waits for the operation `ov` started on this handle, up to `limit`; `None` when the time ran out (the operation is then cancelled
    /// and finished before this returns).
    fn finish(
        &self,
        ov: &OVERLAPPED,
        event: &Event,
        limit: Option<Duration>,
    ) -> io::Result<Option<u32>> {
        let ms = limit.map_or(INFINITE, |d| {
            u32::try_from(d.as_millis())
                .unwrap_or(INFINITE - 1)
                .min(INFINITE - 1)
        });
        // SAFETY: a live event handle.
        let w = unsafe { WaitForSingleObject(event.0 .0, ms) };
        let mut n = 0u32;
        if w == WAIT_TIMEOUT {
            // SAFETY: cancel, then wait for the cancellation so the buffer is free again.
            unsafe {
                CancelIoEx(self.handle.0, ov);
                if GetOverlappedResult(self.handle.0, ov, &mut n, 1) != 0 && n > 0 {
                    return Ok(Some(n));
                }
            }
            return Ok(None);
        }
        // SAFETY: the operation completed; `ov` is the one it was started with.
        if unsafe { GetOverlappedResult(self.handle.0, ov, &mut n, 0) } != 0 {
            return Ok(Some(n));
        }
        match unsafe { GetLastError() } {
            ERROR_BROKEN_PIPE | ERROR_PIPE_NOT_CONNECTED | ERROR_NO_DATA => Ok(Some(0)),
            ERROR_OPERATION_ABORTED if self.closed.load(Ordering::Acquire) => Ok(Some(0)),
            e => Err(os_error(e)),
        }
    }

    pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        if self.closed.load(Ordering::Acquire) || buf.is_empty() {
            return Ok(0);
        }
        let event = Event::new()?;
        let mut ov = overlapped(&event);
        let mut n = 0u32;
        let len = u32::try_from(buf.len()).unwrap_or(u32::MAX);
        // SAFETY: `buf` and `ov` stay alive until `finish` has seen the operation end.
        let ok = unsafe { ReadFile(self.handle.0, buf.as_mut_ptr().cast(), len, &mut n, &mut ov) };
        if ok != 0 {
            return Ok(n as usize);
        }
        match unsafe { GetLastError() } {
            ERROR_IO_PENDING => {}
            ERROR_BROKEN_PIPE | ERROR_PIPE_NOT_CONNECTED | ERROR_NO_DATA => return Ok(0),
            e => return Err(os_error(e)),
        }
        let limit = *self
            .timeout
            .lock()
            .map_err(|_| io::Error::other("poisoned"))?;
        match self.finish(&ov, &event, limit)? {
            Some(n) => Ok(n as usize),
            None => Err(io::Error::from(io::ErrorKind::TimedOut)),
        }
    }

    pub fn write(&self, buf: &[u8]) -> io::Result<usize> {
        if self.closed.load(Ordering::Acquire) {
            return Err(io::Error::from(io::ErrorKind::BrokenPipe));
        }
        if buf.is_empty() {
            return Ok(0);
        }
        let event = Event::new()?;
        let mut ov = overlapped(&event);
        let mut n = 0u32;
        let len = u32::try_from(buf.len()).unwrap_or(u32::MAX);
        // SAFETY: `buf` and `ov` stay alive until `finish` has seen the operation end.
        let ok = unsafe { WriteFile(self.handle.0, buf.as_ptr().cast(), len, &mut n, &mut ov) };
        if ok != 0 {
            return Ok(n as usize);
        }
        match unsafe { GetLastError() } {
            ERROR_IO_PENDING => {}
            ERROR_BROKEN_PIPE | ERROR_PIPE_NOT_CONNECTED | ERROR_NO_DATA => {
                return Err(io::Error::from(io::ErrorKind::BrokenPipe))
            }
            e => return Err(os_error(e)),
        }
        match self.finish(&ov, &event, None)? {
            Some(0) => Err(io::Error::from(io::ErrorKind::BrokenPipe)),
            Some(n) => Ok(n as usize),
            None => Err(io::Error::from(io::ErrorKind::TimedOut)),
        }
    }
}

/// Raw mode for the console on stdin: no line editing, no echo, no Ctrl-C handling by the console, keys delivered as the bytes a
/// terminal sends (virtual-terminal input). Output is switched to understand escape sequences too. Fails when stdin is not a console.
pub(crate) fn enable_raw_mode() -> io::Result<crate::RawMode> {
    // SAFETY: plain calls on the process's standard handles; modes are read before they are changed.
    unsafe {
        let stdin = GetStdHandle(STD_INPUT_HANDLE);
        let mut saved_in = 0u32;
        if GetConsoleMode(stdin, &mut saved_in) == 0 {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "stdin is not a console",
            ));
        }
        let raw = (saved_in & !(ENABLE_ECHO_INPUT | ENABLE_LINE_INPUT | ENABLE_PROCESSED_INPUT))
            | ENABLE_VIRTUAL_TERMINAL_INPUT;
        if SetConsoleMode(stdin, raw) == 0 {
            return Err(os_error(GetLastError()));
        }
        let stdout = GetStdHandle(STD_OUTPUT_HANDLE);
        let mut out = 0u32;
        let saved_out = if GetConsoleMode(stdout, &mut out) != 0 {
            SetConsoleMode(
                stdout,
                out | ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING,
            );
            Some(out)
        } else {
            None
        };
        Ok(crate::RawMode {
            saved_in,
            saved_out,
        })
    }
}

pub(crate) fn restore_mode(saved: &crate::RawMode) {
    // SAFETY: restoring modes read earlier from the same handles.
    unsafe {
        SetConsoleMode(GetStdHandle(STD_INPUT_HANDLE), saved.saved_in);
        if let Some(out) = saved.saved_out {
            SetConsoleMode(GetStdHandle(STD_OUTPUT_HANDLE), out);
        }
    }
}

/// Size of the console window on stdout.
pub(crate) fn terminal_size() -> Option<(u16, u16)> {
    // SAFETY: an all-zero info structure is a valid out-buffer.
    let mut info: CONSOLE_SCREEN_BUFFER_INFO = unsafe { std::mem::zeroed() };
    // SAFETY: the standard output handle and a valid out pointer.
    if unsafe { GetConsoleScreenBufferInfo(GetStdHandle(STD_OUTPUT_HANDLE), &mut info) } == 0 {
        return None;
    }
    let w = info.srWindow;
    let (cols, rows) = (
        i32::from(w.Right) - i32::from(w.Left) + 1,
        i32::from(w.Bottom) - i32::from(w.Top) + 1,
    );
    (cols > 0 && rows > 0).then_some((cols as u16, rows as u16))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("agora-pipe-{}-{name}.sock", std::process::id()))
    }

    #[test]
    fn a_client_and_a_server_talk_both_ways_and_a_second_listener_is_refused() {
        let p = path("talk");
        let l = listen(&p).unwrap();
        assert_eq!(
            listen(&p).err().map(|e| e.kind()),
            Some(io::ErrorKind::AddrInUse)
        );
        let server = std::thread::spawn(move || {
            let mut s = l.accept().unwrap();
            let mut b = [0u8; 5];
            s.read_exact_pipe(&mut b);
            assert_eq!(&b, b"hello");
            s.write_all(b"world").unwrap();
            l
        });
        let mut c = connect(&p).unwrap();
        c.write_all(b"hello").unwrap();
        let mut b = [0u8; 5];
        c.read_exact_pipe(&mut b);
        assert_eq!(&b, b"world");
        let _l = server.join().unwrap();
    }

    impl Stream {
        fn read_exact_pipe(&mut self, mut buf: &mut [u8]) {
            while !buf.is_empty() {
                let n = Read::read(self, buf).unwrap();
                assert!(n > 0, "the pipe ended early");
                buf = &mut buf[n..];
            }
        }
    }

    impl Read for Stream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            Stream::read(self, buf)
        }
    }

    impl Write for Stream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            Stream::write(self, buf)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn accept_without_a_client_says_would_block_and_a_waiting_read_times_out_and_shutdown_wakes_it()
    {
        let p = path("modes");
        let l = listen(&p).unwrap();
        l.set_nonblocking(true);
        assert_eq!(
            l.accept().err().map(|e| e.kind()),
            Some(io::ErrorKind::WouldBlock)
        );
        // The same pending connect is picked up when the client arrives.
        let c = connect(&p).unwrap();
        let mut s = loop {
            match l.accept() {
                Ok(s) => break s,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(e) => panic!("{e}"),
            }
        };
        s.set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let mut b = [0u8; 4];
        assert_eq!(
            Read::read(&mut s, &mut b).err().map(|e| e.kind()),
            Some(io::ErrorKind::TimedOut)
        );
        // A read waiting in one thread does not stop a write from another thread, and shutdown ends the read.
        s.set_read_timeout(None).unwrap();
        let reader = s.try_clone().unwrap();
        let t = std::thread::spawn(move || {
            let mut r = reader;
            let mut b = [0u8; 4];
            Read::read(&mut r, &mut b).unwrap()
        });
        std::thread::sleep(Duration::from_millis(200));
        let mut w = s.try_clone().unwrap();
        w.write_all(b"ping").unwrap();
        let mut got = [0u8; 4];
        let mut c = c;
        c.read_exact_pipe(&mut got);
        assert_eq!(&got, b"ping");
        s.shutdown().unwrap();
        assert_eq!(t.join().unwrap(), 0);
    }

    #[test]
    fn a_closed_peer_reads_as_the_end_of_the_stream() {
        let p = path("eof");
        let l = listen(&p).unwrap();
        let c = connect(&p).unwrap();
        let mut s = l.accept().unwrap();
        drop(c);
        let mut b = [0u8; 4];
        assert_eq!(Read::read(&mut s, &mut b).unwrap(), 0);
    }
}
