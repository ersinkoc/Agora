//! Windows backend of the local control channel (ADR-0045): a named pipe with an owner-only security descriptor, behind the same
//! functions and types the Unix socket backend implements. The pipe is opened for overlapped I/O, so a read that waits in one
//! thread never blocks a write from another thread on the same pipe, and a read can time out or be cancelled.
//!
//! Every operation waits for its own completion (or cancels and waits for the cancellation) before it returns, so the buffer and the
//! `OVERLAPPED` it passed to the system are never used after they are gone.

use std::io;
use std::path::{Path, PathBuf};
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
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, WaitNamedPipeW, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{CreateEventW, WaitForSingleObject, INFINITE};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

/// Only the owner of the pipe and the system may open it; nobody else on the machine, and nobody over the network.
const OWNER_ONLY: &str = "D:P(A;;GA;;;OW)(A;;GA;;;SY)";

/// Gives `path` a protected DACL that admits the current user and SYSTEM only, what `0600`/`0700` do on Unix (private keys and tokens,
/// the folder of installed adapters). Nothing is inherited from above it any more, so a folder that lets other users in (a data
/// volume's default ACL) does not reach it. `inherit` also covers what is, or will be, below a folder.
pub(crate) fn set_owner_only(path: &Path, inherit: bool) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Security::Authorization::{SetNamedSecurityInfoW, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{
        GetSecurityDescriptorDacl, ACL, DACL_SECURITY_INFORMATION,
        PROTECTED_DACL_SECURITY_INFORMATION,
    };
    let sd = owner_only_descriptor(inherit)?;
    let (mut present, mut defaulted, mut dacl) = (0i32, 0i32, null_mut::<ACL>());
    // SAFETY: `sd` is a valid descriptor that outlives every use of `dacl`.
    if unsafe { GetSecurityDescriptorDacl(sd.0, &mut present, &mut dacl, &mut defaulted) } == 0 {
        return Err(os_error(unsafe { GetLastError() }));
    }
    let name: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: a NUL-terminated path and a valid DACL.
    let err = unsafe {
        SetNamedSecurityInfoW(
            name.as_ptr().cast_mut(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            dacl,
            null(),
        )
    };
    if err == 0 {
        Ok(())
    } else {
        Err(os_error(err))
    }
}

/// The protected descriptor of [`set_owner_only`]: the current user and SYSTEM, nothing inherited from above.
fn owner_only_descriptor(inherit: bool) -> io::Result<Descriptor> {
    let user = crate::winconfine::current_user_sid_text()?;
    let flags = if inherit { "OICI" } else { "" };
    Descriptor::from_sddl(&format!("D:P(A;{flags};FA;;;{user})(A;{flags};FA;;;SY)"))
}

/// Creates the file at `path`, which must not exist yet (`CREATE_NEW`, so a planted file or link is not followed), already owner-only:
/// the descriptor is given at creation, so there is no moment in which the file carries what its folder passes on and another user
/// could open it (#320). The handle is not inheritable and the file is not shared while open.
pub(crate) fn create_private_new(path: &Path) -> io::Result<std::fs::File> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{CREATE_NEW, FILE_ATTRIBUTE_NORMAL};
    let sd = owner_only_descriptor(false)?;
    let attrs = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.0,
        bInheritHandle: 0,
    };
    let name: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: a NUL-terminated path; `attrs` and the descriptor it points to live for the call.
    let h = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_WRITE,
            0,
            &attrs,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return Err(os_error(unsafe { GetLastError() }));
    }
    // SAFETY: a fresh handle that this `File` now owns and closes.
    Ok(unsafe { std::fs::File::from_raw_handle(h) })
}

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
/// folders do not meet. The path is hashed in one spelling (absolute, `\`, no `..`, not verbatim), so every spelling of one socket file
/// names one pipe, as every spelling reaches one socket on Unix. A path already in that spelling hashes as before.
fn pipe_name(path: &Path) -> Vec<u16> {
    let normal = std::path::absolute(path).map_or_else(|_| path.to_path_buf(), plain_path);
    let full = normal.to_string_lossy().to_lowercase();
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
        // Clients that left before they were accepted are skipped, however many there were.
        loop {
            if let Some(s) = self.accept_once()? {
                return Ok(s);
            }
        }
    }

    /// One accept; `None` when the instance's client had already left (the instance is reset for the next one).
    fn accept_once(&self) -> io::Result<Option<Stream>> {
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
                    // A client connected and left before this accept: the instance stays "being closed" until it is disconnected,
                    // and every later accept would fail on it. Reset it (the name stays served) and wait for the next client.
                    ERROR_NO_DATA => {
                        // SAFETY: a live pipe instance with no operation pending.
                        unsafe { DisconnectNamedPipe(inst) };
                        return Ok(None);
                    }
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
        conn.map(|h| Some(Stream::from_handle(h)))
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

/// `std::fs::canonicalize` answers in the verbatim form (`\\?\C:\work`, `\\?\UNC\server\share`), which `cmd.exe` refuses as a current
/// folder (it falls back to `C:\Windows`) and which some tools cannot open. Gives the ordinary spelling when it means the same file:
/// a drive path or a UNC share, shorter than `MAX_PATH`, whose parts need no verbatim escaping. Otherwise the path is returned as it is.
pub(crate) fn plain_path(path: PathBuf) -> PathBuf {
    const MAX_PATH: usize = 260;
    let Some(text) = path.to_str() else {
        return path;
    };
    let plain = if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        let b = rest.as_bytes();
        if b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\' {
            rest.to_owned()
        } else {
            return path;
        }
    } else {
        return path;
    };
    // Parts that only the verbatim form can name: a trailing dot or space is trimmed, and a device name (`CON`, `NUL`, `COM1`)
    // means the device, in the ordinary spelling. Those keep the verbatim form.
    let device = |p: &str| {
        let stem = p.split('.').next().unwrap_or(p).to_ascii_uppercase();
        matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || ((stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.len() == 4
                && stem.as_bytes()[3].is_ascii_digit())
    };
    let odd = plain
        .split('\\')
        .filter(|p| !p.is_empty())
        .any(|p| p.ends_with('.') || p.ends_with(' ') || device(p));
    if plain.len() >= MAX_PATH || odd || plain.contains('/') {
        return path;
    }
    PathBuf::from(plain)
}

/// See [`crate::opened_path`]: `GetFinalPathNameByHandleW` (the `\\?\` spelling, as `std::fs::canonicalize` gives).
pub(crate) fn opened_path(file: &std::fs::File) -> io::Result<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW;
    let mut buf = vec![0u16; 512];
    loop {
        let cap = u32::try_from(buf.len()).unwrap_or(u32::MAX);
        // SAFETY: a live handle owned by `file`; `buf` holds `cap` UTF-16 units. 0 = normalized name, DOS volume name.
        let n =
            unsafe { GetFinalPathNameByHandleW(file.as_raw_handle(), buf.as_mut_ptr(), cap, 0) };
        if n == 0 {
            return Err(io::Error::last_os_error());
        }
        // A buffer too small gets the size it needs (with the terminating NUL); a fitting one the length without it.
        if n < cap {
            buf.truncate(n as usize);
            return Ok(PathBuf::from(std::ffi::OsString::from_wide(&buf)));
        }
        buf.resize(n as usize, 0);
    }
}

/// See [`crate::open_dir`]: follow the path once, then callers verify the resulting handle before enumerating it.
pub(crate) fn open_dir(path: &Path) -> io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    let dir = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .map_err(|e| io::Error::new(e.kind(), format!("open directory: {e}")))?;
    if !dir.metadata()?.is_dir() {
        return Err(io::ErrorKind::NotADirectory.into());
    }
    Ok(dir)
}

/// See [`crate::visit_dir`]: a fresh synchronous handle to the same object, then one directory entry per query.
/// No path is reopened, no reparse target is traversed for entry metadata, and no later entry is requested after Break.
pub(crate) fn visit_dir(
    dir: &std::fs::File,
    visitor: &mut dyn FnMut(crate::DirItem) -> std::ops::ControlFlow<()>,
) -> io::Result<()> {
    use std::ffi::OsString;
    use std::mem::{offset_of, size_of, size_of_val};
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
    use windows_sys::Wdk::Storage::FileSystem::{
        FileDirectoryInformation, NtOpenFile, NtQueryDirectoryFile, FILE_DIRECTORY_FILE,
        FILE_DIRECTORY_INFORMATION, FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT,
    };
    use windows_sys::Win32::Foundation::{
        RtlNtStatusToDosError, STATUS_NO_MORE_FILES, STATUS_SUCCESS, UNICODE_STRING,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_LIST_DIRECTORY,
        FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, SYNCHRONIZE,
    };
    use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

    let mut empty = [0u16];
    let name = UNICODE_STRING {
        Length: 0,
        MaximumLength: 2,
        Buffer: empty.as_mut_ptr(),
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: dir.as_raw_handle(),
        ObjectName: &name,
        ..Default::default()
    };
    let mut handle = null_mut();
    let mut opened = IO_STATUS_BLOCK::default();
    // SAFETY: live input/output structures; an empty name relative to the held directory reopens that object, without a
    // path lookup. Explicit directory and synchronous options give this enumeration its own cursor and completed queries.
    let status = unsafe {
        NtOpenFile(
            &mut handle,
            FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            &attributes,
            &mut opened,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
        )
    };
    if status != STATUS_SUCCESS {
        // SAFETY: converting the open operation's returned NTSTATUS.
        let error = os_error(unsafe { RtlNtStatusToDosError(status) });
        return Err(io::Error::new(
            error.kind(),
            format!("reopen directory: {error}"),
        ));
    }
    let handle = Handle(handle);
    // Eight-byte aligned and large enough for a complete record plus a maximum-length UTF-16 name, with bounded memory.
    let mut buffer = vec![0u64; 16 * 1024];
    let mut first = true;
    loop {
        let mut result = IO_STATUS_BLOCK::default();
        // SAFETY: a synchronous owned handle, a live aligned output buffer and status block. No APC or event. Single-entry
        // mode preserves the callback's early-stop contract; the first call restarts only this enumeration's own cursor.
        let status = unsafe {
            NtQueryDirectoryFile(
                handle.0,
                null_mut(),
                None,
                null(),
                &mut result,
                buffer.as_mut_ptr().cast(),
                size_of_val(buffer.as_slice()) as u32,
                FileDirectoryInformation,
                true,
                null(),
                first,
            )
        };
        first = false;
        if status == STATUS_NO_MORE_FILES {
            return Ok(());
        }
        if status != STATUS_SUCCESS {
            // SAFETY: converting the returned NTSTATUS to a normal Win32 error code.
            let error = os_error(unsafe { RtlNtStatusToDosError(status) });
            return Err(io::Error::new(
                error.kind(),
                format!("query directory: {error}"),
            ));
        }
        let name_offset = offset_of!(FILE_DIRECTORY_INFORMATION, FileName);
        if result.Information < name_offset || result.Information > size_of_val(buffer.as_slice()) {
            return Err(io::ErrorKind::InvalidData.into());
        }
        // SAFETY: the buffer is aligned, initialized, and contains the fixed portion of the returned record.
        let entry = unsafe { &*buffer.as_ptr().cast::<FILE_DIRECTORY_INFORMATION>() };
        let bytes = entry.FileNameLength as usize;
        if bytes == 0 || !bytes.is_multiple_of(2) || bytes > result.Information - name_offset {
            return Err(io::ErrorKind::InvalidData.into());
        }
        // SAFETY: the name is UTF-16 aligned; its whole byte range was checked against the completed query's output.
        let name = unsafe {
            std::slice::from_raw_parts(
                buffer.as_ptr().cast::<u8>().add(name_offset).cast::<u16>(),
                bytes / 2,
            )
        };
        let name = OsString::from_wide(name);
        if name == "." || name == ".." {
            continue;
        }
        let link = entry.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0;
        let item = crate::DirItem {
            name,
            is_dir: !link && entry.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0,
            len: if link {
                0
            } else {
                u64::try_from(entry.EndOfFile).unwrap_or(0)
            },
            modified: directory_time(entry.LastWriteTime),
        };
        if visitor(item).is_break() {
            return Ok(());
        }
    }
}

/// NT directory timestamps are signed 100 ns ticks since 1601; preserve dates on either side of the Unix epoch.
fn directory_time(ticks: i64) -> std::time::SystemTime {
    const UNIX_EPOCH_TICKS: i128 = 116_444_736_000_000_000;
    let delta = i128::from(ticks) - UNIX_EPOCH_TICKS;
    let magnitude = delta.unsigned_abs();
    let duration = Duration::new(
        (magnitude / 10_000_000) as u64,
        ((magnitude % 10_000_000) * 100) as u32,
    );
    let epoch = std::time::UNIX_EPOCH;
    if delta < 0 {
        epoch.checked_sub(duration)
    } else {
        epoch.checked_add(duration)
    }
    .unwrap_or(epoch)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn verbatim_paths_get_their_ordinary_spelling_only_when_it_means_the_same_file() {
        let p = |s: &str| plain_path(PathBuf::from(s));
        assert_eq!(p(r"\\?\D:\work\repo"), PathBuf::from(r"D:\work\repo"));
        assert_eq!(
            p(r"\\?\UNC\server\share\x"),
            PathBuf::from(r"\\server\share\x")
        );
        assert_eq!(p(r"C:\already\plain"), PathBuf::from(r"C:\already\plain"));
        // Kept verbatim: names the ordinary form would change, a device name, a long path, a volume GUID path.
        assert_eq!(p(r"\\?\C:\dir.\x"), PathBuf::from(r"\\?\C:\dir.\x"));
        assert_eq!(p(r"\\?\C:\a\nul.txt"), PathBuf::from(r"\\?\C:\a\nul.txt"));
        let long = format!(r"\\?\C:\{}", "a".repeat(300));
        assert_eq!(p(&long), PathBuf::from(&long));
        let guid = r"\\?\Volume{01234567-89ab-cdef-0123-456789abcdef}\x";
        assert_eq!(p(guid), PathBuf::from(guid));
    }

    #[test]
    fn a_private_file_is_owner_only_from_its_creation_on() {
        let dir = std::env::temp_dir().join(format!("agora-private-create-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // A parent that lets everyone change what is below it.
        let out = std::process::Command::new("icacls")
            .arg(&dir)
            .args(["/grant", "*S-1-1-0:(OI)(CI)(M)"])
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        let acl = |p: &Path| {
            let out = std::process::Command::new("icacls")
                .arg(p)
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).into_owned()
        };
        // Control: a plain exclusive create here carries the parent's grant from the start, which is the window #320 is about.
        let plain = dir.join("plain.tmp");
        let f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&plain)
            .unwrap();
        assert!(
            acl(&plain).contains("Everyone"),
            "the parent is not permissive"
        );
        drop(f);
        // Looked at while the file is still open and empty, before anything else touches it.
        let private = dir.join("private.tmp");
        let f = create_private_new(&private).unwrap();
        let a = acl(&private);
        assert!(!a.contains("Everyone"), "{a}");
        assert!(!a.contains("(I)"), "inherited entries: {a}");
        assert!(a.contains("SYSTEM"), "{a}");
        drop(f);
        // Exclusive: an existing name is never opened.
        assert!(create_private_new(&private).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn private_files_and_folders_shut_out_a_permissive_parent() {
        let dir = std::env::temp_dir().join(format!("agora-private-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // A parent that lets everyone change what is below it, like a data volume's default ACL lets every user.
        let icacls = |args: &[&str]| {
            let out = std::process::Command::new("icacls")
                .arg(&dir)
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success(), "{out:?}");
        };
        icacls(&["/grant", "*S-1-1-0:(OI)(CI)(M)"]);
        let acl = |p: &Path| {
            let out = std::process::Command::new("icacls")
                .arg(p)
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).into_owned()
        };
        let key = dir.join("key.pem");
        crate::write_private(&key, b"one").unwrap();
        crate::write_private(&key, b"two-longer").unwrap();
        assert_eq!(std::fs::read(&key).unwrap(), b"two-longer");
        let adapters = dir.join("adapters");
        std::fs::create_dir_all(&adapters).unwrap();
        crate::restrict_to_owner(&adapters).unwrap();
        std::fs::write(adapters.join("tool.exe"), b"x").unwrap();
        for p in [key.clone(), adapters.clone(), adapters.join("tool.exe")] {
            let a = acl(&p);
            assert!(!a.contains("Everyone"), "{}: {a}", p.display());
            assert!(a.contains("SYSTEM"), "{}: {a}", p.display());
        }
        assert!(std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .all(|e| !e.file_name().to_string_lossy().contains(".tmp-")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_second_update_works_while_the_program_moved_aside_by_the_first_still_runs() {
        let dir = std::env::temp_dir().join(format!("agora-update-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Any long-running program stands in for a node started before the first update.
        let ping = Path::new(r"C:\Windows\System32\PING.EXE");
        let target = dir.join("agora.exe");
        std::fs::copy(ping, &target).unwrap();
        let mut node = std::process::Command::new(&target)
            .args(["-n", "30", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        for n in 1..=3 {
            let new = dir.join(format!("new-{n}.exe"));
            std::fs::copy(ping, &new).unwrap();
            crate::replace_executable(&new, &target).unwrap_or_else(|e| panic!("update {n}: {e}"));
            assert!(target.exists() && !new.exists());
        }
        let _ = node.kill();
        let _ = node.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_canonical_folder_is_one_cmd_can_start_in() {
        let dir = std::env::temp_dir();
        let canon = crate::canonicalize_plain(&dir).unwrap();
        assert!(!canon.to_string_lossy().starts_with(r"\\?\"), "{canon:?}");
        let out = std::process::Command::new("cmd")
            .args(["/c", "cd"])
            .current_dir(&canon)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            text.trim()
                .eq_ignore_ascii_case(canon.to_string_lossy().trim_end_matches('\\')),
            "cmd started in {text:?}, not {canon:?}"
        );
    }

    fn path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("agora-pipe-{}-{name}.sock", std::process::id()))
    }

    #[test]
    fn every_spelling_of_one_socket_path_reaches_one_listener() {
        let p = path("spelling");
        let l = listen(&p).unwrap();
        let text = p.to_string_lossy().into_owned();
        let parent = p.parent().unwrap();
        let spellings = [
            PathBuf::from(text.replace('\\', "/")),
            parent.join("x").join("..").join(p.file_name().unwrap()),
            PathBuf::from(format!(r"\\?\{text}")),
            PathBuf::from(text.to_uppercase()),
        ];
        // Pipe names compare without case, like the paths.
        let name = |p: &Path| String::from_utf16_lossy(&pipe_name(p)).to_lowercase();
        for s in &spellings {
            assert_eq!(name(s), name(&p), "{}", s.display());
            let client = connect(s);
            assert!(client.is_ok(), "{}", s.display());
            // Frees the instance for the next client.
            l.accept().unwrap();
            drop(client);
        }
        // Relative to the current folder: the same pipe as the absolute spelling there.
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(
            pipe_name(Path::new("n.sock")),
            pipe_name(&cwd.join("n.sock"))
        );
        // Two different socket files still get two pipes.
        assert_ne!(pipe_name(&p), pipe_name(&path("other")));
        drop(l);
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

    #[test]
    fn a_client_that_leaves_before_accept_does_not_stop_the_next_one() {
        let p = path("early");
        let l = listen(&p).unwrap();
        // Connects and leaves before the server accepts (it holds the only free instance until then).
        drop(connect(&p).unwrap());
        let client = {
            let p = p.clone();
            std::thread::spawn(move || {
                let mut c = connect(&p).unwrap();
                c.write_all(b"hi").unwrap();
                let mut b = [0u8; 2];
                c.read_exact_pipe(&mut b);
                b
            })
        };
        let mut s = l.accept().unwrap();
        let mut b = [0u8; 2];
        s.read_exact_pipe(&mut b);
        assert_eq!(&b, b"hi");
        s.write_all(b"ok").unwrap();
        assert_eq!(&client.join().unwrap(), b"ok");
    }
}
