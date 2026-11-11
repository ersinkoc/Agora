//! Windows agent confinement (ADR-0050): a restricted token with one restricting SID per agent.
//!
//! Windows grants an access to a process with a restricted token only when the normal check **and** the check against the restricting
//! SIDs both pass. The restricting list is this agent's own random SID plus `Everyone`, `Users` and the logon SID. The node adds an ACL
//! entry for the agent's SID to exactly the folders the agent may use, so everything the user alone can reach (the profile, the node's
//! keys, another agent's folders) is refused, while system folders that every user can read stay readable.
//!
//! The entries are recorded in a ledger file *before* they are added and removed when the agent ends; [`recover_stale`] undoes what a
//! crashed node left behind. The feature stays off ([`enabled`]) until the maintainer switches it on after real-machine checks.

use std::io;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, LocalFree, FILETIME, HANDLE};
use windows_sys::Win32::Security::Authorization::{
    BuildTrusteeWithSidW, ConvertStringSidToSidW, GetNamedSecurityInfoW, SetEntriesInAclW,
    SetNamedSecurityInfoW, EXPLICIT_ACCESS_W, GRANT_ACCESS, REVOKE_ACCESS, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::Cryptography::{
    BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
};
use windows_sys::Win32::Security::{
    CreateRestrictedToken, GetTokenInformation, InitializeSecurityDescriptor, SetFileSecurityW,
    SetSecurityDescriptorDacl, TokenLogonSid, DACL_SECURITY_INFORMATION, DISABLE_MAX_PRIVILEGE,
    PSID, SECURITY_DESCRIPTOR, SID_AND_ATTRIBUTES, TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE,
    TOKEN_GROUPS, TOKEN_QUERY,
};
use windows_sys::Win32::Storage::FileSystem::{
    DELETE, FILE_DELETE_CHILD, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
    FILE_READ_ATTRIBUTES, FILE_TRAVERSE,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetExitCodeProcess, GetProcessTimes, OpenProcess,
    OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};

use crate::Sandbox;

/// The switch. Off until the maintainer enables it (ADR-0050 asks for real-machine checks first); `AGORA_WINDOWS_CONFINEMENT=1`
/// turns it on for one run so those checks can be made without a build.
const ENABLED: bool = false;

pub(crate) fn enabled() -> bool {
    ENABLED || std::env::var_os("AGORA_WINDOWS_CONFINEMENT").is_some_and(|v| v == "1")
}

/// `OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE`: the entry covers the folder and everything below it.
const INHERIT_ALL: u32 = 0x3;
const READ_MASK: u32 = FILE_GENERIC_READ | FILE_GENERIC_EXECUTE;
const WRITE_MASK: u32 =
    FILE_GENERIC_READ | FILE_GENERIC_WRITE | FILE_GENERIC_EXECUTE | DELETE | FILE_DELETE_CHILD;
/// What a folder above a granted one gets: its attributes can be read (`stat`) and it can be passed through, nothing else, and on
/// that folder only. Tools like git `stat` every folder above a file they create; without this they fail under the user profile.
/// `SYNCHRONIZE` (0x0010_0000) comes with every file open.
const ANCESTOR_MASK: u32 = FILE_READ_ATTRIBUTES | FILE_TRAVERSE | 0x0010_0000;
/// Marks a ledger line whose entry was set on that folder only (an ancestor), so it is removed the same way.
const HERE_MARK: &str = "here";

/// The folders above `paths`, without the drive root, without the paths themselves, each once.
fn ancestors_of(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for p in paths {
        for a in p.ancestors().skip(1) {
            // The drive root (`C:\`) has no parent; every user may already read its attributes.
            if a.parent().is_none() || a.as_os_str().is_empty() {
                continue;
            }
            let a = a.to_path_buf();
            if !paths.contains(&a) && !out.contains(&a) {
                out.push(a);
            }
        }
    }
    out
}

fn last_error() -> io::Error {
    // SAFETY: plain call.
    io::Error::from_raw_os_error(unsafe { GetLastError() } as i32)
}

fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    s.encode_wide().chain(std::iter::once(0)).collect()
}

/// A SID the system allocated, freed on drop.
struct Sid(PSID);

impl Sid {
    fn from_string(s: &str) -> io::Result<Self> {
        let w = wide(std::ffi::OsStr::new(s));
        let mut sid: PSID = null_mut();
        // SAFETY: `w` is a NUL-terminated string; `sid` receives a LocalAlloc'd SID.
        if unsafe { ConvertStringSidToSidW(w.as_ptr(), &mut sid) } == 0 {
            return Err(last_error());
        }
        Ok(Self(sid))
    }
}

impl Drop for Sid {
    fn drop(&mut self) {
        // SAFETY: allocated by ConvertStringSidToSidW and freed once.
        unsafe { LocalFree(self.0) };
    }
}

struct Token(HANDLE);

impl Drop for Token {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: owned and closed once.
            unsafe { CloseHandle(self.0) };
        }
    }
}

/// A fresh SID that belongs to nothing else: four random 32-bit numbers below the domain authority.
fn random_sid_string() -> io::Result<String> {
    let mut n = [0u32; 4];
    // SAFETY: a 16-byte buffer and the system RNG.
    let status = unsafe {
        BCryptGenRandom(
            null_mut(),
            n.as_mut_ptr().cast(),
            16,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status != 0 {
        return Err(io::Error::other("no random numbers"));
    }
    Ok(format!("S-1-5-21-{}-{}-{}-{}", n[0], n[1], n[2], n[3]))
}

/// Adds (`grant`) or removes the SID's entry on `path` itself: not inherited by anything below, and written with
/// `SetFileSecurityW`, which (unlike `SetNamedSecurityInfoW`) does not walk the folder's children. For the folders above a granted one.
fn change_acl_here(path: &Path, sid: PSID, mask: u32, grant: bool) -> io::Result<()> {
    let name = wide(path.as_os_str());
    let mut dacl = null_mut();
    let mut descriptor = null_mut();
    // SAFETY: a NUL-terminated path; the out pointers receive memory we free below.
    let err = unsafe {
        GetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            &mut dacl,
            null_mut(),
            &mut descriptor,
        )
    };
    if err != 0 {
        return Err(io::Error::from_raw_os_error(err as i32));
    }
    // SAFETY: an all-zero EXPLICIT_ACCESS_W is a valid start.
    let mut access: EXPLICIT_ACCESS_W = unsafe { std::mem::zeroed() };
    access.grfAccessPermissions = mask;
    access.grfAccessMode = if grant { GRANT_ACCESS } else { REVOKE_ACCESS };
    access.grfInheritance = 0; // NO_INHERITANCE
                               // SAFETY: `sid` is a valid SID that outlives the call.
    unsafe { BuildTrusteeWithSidW(&mut access.Trustee, sid) };
    let mut new_acl = null_mut();
    // SAFETY: one entry, the existing DACL (kept alive by `descriptor`).
    let err = unsafe { SetEntriesInAclW(1, &access, dacl, &mut new_acl) };
    let result = if err != 0 {
        Err(io::Error::from_raw_os_error(err as i32))
    } else {
        // SAFETY: an all-zero descriptor is initialized by the call before use.
        let mut sd: SECURITY_DESCRIPTOR = unsafe { std::mem::zeroed() };
        let sd_ptr = (&mut sd as *mut SECURITY_DESCRIPTOR).cast();
        // SAFETY: `sd` lives until the end of this block; `new_acl` stays valid until freed below.
        let ok = unsafe {
            InitializeSecurityDescriptor(sd_ptr, 1) != 0
                && SetSecurityDescriptorDacl(sd_ptr, 1, new_acl, 0) != 0
                && SetFileSecurityW(name.as_ptr(), DACL_SECURITY_INFORMATION, sd_ptr) != 0
        };
        if ok {
            Ok(())
        } else {
            Err(last_error())
        }
    };
    // SAFETY: both were allocated by the calls above and are freed once.
    unsafe {
        if !new_acl.is_null() {
            LocalFree(new_acl.cast());
        }
        LocalFree(descriptor);
    }
    result
}

/// Adds (`grant`) or removes the SID's entry on `path`'s DACL.
fn change_acl(path: &Path, sid: PSID, mask: u32, grant: bool) -> io::Result<()> {
    let name = wide(path.as_os_str());
    let mut dacl = null_mut();
    let mut descriptor = null_mut();
    // SAFETY: a NUL-terminated path; the out pointers receive memory we free below.
    let err = unsafe {
        GetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            &mut dacl,
            null_mut(),
            &mut descriptor,
        )
    };
    if err != 0 {
        return Err(io::Error::from_raw_os_error(err as i32));
    }
    // SAFETY: an all-zero EXPLICIT_ACCESS_W is a valid start.
    let mut access: EXPLICIT_ACCESS_W = unsafe { std::mem::zeroed() };
    access.grfAccessPermissions = mask;
    access.grfAccessMode = if grant { GRANT_ACCESS } else { REVOKE_ACCESS };
    access.grfInheritance = INHERIT_ALL;
    // SAFETY: `sid` is a valid SID that outlives the call.
    unsafe { BuildTrusteeWithSidW(&mut access.Trustee, sid) };
    let mut new_acl = null_mut();
    // SAFETY: one entry, the existing DACL (kept alive by `descriptor`).
    let err = unsafe { SetEntriesInAclW(1, &access, dacl, &mut new_acl) };
    let result = if err != 0 {
        Err(io::Error::from_raw_os_error(err as i32))
    } else {
        // SAFETY: a NUL-terminated path and a valid ACL.
        let err = unsafe {
            SetNamedSecurityInfoW(
                name.as_ptr().cast_mut(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                new_acl,
                null(),
            )
        };
        if err == 0 {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(err as i32))
        }
    };
    // SAFETY: both were allocated by the calls above and are freed once.
    unsafe {
        if !new_acl.is_null() {
            LocalFree(new_acl.cast());
        }
        LocalFree(descriptor);
    }
    result
}

/// Where the ledgers live: one `<sid>.acl` file per running agent.
fn ledger_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("agora")
        .join("acl-ledger")
}

/// A process's creation time as one number (two processes never share a pid *and* a start time).
fn start_time(process: HANDLE) -> Option<u64> {
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
    // SAFETY: a live handle with query access and four out structures.
    let ok = unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) };
    (ok != 0).then(|| (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
}

/// The first line of a ledger: the node process that owns it, so another node's recovery leaves it alone while that process lives.
fn owner_line() -> String {
    // SAFETY: plain calls on this process.
    let (pid, started) = unsafe {
        (
            GetCurrentProcessId(),
            start_time(GetCurrentProcess()).unwrap_or(0),
        )
    };
    format!("#owner\t{pid}\t{started}\n")
}

/// True while the process named by a ledger's `#owner` line is still running (same pid and same start time).
fn owner_alive(line: &str) -> bool {
    let mut parts = line.split('\t');
    let (Some("#owner"), Some(pid), Some(started)) = (parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    let (Ok(pid), Ok(started)) = (pid.parse::<u32>(), started.parse::<u64>()) else {
        return false;
    };
    // SAFETY: opening a process for a limited query; the handle is closed below.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return false;
    }
    let mut code = 0u32;
    // SAFETY: a live handle and an out value.
    let running = unsafe { GetExitCodeProcess(handle, &mut code) } != 0 && code == 259;
    let same = start_time(handle) == Some(started);
    // SAFETY: opened above and closed once.
    unsafe { CloseHandle(handle) };
    running && same
}

fn write_ledger(file: &Path, sid: &str, paths: &[PathBuf], here: &[PathBuf]) -> io::Result<()> {
    let mut text = owner_line();
    for p in paths {
        text.push_str(sid);
        text.push('\t');
        text.push_str(&p.to_string_lossy());
        text.push('\n');
    }
    for p in here {
        text.push_str(&format!("{sid}\t{}\t{HERE_MARK}\n", p.to_string_lossy()));
    }
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(file, text)
}

/// One agent's confinement: its SID, the folders that carry it, and the token to start the process with. Dropping it removes the
/// entries again.
pub(crate) struct Confinement {
    sid: Sid,
    granted: Vec<PathBuf>,
    /// Folders above the granted ones, each with an entry on that folder only.
    granted_here: Vec<PathBuf>,
    ledger: PathBuf,
    token: Token,
}

impl Confinement {
    pub(crate) fn new(sandbox: &Sandbox) -> io::Result<Self> {
        Self::in_ledger_dir(sandbox, &ledger_dir())
    }

    fn in_ledger_dir(sandbox: &Sandbox, dir: &Path) -> io::Result<Self> {
        let sid_text = random_sid_string()?;
        let sid = Sid::from_string(&sid_text)?;
        let ledger = dir.join(format!("{sid_text}.acl"));
        // Folders that exist, read ones first; a folder that is also writable keeps the stronger entry.
        let mut wanted: Vec<(PathBuf, u32)> = Vec::new();
        for p in &sandbox.read {
            if p.exists() && !sandbox.write.contains(p) {
                wanted.push((p.clone(), READ_MASK));
            }
        }
        for p in &sandbox.write {
            if p.exists() {
                wanted.push((p.clone(), WRITE_MASK));
            }
        }
        // Recorded before anything is changed: a crash between the two leaves a ledger that recovery can undo.
        let all: Vec<PathBuf> = wanted.iter().map(|(p, _)| p.clone()).collect();
        let above = ancestors_of(&all);
        write_ledger(&ledger, &sid_text, &all, &above)?;
        let mut confinement = Self {
            sid,
            granted: Vec::new(),
            granted_here: Vec::new(),
            ledger,
            token: Token(null_mut()),
        };
        for (path, mask) in wanted {
            // On failure `confinement` is dropped, which undoes what was added so far.
            change_acl(&path, confinement.sid.0, mask, true)?;
            confinement.granted.push(path);
        }
        for path in above {
            // A folder whose permissions the user may not change (`C:\Users`, owned by the system) is left as it is: the user could
            // not have opened it to the agent anyway, and such folders already let every user read their attributes.
            match change_acl_here(&path, confinement.sid.0, ANCESTOR_MASK, true) {
                Ok(()) => confinement.granted_here.push(path),
                Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {}
                Err(e) => return Err(e),
            }
        }
        confinement.token = restricted_token(&confinement.sid)?;
        Ok(confinement)
    }

    /// The primary token to start the agent's process with.
    pub(crate) fn token(&self) -> HANDLE {
        self.token.0
    }
}

impl Drop for Confinement {
    fn drop(&mut self) {
        let mut all_removed = true;
        for path in &self.granted {
            if change_acl(path, self.sid.0, 0, false).is_err() {
                all_removed = false;
            }
        }
        for path in &self.granted_here {
            if change_acl_here(path, self.sid.0, 0, false).is_err() {
                all_removed = false;
            }
        }
        // A failed removal keeps the ledger so the next start retries; the SID dies with the agent either way.
        if all_removed {
            let _ = std::fs::remove_file(&self.ledger);
        }
    }
}

/// The current process's token, restricted: every privilege dropped, and the restricting list is the agent's SID, `Everyone`,
/// `Users` and the logon SID (without that last one the console and desktop refuse the process).
fn restricted_token(agent: &Sid) -> io::Result<Token> {
    let mut own: HANDLE = null_mut();
    // SAFETY: the pseudo handle of this process and an out pointer.
    if unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_DUPLICATE | TOKEN_QUERY | TOKEN_ASSIGN_PRIMARY,
            &mut own,
        )
    } == 0
    {
        return Err(last_error());
    }
    let own = Token(own);

    let everyone = Sid::from_string("S-1-1-0")?;
    let users = Sid::from_string("S-1-5-32-545")?;
    // The logon SID: the one group of the token flagged as the logon session.
    let mut needed = 0u32;
    // SAFETY: the documented size query.
    unsafe { GetTokenInformation(own.0, TokenLogonSid, null_mut(), 0, &mut needed) };
    let mut buf = vec![0u8; (needed as usize).max(16)];
    // SAFETY: `buf` is `needed` bytes.
    if unsafe {
        GetTokenInformation(
            own.0,
            TokenLogonSid,
            buf.as_mut_ptr().cast(),
            buf.len() as u32,
            &mut needed,
        )
    } == 0
    {
        return Err(last_error());
    }
    // SAFETY: GetTokenInformation(TokenLogonSid) wrote a TOKEN_GROUPS at the start of `buf`.
    let groups = unsafe { &*(buf.as_ptr().cast::<TOKEN_GROUPS>()) };
    let mut list = vec![
        SID_AND_ATTRIBUTES {
            Sid: agent.0,
            Attributes: 0,
        },
        SID_AND_ATTRIBUTES {
            Sid: everyone.0,
            Attributes: 0,
        },
        SID_AND_ATTRIBUTES {
            Sid: users.0,
            Attributes: 0,
        },
    ];
    if groups.GroupCount > 0 {
        list.push(SID_AND_ATTRIBUTES {
            Sid: groups.Groups[0].Sid,
            Attributes: 0,
        });
    }
    let mut restricted: HANDLE = null_mut();
    // SAFETY: `list` and the SIDs it points to outlive the call.
    let ok = unsafe {
        CreateRestrictedToken(
            own.0,
            DISABLE_MAX_PRIVILEGE,
            0,
            null(),
            0,
            null(),
            list.len() as u32,
            list.as_ptr(),
            &mut restricted,
        )
    };
    if ok == 0 {
        return Err(last_error());
    }
    Ok(Token(restricted))
}

/// Removes the entries a crashed node left behind. Call it once when the node starts, before any agent is running.
pub(crate) fn recover_stale() {
    let Ok(dir) = std::fs::read_dir(ledger_dir()) else {
        return;
    };
    for entry in dir.flatten() {
        let path = entry.path();
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        // A ledger of a node that is still running belongs to agents that need their permissions.
        if text.lines().next().is_some_and(owner_alive) {
            continue;
        }
        let mut clean = true;
        for line in text.lines() {
            let Some((sid_text, folder)) = line.split_once('\t') else {
                continue;
            };
            if sid_text == "#owner" {
                continue;
            }
            let Ok(sid) = Sid::from_string(sid_text) else {
                continue;
            };
            let (folder, here) = match folder.rsplit_once('\t') {
                Some((f, HERE_MARK)) => (f, true),
                _ => (folder, false),
            };
            let folder = Path::new(folder);
            // A folder that is gone has nothing to undo.
            let undone = if here {
                // Skipped at grant time for the same reason (see `in_ledger_dir`): nothing to undo.
                change_acl_here(folder, sid.0, 0, false).or_else(|e| {
                    (e.kind() == io::ErrorKind::PermissionDenied)
                        .then_some(())
                        .ok_or(e)
                })
            } else {
                change_acl(folder, sid.0, 0, false)
            };
            if folder.exists() && undone.is_err() {
                clean = false;
            }
        }
        if clean {
            let _ = std::fs::remove_file(&path);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_ledger_is_alive_while_its_owner_runs_and_stale_otherwise() {
        // This very process owns the line it just wrote.
        assert!(owner_alive(owner_line().trim_end()));
        // A pid nobody has, a wrong start time for a live pid, and a line that is not an owner line.
        assert!(!owner_alive("#owner\t4294967290\t1"));
        let pid = std::process::id();
        assert!(!owner_alive(&format!("#owner\t{pid}\t1")));
        assert!(!owner_alive("S-1-5-21-1-2-3-4\tC:\\x"));
        assert!(!owner_alive(""));
    }
}
