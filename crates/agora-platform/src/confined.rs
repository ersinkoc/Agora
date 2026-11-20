//! Folder handles for writing below a root without a path being looked up twice (issue #847).
//!
//! A path checked and then opened by name is two steps another process can change in between: a folder or the file swapped for a link.
//! [`ConfinedDir`] holds a folder open and does everything relative to that handle, one name at a time:
//!
//! - **Unix**: `openat`/`mkdirat`/`renameat`/`unlinkat`/`fstatat` on the descriptor; every folder opened with
//!   `O_NOFOLLOW | O_DIRECTORY`, every file created with `O_CREAT | O_EXCL | O_NOFOLLOW`. A link anywhere is refused ([`not_confined`]),
//!   never followed.
//! - **Windows**: `NtCreateFile`/`NtSetInformationFile` with the folder handle as `RootDirectory`, always with
//!   `FILE_OPEN_REPARSE_POINT`, so a link or junction is opened as itself and then refused (its attribute is checked on the handle).
//!   Guaranteed: no link or junction is followed by a name step, and a rename happens inside the held folder. Not guaranteed: the
//!   root itself is opened by path once (the caller compares where the handle really is, [`ConfinedDir::real_path`]); an
//!   alternate data stream or short-name spelling is refused by name rather than resolved ([`valid_component`]); a rename over a file
//!   another process holds open fails instead of replacing it.
//!
//! Names are single components. `..`, separators and anything else a platform treats specially are refused.

use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::path::PathBuf;

/// The error inside an [`io::Error`] from [`ConfinedDir`]: a link (or something that is not a folder) stood where a folder was
/// expected, or a name was not a plain component, so nothing was followed and nothing was written there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotConfined;

impl std::fmt::Display for NotConfined {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("not_confined")
    }
}

impl std::error::Error for NotConfined {}

/// Whether `e` (from [`ConfinedDir`]) says a link, a non-folder, or a name that is not a plain component was refused.
pub fn not_confined(e: &io::Error) -> bool {
    e.get_ref().is_some_and(|inner| inner.is::<NotConfined>())
}

fn refused() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, NotConfined)
}

/// What an entry is, described without following a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Folder,
    Link,
    /// A FIFO, a device, a socket: never a write target.
    Other,
}

/// One entry of a held folder: its kind and, on Unix, its permission bits (0 elsewhere).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub kind: EntryKind,
    pub mode: u32,
}

/// Whether `name` is one plain component this module will use: not empty, not `.` or `..`, no separator, no NUL; on Windows also
/// nothing the file system gives a second meaning (`:` streams, reserved device names, trailing dots and spaces, short names such as
/// `ID_RSA~1`, wildcard characters). Refusing is the safe answer for a name this does not understand.
pub fn valid_component(name: &OsStr) -> bool {
    let Some(text) = name.to_str() else {
        // Not text: on Windows an unpaired surrogate, on Unix arbitrary bytes. Unix has no second meaning for them.
        return cfg!(unix) && !name.is_empty() && !has_unix_separator_bytes(name);
    };
    if text.is_empty() || text == "." || text == ".." || text.contains(['/', '\\', '\0']) {
        // A backslash is an ordinary character on Unix, but the secret policy treats it as a separator, so it is refused everywhere.
        return false;
    }
    #[cfg(windows)]
    {
        windows_valid(text)
    }
    #[cfg(not(windows))]
    {
        true
    }
}

fn has_unix_separator_bytes(name: &OsStr) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        name.as_bytes()
            .iter()
            .any(|b| matches!(b, b'/' | b'\\' | 0))
    }
    #[cfg(not(unix))]
    {
        let _ = name;
        true
    }
}

#[cfg(any(windows, test))]
fn windows_valid(text: &str) -> bool {
    if text.contains([':', '<', '>', '"', '|', '?', '*']) || text.chars().any(|c| c < ' ') {
        return false;
    }
    if text.ends_with('.') || text.ends_with(' ') {
        return false;
    }
    // `NAME~1`: an 8.3 alias that names another file under a spelling the secret policy does not know.
    let bytes = text.as_bytes();
    if bytes
        .windows(2)
        .any(|w| w[0] == b'~' && w[1].is_ascii_digit())
    {
        return false;
    }
    let stem = text.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || (stem.len() == 4
        && (stem.starts_with("COM") || stem.starts_with("LPT"))
        && stem.as_bytes()[3].is_ascii_digit());
    !reserved
}

/// A folder held open; see the module docs. Every method takes one plain name relative to it.
#[derive(Debug)]
pub struct ConfinedDir {
    dir: File,
}

impl ConfinedDir {
    /// Opens the folder at `path` (the one place a path is looked up; it follows links as any open does, so the caller checks where
    /// the handle really is with [`Self::real_path`]).
    pub fn open_root(path: &std::path::Path) -> io::Result<Self> {
        Ok(Self {
            dir: crate::open_dir(path)?,
        })
    }

    /// Where this folder really is now, as the kernel knows it ([`crate::opened_path`]).
    pub fn real_path(&self) -> io::Result<PathBuf> {
        crate::opened_path(&self.dir)
    }

    /// The folder `name` inside this one, opened without following a link. With `create`, a missing folder is made first. A link, a
    /// file, or anything else that is not a folder is refused ([`not_confined`]).
    pub fn subdir(&self, name: &OsStr, create: bool) -> io::Result<Self> {
        if !valid_component(name) {
            return Err(refused());
        }
        imp::subdir(&self.dir, name, create).map(|dir| Self { dir })
    }

    /// What `name` is, without following it; `None` when there is nothing by that name.
    pub fn entry(&self, name: &OsStr) -> io::Result<Option<Entry>> {
        if !valid_component(name) {
            return Err(refused());
        }
        imp::entry(&self.dir, name)
    }

    /// A new regular file `name`, opened for writing, failing when anything exists by that name (a link included, never followed).
    /// `mode` is the Unix permission bits (the process umask applies); ignored elsewhere. On Windows the handle is not shared, and
    /// carries the access [`Self::rename_over`] needs.
    pub fn create_new(&self, name: &OsStr, mode: u32) -> io::Result<File> {
        if !valid_component(name) {
            return Err(refused());
        }
        imp::create_new(&self.dir, name, mode)
    }

    /// Renames the file `file` (just made by [`Self::create_new`] as `from`) to `to` inside this folder, replacing a file already
    /// there (a link at `to` is replaced as itself, never followed). Atomic on both systems. On Unix `from` must still be the file `file`
    /// (device and inode compared, regular file), else [`not_confined`] and nothing is renamed.
    pub fn rename_over(&self, file: &File, from: &OsStr, to: &OsStr) -> io::Result<()> {
        if !valid_component(from) || !valid_component(to) {
            return Err(refused());
        }
        imp::rename_over(&self.dir, file, from, to)
    }

    /// Removes `name` only if it is still the file `file` (made by [`Self::create_new`]) has open: on Unix the name is compared with
    /// the handle (device, inode, regular file) and anything else is left alone; on Windows the handle itself is deleted.
    pub fn remove_owned(&self, file: &File, name: &OsStr) -> io::Result<()> {
        if !valid_component(name) {
            return Err(refused());
        }
        imp::remove_owned(&self.dir, file, name)
    }

    /// Removes the file (or link, as itself) `name`. Close any handle to it first.
    pub fn remove_file(&self, name: &OsStr) -> io::Result<()> {
        if !valid_component(name) {
            return Err(refused());
        }
        imp::remove_file(&self.dir, name)
    }
}

#[cfg(unix)]
mod imp {
    use super::{refused, Entry, EntryKind};
    use std::ffi::{CString, OsStr};
    use std::fs::File;
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;

    fn cname(name: &OsStr) -> io::Result<CString> {
        CString::new(name.as_bytes()).map_err(|_| refused())
    }

    fn open_at(dir: &File, name: &CString, flags: libc::c_int, mode: u32) -> io::Result<File> {
        // SAFETY: a live directory descriptor and a NUL-terminated name; on success the new descriptor is ours alone.
        let fd = unsafe {
            libc::openat(
                dir.as_raw_fd(),
                name.as_ptr(),
                flags | libc::O_CLOEXEC | libc::O_NOCTTY,
                mode as libc::c_uint,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `fd` is a fresh descriptor returned by `openat`.
        Ok(unsafe { File::from_raw_fd(fd) })
    }

    pub(super) fn subdir(dir: &File, name: &OsStr, create: bool) -> io::Result<File> {
        let c = cname(name)?;
        let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW;
        let mut tried_create = false;
        loop {
            match open_at(dir, &c, flags, 0) {
                Ok(f) => return Ok(f),
                // A link (ELOOP, or ENOTDIR on systems that report the directory flag first) or a file in the way.
                Err(e) if matches!(e.raw_os_error(), Some(libc::ELOOP | libc::ENOTDIR)) => {
                    return Err(refused())
                }
                Err(e) if create && !tried_create && e.kind() == io::ErrorKind::NotFound => {
                    tried_create = true;
                    // SAFETY: a live directory descriptor and a NUL-terminated name.
                    let r = unsafe { libc::mkdirat(dir.as_raw_fd(), c.as_ptr(), 0o777) };
                    if r < 0 {
                        let e = io::Error::last_os_error();
                        // Created by someone else in between: open what is there (still without following).
                        if e.kind() != io::ErrorKind::AlreadyExists {
                            return Err(e);
                        }
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }

    pub(super) fn entry(dir: &File, name: &OsStr) -> io::Result<Option<Entry>> {
        let c = cname(name)?;
        // SAFETY: zeroed `stat` is a valid out-parameter; descriptor and name are live.
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        let r = unsafe {
            libc::fstatat(
                dir.as_raw_fd(),
                c.as_ptr(),
                &mut st,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if r < 0 {
            let e = io::Error::last_os_error();
            return if e.kind() == io::ErrorKind::NotFound {
                Ok(None)
            } else {
                Err(e)
            };
        }
        let kind = match st.st_mode & libc::S_IFMT {
            libc::S_IFREG => EntryKind::File,
            libc::S_IFDIR => EntryKind::Folder,
            libc::S_IFLNK => EntryKind::Link,
            _ => EntryKind::Other,
        };
        // `st_mode` is 16 bits on macOS and 32 on Linux.
        #[allow(clippy::useless_conversion)]
        let mode = u32::from(st.st_mode & 0o7777);
        Ok(Some(Entry { kind, mode }))
    }

    pub(super) fn create_new(dir: &File, name: &OsStr, mode: u32) -> io::Result<File> {
        let c = cname(name)?;
        // O_NONBLOCK: never waits on a FIFO (O_EXCL already refuses one that exists); regular files ignore it.
        let flags =
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_NONBLOCK;
        let file = open_at(dir, &c, flags, mode)?;
        if !file.metadata()?.is_file() {
            return Err(refused());
        }
        Ok(file)
    }

    /// Whether `name` in `dir` is still the very file `file` has open: same device and inode, and a regular file (not followed).
    // `st_dev` and `st_ino` widths differ between Linux and macOS.
    #[allow(clippy::unnecessary_cast)]
    fn is_same_file(dir: &File, file: &File, name: &OsStr) -> io::Result<bool> {
        use std::os::unix::fs::MetadataExt;
        let c = cname(name)?;
        // SAFETY: zeroed `stat` is a valid out-parameter; descriptor and name are live.
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        let r = unsafe {
            libc::fstatat(
                dir.as_raw_fd(),
                c.as_ptr(),
                &mut st,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if r < 0 {
            let e = io::Error::last_os_error();
            return if e.kind() == io::ErrorKind::NotFound {
                Ok(false)
            } else {
                Err(e)
            };
        }
        let mine = file.metadata()?;
        Ok(st.st_mode & libc::S_IFMT == libc::S_IFREG
            && st.st_ino as u64 == mine.ino()
            && st.st_dev as u64 == mine.dev())
    }

    pub(super) fn rename_over(dir: &File, file: &File, from: &OsStr, to: &OsStr) -> io::Result<()> {
        // The temporary name must still be the file this process made: a name swapped for a link or another file is not renamed.
        if !is_same_file(dir, file, from)? {
            return Err(refused());
        }
        let (f, t) = (cname(from)?, cname(to)?);
        // SAFETY: one live directory descriptor for both sides and NUL-terminated names; `renameat` replaces `to` itself, never
        // what a link there points at.
        let r = unsafe { libc::renameat(dir.as_raw_fd(), f.as_ptr(), dir.as_raw_fd(), t.as_ptr()) };
        if r < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub(super) fn remove_owned(dir: &File, file: &File, name: &OsStr) -> io::Result<()> {
        // Only what this process made: anything else under the name is not ours to remove.
        if !is_same_file(dir, file, name)? {
            return Ok(());
        }
        remove_file(dir, name)
    }

    pub(super) fn remove_file(dir: &File, name: &OsStr) -> io::Result<()> {
        let c = cname(name)?;
        // SAFETY: a live directory descriptor and a NUL-terminated name; flags 0 removes a non-directory entry itself.
        let r = unsafe { libc::unlinkat(dir.as_raw_fd(), c.as_ptr(), 0) };
        if r < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(windows)]
mod imp {
    use super::{refused, Entry, EntryKind};
    use std::ffi::OsStr;
    use std::fs::File;
    use std::io;
    use std::mem::{size_of, size_of_val};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use std::ptr::null_mut;
    use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
    use windows_sys::Wdk::Storage::FileSystem::{
        FileDispositionInformation, FileRenameInformation, NtCreateFile, NtSetInformationFile,
        FILE_CREATE, FILE_DIRECTORY_FILE, FILE_DISPOSITION_INFORMATION, FILE_NON_DIRECTORY_FILE,
        FILE_OPEN, FILE_OPEN_IF, FILE_OPEN_REPARSE_POINT, FILE_RENAME_INFORMATION,
        FILE_SYNCHRONOUS_IO_NONALERT,
    };
    use windows_sys::Win32::Foundation::{
        RtlNtStatusToDosError, OBJ_CASE_INSENSITIVE, STATUS_NOT_A_DIRECTORY, STATUS_SUCCESS,
        UNICODE_STRING,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FileAttributeTagInfo, GetFileInformationByHandleEx, GetFileType, DELETE,
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO,
        FILE_GENERIC_WRITE, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TRAVERSE, FILE_TYPE_DISK, SYNCHRONIZE,
    };
    use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

    fn status_error(status: i32) -> io::Error {
        // SAFETY: converting an NTSTATUS to a Win32 error code touches no memory.
        let code = unsafe { RtlNtStatusToDosError(status) };
        io::Error::from_raw_os_error(code as i32)
    }

    /// `NtCreateFile` relative to `dir`, with case-insensitive matching as every Win32 open has. Callers always pass
    /// `FILE_OPEN_REPARSE_POINT` so a link is opened as itself.
    fn nt_open(
        dir: &File,
        name: &OsStr,
        access: u32,
        share: u32,
        disposition: u32,
        options: u32,
    ) -> io::Result<File> {
        let mut wide: Vec<u16> = name.encode_wide().collect();
        let bytes = u16::try_from(wide.len() * 2).map_err(|_| refused())?;
        let object_name = UNICODE_STRING {
            Length: bytes,
            MaximumLength: bytes,
            Buffer: wide.as_mut_ptr(),
        };
        let attributes = OBJECT_ATTRIBUTES {
            Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
            RootDirectory: dir.as_raw_handle(),
            ObjectName: &object_name,
            Attributes: OBJ_CASE_INSENSITIVE,
            ..Default::default()
        };
        let mut handle = null_mut();
        let mut block = IO_STATUS_BLOCK::default();
        // SAFETY: live, initialized structures; the name is relative to a held directory handle, so no path is looked up.
        let status = unsafe {
            NtCreateFile(
                &mut handle,
                access,
                &attributes,
                &mut block,
                std::ptr::null(),
                0,
                share,
                disposition,
                options | FILE_SYNCHRONOUS_IO_NONALERT,
                std::ptr::null(),
                0,
            )
        };
        if status != STATUS_SUCCESS {
            return Err(if status == STATUS_NOT_A_DIRECTORY {
                refused()
            } else {
                status_error(status)
            });
        }
        // SAFETY: the call succeeded, so `handle` is a new handle owned by nobody else.
        Ok(unsafe { File::from_raw_handle(handle) })
    }

    fn attributes(file: &File) -> io::Result<u32> {
        let mut info = FILE_ATTRIBUTE_TAG_INFO::default();
        // SAFETY: a live handle and a correctly sized out-structure.
        let ok = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileAttributeTagInfo,
                (&mut info as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
                size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(info.FileAttributes)
    }

    const SHARE_ALL: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;

    pub(super) fn subdir(dir: &File, name: &OsStr, create: bool) -> io::Result<File> {
        let child = nt_open(
            dir,
            name,
            FILE_LIST_DIRECTORY | FILE_TRAVERSE | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            SHARE_ALL,
            if create { FILE_OPEN_IF } else { FILE_OPEN },
            FILE_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT,
        )?;
        // Opened as itself: a link or junction shows its reparse attribute here and is refused, never followed.
        let attrs = attributes(&child)?;
        if attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 || attrs & FILE_ATTRIBUTE_DIRECTORY == 0 {
            return Err(refused());
        }
        Ok(child)
    }

    pub(super) fn entry(dir: &File, name: &OsStr) -> io::Result<Option<Entry>> {
        let file = match nt_open(
            dir,
            name,
            FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            SHARE_ALL,
            FILE_OPEN,
            FILE_OPEN_REPARSE_POINT,
        ) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let attrs = attributes(&file)?;
        let kind = if attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            EntryKind::Link
        } else if attrs & FILE_ATTRIBUTE_DIRECTORY != 0 {
            EntryKind::Folder
        } else {
            // SAFETY: a live handle.
            if unsafe { GetFileType(file.as_raw_handle()) } == FILE_TYPE_DISK {
                EntryKind::File
            } else {
                EntryKind::Other
            }
        };
        Ok(Some(Entry { kind, mode: 0 }))
    }

    pub(super) fn create_new(dir: &File, name: &OsStr, _mode: u32) -> io::Result<File> {
        // Not shared: nobody else can open the temporary file while it is written. DELETE lets it be renamed through this handle.
        nt_open(
            dir,
            name,
            FILE_GENERIC_WRITE | DELETE | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            0,
            FILE_CREATE,
            FILE_NON_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT,
        )
    }

    pub(super) fn rename_over(
        dir: &File,
        file: &File,
        _from: &OsStr,
        to: &OsStr,
    ) -> io::Result<()> {
        let name: Vec<u16> = to.encode_wide().collect();
        let name_bytes = name.len() * 2;
        let header = std::mem::offset_of!(FILE_RENAME_INFORMATION, FileName);
        // Eight-byte aligned buffer for the fixed part plus the name.
        let mut buffer = vec![0u64; (header + name_bytes).div_ceil(8) + 1];
        let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFORMATION>();
        // SAFETY: the buffer is aligned, zeroed and large enough for the header and the name; the name is copied inside it.
        unsafe {
            (*info).Anonymous.ReplaceIfExists = true;
            // Relative to the held folder: the new name is looked up there, never as a path.
            (*info).RootDirectory = dir.as_raw_handle();
            (*info).FileNameLength = name_bytes as u32;
            std::ptr::copy_nonoverlapping(
                name.as_ptr(),
                buffer.as_mut_ptr().cast::<u8>().add(header).cast::<u16>(),
                name.len(),
            );
        }
        let mut block = IO_STATUS_BLOCK::default();
        // SAFETY: a live handle opened with DELETE access and a fully initialized information structure of the stated length.
        let status = unsafe {
            NtSetInformationFile(
                file.as_raw_handle(),
                &mut block,
                buffer.as_ptr().cast(),
                (header + name_bytes) as u32,
                FileRenameInformation,
            )
        };
        if status != STATUS_SUCCESS {
            return Err(status_error(status));
        }
        Ok(())
    }

    pub(super) fn remove_file(dir: &File, name: &OsStr) -> io::Result<()> {
        let file = nt_open(
            dir,
            name,
            DELETE | SYNCHRONIZE,
            SHARE_ALL,
            FILE_OPEN,
            FILE_NON_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT,
        )?;
        set_delete(&file)
    }

    /// Deletes through the handle this process made (opened with DELETE, not shared): no name is looked up, so nothing else can
    /// be removed.
    pub(super) fn remove_owned(_dir: &File, file: &File, _name: &OsStr) -> io::Result<()> {
        set_delete(file)
    }

    fn set_delete(file: &File) -> io::Result<()> {
        let info = FILE_DISPOSITION_INFORMATION { DeleteFile: true };
        let mut block = IO_STATUS_BLOCK::default();
        // SAFETY: a live handle with DELETE access and a correctly sized information structure.
        let status = unsafe {
            NtSetInformationFile(
                file.as_raw_handle(),
                &mut block,
                (&info as *const FILE_DISPOSITION_INFORMATION).cast(),
                size_of_val(&info) as u32,
                FileDispositionInformation,
            )
        };
        if status != STATUS_SUCCESS {
            return Err(status_error(status));
        }
        Ok(())
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use super::Entry;
    use std::ffi::OsStr;
    use std::fs::File;
    use std::io;

    fn unsupported<T>() -> io::Result<T> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
    pub(super) fn subdir(_: &File, _: &OsStr, _: bool) -> io::Result<File> {
        unsupported()
    }
    pub(super) fn entry(_: &File, _: &OsStr) -> io::Result<Option<Entry>> {
        unsupported()
    }
    pub(super) fn create_new(_: &File, _: &OsStr, _: u32) -> io::Result<File> {
        unsupported()
    }
    pub(super) fn rename_over(_: &File, _: &File, _: &OsStr, _: &OsStr) -> io::Result<()> {
        unsupported()
    }
    pub(super) fn remove_file(_: &File, _: &OsStr) -> io::Result<()> {
        unsupported()
    }
    pub(super) fn remove_owned(_: &File, _: &File, _: &OsStr) -> io::Result<()> {
        unsupported()
    }
}

#[cfg(test)]
mod name_tests {
    use super::*;

    #[test]
    fn plain_components_only() {
        for ok in ["a", "a.txt", ".hidden", "with space", "ünï"] {
            assert!(valid_component(OsStr::new(ok)), "{ok}");
        }
        for bad in ["", ".", "..", "a/b", "a\\b", "a\0b"] {
            assert!(!valid_component(OsStr::new(bad)), "{bad:?}");
        }
    }

    /// The Windows rules are plain string checks, so they run everywhere.
    #[test]
    fn windows_second_meanings_are_refused() {
        for bad in [
            ".env:stream",
            "a.txt::$DATA",
            "NUL",
            "nul.txt",
            "Com1",
            "lpt9.log",
            "trailing.",
            "trailing ",
            "ID_RSA~1",
            "a*b",
            "a?b",
            "q\"",
            "a|b",
            "tab\tname",
            "CON",
            "con.txt",
            "COM9",
            "LPT1",
            "AUX",
            "PRN.x",
            "CONIN$",
            "a<b",
            "a>b",
            "PROGRA~1",
        ] {
            assert!(!windows_valid(bad), "{bad:?}");
        }
        for ok in ["a.txt", "console.rs", "com10", "tilde~a", "lpt"] {
            assert!(windows_valid(ok), "{ok:?}");
        }
    }
}
