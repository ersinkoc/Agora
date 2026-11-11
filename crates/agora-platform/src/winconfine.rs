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
    BuildTrusteeWithSidW, ConvertSidToStringSidW,
    ConvertStringSecurityDescriptorToSecurityDescriptorW, ConvertStringSidToSidW,
    GetNamedSecurityInfoW, SetEntriesInAclW, SetNamedSecurityInfoW, EXPLICIT_ACCESS_W,
    GRANT_ACCESS, REVOKE_ACCESS, SDDL_REVISION_1, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::Cryptography::{
    BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
};
use windows_sys::Win32::Security::{
    CreateRestrictedToken, EqualSid, GetAce, GetSecurityDescriptorControl,
    GetSecurityDescriptorSacl, GetTokenInformation, InitializeSecurityDescriptor, SetFileSecurityW,
    SetSecurityDescriptorControl, SetSecurityDescriptorDacl, SetTokenInformation,
    TokenIntegrityLevel, TokenLogonSid, ACCESS_ALLOWED_ACE, ACE_HEADER, ACL,
    DACL_SECURITY_INFORMATION, DISABLE_MAX_PRIVILEGE, LABEL_SECURITY_INFORMATION, PSID,
    SECURITY_DESCRIPTOR, SID_AND_ATTRIBUTES, TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_GROUPS,
    TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
};
use windows_sys::Win32::Storage::FileSystem::{
    DELETE, FILE_DELETE_CHILD, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
    FILE_READ_ATTRIBUTES, FILE_TRAVERSE,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetExitCodeProcess, GetProcessTimes, OpenProcess,
    OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};

use crate::{Sandbox, SandboxRefusal};

fn refused(why: SandboxRefusal) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, why)
}

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
/// What a folder above a granted one gets (Amendment B, 3): its attributes can be read (`stat`) and it can be passed through, on that
/// folder only. git `stat`s every folder above a ref it locks. `SYNCHRONIZE` (0x0010_0000) comes with every open.
const ANCESTOR_MASK: u32 = FILE_READ_ATTRIBUTES | FILE_TRAVERSE | 0x0010_0000;
/// `WRITE_OWNER`: needed to set a mandatory label. Granted for a while to the node's logon SID where the user lacks it (Amendment B, 2).
const WRITE_OWNER: u32 = 0x0008_0000;
/// The groups whose *write* grant on an allowed folder is refused before start (Amendment B, 4, decision (A)). Adding
/// `Authenticated Users` (`S-1-5-11`) here is decision (B).
const REFUSED_WRITERS: [&str; 2] = ["S-1-5-32-545", "S-1-1-0"];
/// Access bits that let a group change or add files.
const ANY_WRITE: u32 = 0x2 /* FILE_WRITE_DATA / ADD_FILE */
    | 0x4 /* FILE_APPEND_DATA / ADD_SUBDIRECTORY */
    | 0x40 /* FILE_DELETE_CHILD */
    | 0x0001_0000 /* DELETE */
    | 0x0004_0000 /* WRITE_DAC */
    | WRITE_OWNER
    | 0x1000_0000 /* GENERIC_ALL */
    | 0x4000_0000 /* GENERIC_WRITE */;
/// The one switch between the two readings of Amendment B for a folder whose label the user cannot set (a data volume's default ACL,
/// Modify through `Authenticated Users` only): `true` gives the node's logon SID `WRITE_OWNER` there while the label is needed (so a
/// checkout on `D:` works); `false` refuses such a folder before start, as decision 2 reads. `AGORA_WINDOWS_NO_SELF_GRANT=1` turns it
/// off for one run, to measure both.
const SELF_GRANT_WRITE_OWNER: bool = true;

fn self_grant_enabled() -> bool {
    SELF_GRANT_WRITE_OWNER
        && !std::env::var_os("AGORA_WINDOWS_NO_SELF_GRANT").is_some_and(|v| v == "1")
}

/// Ledger line markers.
const HERE_MARK: &str = "here";
const LABEL_LINE: &str = "#label";
const WO_LINE: &str = "#wo";

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
pub(crate) fn random_sid_string() -> io::Result<String> {
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

/// The folders above `paths`, without the drive root and without the paths themselves, each once.
fn ancestors_of(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for p in paths {
        for a in p.ancestors().skip(1) {
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

/// Adds (`grant`) or removes the SID's entry on `path` itself, not inherited, written with `SetFileSecurityW`, which does not walk
/// the folder's children. The descriptor keeps the folder's own DACL control bits (`SE_DACL_AUTO_INHERITED`, `SE_DACL_PROTECTED`),
/// so the folder's inheritance state is unchanged.
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
    access.grfInheritance = 0;
    // SAFETY: `sid` is a valid SID that outlives the call.
    unsafe { BuildTrusteeWithSidW(&mut access.Trustee, sid) };
    let mut new_acl = null_mut();
    // SAFETY: one entry, the existing DACL (kept alive by `descriptor`).
    let err = unsafe { SetEntriesInAclW(1, &access, dacl, &mut new_acl) };
    let result = if err != 0 {
        Err(io::Error::from_raw_os_error(err as i32))
    } else {
        let (mut control, mut revision) = (0u16, 0u32);
        // SAFETY: `descriptor` is a valid descriptor from the call above.
        unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) };
        // SE_DACL_AUTO_INHERIT_REQ | SE_DACL_AUTO_INHERITED | SE_DACL_PROTECTED
        let keep = 0x0100 | 0x0400 | 0x1000;
        // SAFETY: an all-zero descriptor is initialized by the call before use.
        let mut sd: SECURITY_DESCRIPTOR = unsafe { std::mem::zeroed() };
        let sd_ptr = (&mut sd as *mut SECURITY_DESCRIPTOR).cast();
        // SAFETY: `sd` lives until the end of this block; `new_acl` stays valid until freed below.
        let ok = unsafe {
            InitializeSecurityDescriptor(sd_ptr, 1) != 0
                && SetSecurityDescriptorDacl(sd_ptr, 1, new_acl, 0) != 0
                && SetSecurityDescriptorControl(sd_ptr, keep, control & keep) != 0
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

/// The logon SID of this process's token (the node's), as owned bytes.
struct LogonSid(Vec<u8>);

impl LogonSid {
    fn of_this_process() -> io::Result<Self> {
        let mut own: HANDLE = null_mut();
        // SAFETY: the pseudo handle of this process and an out pointer.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut own) } == 0 {
            return Err(last_error());
        }
        let own = Token(own);
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
        // SAFETY: a TOKEN_GROUPS at the start of `buf`.
        let groups = unsafe { &*(buf.as_ptr().cast::<TOKEN_GROUPS>()) };
        if groups.GroupCount == 0 {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        Ok(Self(buf))
    }

    fn psid(&self) -> PSID {
        // SAFETY: checked in `of_this_process`: one group, whose SID points into `self.0`.
        unsafe { (*(self.0.as_ptr().cast::<TOKEN_GROUPS>())).Groups[0].Sid }
    }

    /// `S-1-5-5-x-y`, for the ledger: after a reboot the node runs in another logon session, and recovery must undo the old one's grant.
    fn text(&self) -> io::Result<String> {
        let mut s: *mut u16 = null_mut();
        // SAFETY: a valid SID and an out pointer to a LocalAlloc'd string.
        if unsafe { ConvertSidToStringSidW(self.psid(), &mut s) } == 0 {
            return Err(last_error());
        }
        // SAFETY: a NUL-terminated string from the call above.
        let len = (0..).take_while(|&i| unsafe { *s.add(i) } != 0).count();
        // SAFETY: `len` u16s are valid at `s`.
        let text = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(s, len) });
        // SAFETY: allocated by ConvertSidToStringSidW.
        unsafe { LocalFree(s.cast()) };
        Ok(text)
    }
}

/// Puts (`low`) or removes the inheritable Low mandatory label (no write up) on `path`. Windows passes it on to the files below.
fn set_low_label(path: &Path, low: bool) -> io::Result<()> {
    let sddl: &std::ffi::OsStr = if low {
        "S:(ML;OICI;NW;;;LW)".as_ref()
    } else {
        "S:".as_ref()
    };
    let mut sd = null_mut();
    // SAFETY: a valid NUL-terminated string and an out pointer.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            wide(sddl).as_ptr(),
            SDDL_REVISION_1,
            &mut sd,
            null_mut(),
        )
    } == 0
    {
        return Err(last_error());
    }
    let (mut present, mut defaulted, mut sacl) = (0i32, 0i32, null_mut::<ACL>());
    // SAFETY: `sd` is valid until freed below.
    unsafe { GetSecurityDescriptorSacl(sd, &mut present, &mut sacl, &mut defaulted) };
    let name = wide(path.as_os_str());
    // SAFETY: a NUL-terminated path and a valid (possibly empty) SACL.
    let err = unsafe {
        SetNamedSecurityInfoW(
            name.as_ptr().cast_mut(),
            SE_FILE_OBJECT,
            LABEL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            null(),
            sacl,
        )
    };
    // SAFETY: allocated by the conversion above.
    unsafe { LocalFree(sd) };
    if err == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(err as i32))
    }
}

/// Whether `path` carries an explicit (not inherited) *Low* mandatory label: what Windows gives an object a Low process creates where no
/// label is inherited.
fn has_explicit_low_label(path: &Path) -> bool {
    let name = wide(path.as_os_str());
    let (mut sacl, mut descriptor) = (null_mut::<ACL>(), null_mut());
    // SAFETY: a NUL-terminated path; out pointers freed below.
    let err = unsafe {
        GetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            LABEL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            null_mut(),
            &mut sacl,
            &mut descriptor,
        )
    };
    let mut found = false;
    if err == 0 && !sacl.is_null() {
        if let Ok(low) = Sid::from_string("S-1-16-4096") {
            // SAFETY: a valid ACL from the call above.
            let count = unsafe { (*sacl).AceCount };
            for i in 0..u32::from(count) {
                let mut ace = null_mut();
                // SAFETY: index within AceCount.
                if unsafe { GetAce(sacl, i, &mut ace) } == 0 {
                    continue;
                }
                // SAFETY: every ACE starts with an ACE_HEADER; a mandatory label ACE (type 0x11) has the ACCESS_ALLOWED_ACE layout.
                let header = unsafe { &*(ace.cast::<ACE_HEADER>()) };
                if header.AceType != 0x11 || header.AceFlags & 0x10 != 0 {
                    continue;
                }
                // SAFETY: as above.
                let label = unsafe { &*(ace.cast::<ACCESS_ALLOWED_ACE>()) };
                let sid: PSID = (&label.SidStart as *const u32).cast_mut().cast();
                // SAFETY: both are valid SIDs.
                if unsafe { EqualSid(sid, low.0) } != 0 {
                    found = true;
                }
            }
        }
    }
    if !descriptor.is_null() {
        // SAFETY: allocated by GetNamedSecurityInfoW.
        unsafe { LocalFree(descriptor) };
    }
    found
}

/// Removes the explicit Low labels below `root` one object at a time (`SetFileSecurityW`, no walk of its own). Called after the folder's
/// own label is gone: anything a Low agent created where no label was inherited got an explicit one, which taking back the folder's label
/// does not reach, and which would leave it writable to later Low processes.
fn clear_explicit_low_labels(root: &Path) -> bool {
    // An empty SACL: no label, so the object is Medium again (or inherits what is above it).
    let mut acl_buf = [0u32; 4];
    let acl = acl_buf.as_mut_ptr().cast::<ACL>();
    // SAFETY: an all-zero descriptor is initialized by the call below before use.
    let mut sd: SECURITY_DESCRIPTOR = unsafe { std::mem::zeroed() };
    let sd_ptr = (&mut sd as *mut SECURITY_DESCRIPTOR).cast();
    // SAFETY: a 16-byte buffer holds an empty ACL; `acl_buf` and `sd` outlive every use below.
    let ready = unsafe {
        windows_sys::Win32::Security::InitializeAcl(acl, 16, 2) != 0
            && InitializeSecurityDescriptor(sd_ptr, 1) != 0
            && windows_sys::Win32::Security::SetSecurityDescriptorSacl(sd_ptr, 1, acl, 0) != 0
    };
    if !ready {
        return false;
    }
    let mut clean = true;
    let mut stack = vec![root.to_path_buf()];
    while let Some(p) = stack.pop() {
        if p != root && has_explicit_low_label(&p) {
            let name = wide(p.as_os_str());
            // SAFETY: a NUL-terminated path and the descriptor built above.
            if unsafe { SetFileSecurityW(name.as_ptr(), LABEL_SECURITY_INFORMATION, sd_ptr) } == 0 {
                clean = false;
            }
        }
        let Ok(meta) = std::fs::symlink_metadata(&p) else {
            continue;
        };
        if meta.is_dir() && !meta.file_type().is_symlink() {
            if let Ok(entries) = std::fs::read_dir(&p) {
                stack.extend(entries.flatten().map(|e| e.path()));
            }
        }
    }
    clean
}

/// Whether `path` carries an explicit (not inherited) mandatory label already.
fn has_explicit_label(path: &Path) -> bool {
    let name = wide(path.as_os_str());
    let (mut sacl, mut descriptor) = (null_mut::<ACL>(), null_mut());
    // SAFETY: a NUL-terminated path; out pointers freed below.
    let err = unsafe {
        GetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            LABEL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            null_mut(),
            &mut sacl,
            &mut descriptor,
        )
    };
    let mut explicit = false;
    if err == 0 && !sacl.is_null() {
        // SAFETY: a valid ACL from the call above.
        let count = unsafe { (*sacl).AceCount };
        for i in 0..u32::from(count) {
            let mut ace = null_mut();
            // SAFETY: index within AceCount.
            if unsafe { GetAce(sacl, i, &mut ace) } != 0 {
                // SAFETY: every ACE starts with an ACE_HEADER; INHERITED_ACE is 0x10.
                let header = unsafe { &*(ace.cast::<ACE_HEADER>()) };
                if header.AceFlags & 0x10 == 0 {
                    explicit = true;
                }
            }
        }
    }
    if !descriptor.is_null() {
        // SAFETY: allocated by GetNamedSecurityInfoW.
        unsafe { LocalFree(descriptor) };
    }
    explicit
}

/// The first file or folder at or below `root` whose own DACL lets `Users` or `Everyone` write (Amendment B, 4). Walks the whole tree:
/// a single child with such a grant would let any confined agent write there through the Low label.
fn broad_writer_below(root: &Path) -> io::Result<Option<(PathBuf, SandboxRefusal)>> {
    let refused: Vec<Sid> = REFUSED_WRITERS
        .iter()
        .map(|s| Sid::from_string(s))
        .collect::<io::Result<_>>()?;
    let need_owner = !self_grant_enabled();
    let mut stack = vec![root.to_path_buf()];
    while let Some(p) = stack.pop() {
        // Without the self-grant, a file or folder whose label the user cannot set is refused here, before anything changes.
        if grants_write_to(&p, &refused) {
            return Ok(Some((p, SandboxRefusal::BroadWriteGrant)));
        }
        if need_owner && !may_write_owner(&p) {
            return Ok(Some((p, SandboxRefusal::LabelFailed)));
        }
        // A reparse point (junction, link) is not followed: its target is not part of the allowed folder.
        let meta = std::fs::symlink_metadata(&p)?;
        if meta.is_dir() && !meta.file_type().is_symlink() {
            for e in std::fs::read_dir(&p)?.flatten() {
                stack.push(e.path());
            }
        }
    }
    Ok(None)
}

/// Whether this process may change `path`'s owner and label (`WRITE_OWNER`): an open asking for just that right says so.
fn may_write_owner(path: &Path) -> bool {
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    let name = wide(path.as_os_str());
    // SAFETY: a NUL-terminated path; the handle is closed below.
    let h = unsafe {
        CreateFileW(
            name.as_ptr(),
            WRITE_OWNER,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            null_mut(),
        )
    };
    if h == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
        return false;
    }
    // SAFETY: opened above, closed once.
    unsafe { CloseHandle(h) };
    true
}

/// Whether `path`'s DACL has an explicit (not inherited) allow entry for `sid`.
fn has_explicit_ace_for(path: &Path, sid: PSID) -> bool {
    let name = wide(path.as_os_str());
    let (mut dacl, mut descriptor) = (null_mut::<ACL>(), null_mut());
    // SAFETY: a NUL-terminated path; out pointers freed below.
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
    let mut found = false;
    if err == 0 && !dacl.is_null() {
        // SAFETY: a valid ACL from the call above.
        let count = unsafe { (*dacl).AceCount };
        for i in 0..u32::from(count) {
            let mut ace = null_mut();
            // SAFETY: index within AceCount.
            if unsafe { GetAce(dacl, i, &mut ace) } == 0 {
                continue;
            }
            // SAFETY: every ACE starts with an ACE_HEADER; INHERITED_ACE is 0x10.
            let header = unsafe { &*(ace.cast::<ACE_HEADER>()) };
            if header.AceType != 0 || header.AceFlags & 0x10 != 0 {
                continue;
            }
            // SAFETY: type 0 is an ACCESS_ALLOWED_ACE.
            let allowed = unsafe { &*(ace.cast::<ACCESS_ALLOWED_ACE>()) };
            let ace_sid: PSID = (&allowed.SidStart as *const u32).cast_mut().cast();
            // SAFETY: both are valid SIDs.
            if unsafe { EqualSid(ace_sid, sid) } != 0 {
                found = true;
                break;
            }
        }
    }
    if !descriptor.is_null() {
        // SAFETY: allocated by GetNamedSecurityInfoW.
        unsafe { LocalFree(descriptor) };
    }
    found
}

fn grants_write_to(path: &Path, sids: &[Sid]) -> bool {
    let name = wide(path.as_os_str());
    let (mut dacl, mut descriptor) = (null_mut::<ACL>(), null_mut());
    // SAFETY: a NUL-terminated path; out pointers freed below.
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
    let mut found = false;
    if err == 0 && !dacl.is_null() {
        // SAFETY: a valid ACL from the call above.
        let count = unsafe { (*dacl).AceCount };
        for i in 0..u32::from(count) {
            let mut ace = null_mut();
            // SAFETY: index within AceCount.
            if unsafe { GetAce(dacl, i, &mut ace) } == 0 {
                continue;
            }
            // SAFETY: every ACE starts with an ACE_HEADER.
            let header = unsafe { &*(ace.cast::<ACE_HEADER>()) };
            // An allow entry (type 0) that applies to this object (not INHERIT_ONLY, 0x08).
            if header.AceType != 0 || header.AceFlags & 0x08 != 0 {
                continue;
            }
            // SAFETY: type 0 is an ACCESS_ALLOWED_ACE; its SID starts at `SidStart`.
            let allowed = unsafe { &*(ace.cast::<ACCESS_ALLOWED_ACE>()) };
            if allowed.Mask & ANY_WRITE == 0 {
                continue;
            }
            let ace_sid: PSID = (&allowed.SidStart as *const u32).cast_mut().cast();
            // SAFETY: both are valid SIDs.
            if sids.iter().any(|s| unsafe { EqualSid(ace_sid, s.0) } != 0) {
                found = true;
                break;
            }
        }
    }
    if !descriptor.is_null() {
        // SAFETY: allocated by GetNamedSecurityInfoW.
        unsafe { LocalFree(descriptor) };
    }
    found
}

/// Amendment B, 4: refuses a profile whose writable folders (or anything below them) let `Users` or `Everyone` write. Returns the
/// first such path.
pub(crate) fn preflight(sandbox: &Sandbox) -> Result<(), (PathBuf, SandboxRefusal)> {
    for p in sandbox.write.iter().filter(|p| p.exists()) {
        match broad_writer_below(p) {
            Ok(None) => {}
            Ok(Some(hit)) => return Err(hit),
            // A folder whose permissions cannot be read cannot be promised either.
            Err(_) => return Err((p.clone(), SandboxRefusal::BroadWriteGrant)),
        }
    }
    Ok(())
}

/// Serializes every decision about a shared folder's label (planning and labelling at start, removal at stop and in recovery) across
/// all nodes of this user: otherwise an agent ending while another starts in the same folder can remove a label the starting agent
/// has just counted on.
struct LedgerLock(HANDLE);

impl LedgerLock {
    fn acquire() -> Option<Self> {
        use windows_sys::Win32::System::Threading::{CreateMutexW, WaitForSingleObject, INFINITE};
        let name = wide("Local\\agora-acl-ledger".as_ref());
        // SAFETY: a NUL-terminated name; no security attributes.
        let h = unsafe { CreateMutexW(null(), 0, name.as_ptr()) };
        if h.is_null() {
            return None;
        }
        // SAFETY: a valid mutex handle. WAIT_ABANDONED (a crashed holder) also grants ownership.
        unsafe { WaitForSingleObject(h, INFINITE) };
        Some(Self(h))
    }
}

impl Drop for LedgerLock {
    fn drop(&mut self) {
        use windows_sys::Win32::System::Threading::ReleaseMutex;
        // SAFETY: owned since `acquire`, released and closed once.
        unsafe {
            ReleaseMutex(self.0);
            CloseHandle(self.0);
        }
    }
}

/// Whether another ledger whose node still runs lists exactly `line` (a shared folder's label or self-grant is still in use).
fn line_in_use_elsewhere(line: &str, own: &Path) -> bool {
    let Ok(dir) = std::fs::read_dir(ledger_dir()) else {
        return false;
    };
    for e in dir.flatten() {
        let path = e.path();
        if path == own {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if text.lines().next().is_some_and(owner_alive) && text.lines().any(|l| l == line) {
            return true;
        }
    }
    false
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

/// What one agent changes, written to its ledger *before* any of it is done.
struct Plan<'a> {
    sid: &'a str,
    grants: &'a [PathBuf],
    above: &'a [PathBuf],
    labels: &'a [PathBuf],
    /// The node's logon SID, which may get `WRITE_OWNER` on the labelled folders.
    logon: &'a str,
}

fn wo_line(path: &Path, logon: &str) -> String {
    format!("{WO_LINE}\t{}\t{logon}", path.to_string_lossy())
}

fn write_ledger(file: &Path, plan: &Plan) -> io::Result<()> {
    let mut text = owner_line();
    for p in plan.grants {
        text.push_str(&format!("{}\t{}\n", plan.sid, p.to_string_lossy()));
    }
    for p in plan.above {
        text.push_str(&format!(
            "{}\t{}\t{HERE_MARK}\n",
            plan.sid,
            p.to_string_lossy()
        ));
    }
    // A label (and the self-grant that may be needed to set it) is per folder, shared by the agents that use it.
    for p in plan.labels {
        text.push_str(&wo_line(p, plan.logon));
        text.push('\n');
        text.push_str(&format!("{LABEL_LINE}\t{}\n", p.to_string_lossy()));
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
    /// Folders this agent labelled Low (or found labelled by a live agent and shares).
    labelled: Vec<PathBuf>,
    logon: Option<LogonSid>,
    logon_text: String,
    ledger: PathBuf,
    token: Token,
}

impl Confinement {
    /// `identity` is the agent's SID from [`crate::new_agent_identity`], made before its mesh endpoint so both name the same SID; `None`
    /// makes a fresh one.
    pub(crate) fn new(sandbox: &Sandbox, identity: Option<&str>) -> io::Result<Self> {
        Self::in_ledger_dir(sandbox, &ledger_dir(), identity)
    }

    fn in_ledger_dir(sandbox: &Sandbox, dir: &Path, identity: Option<&str>) -> io::Result<Self> {
        // Amendment B, 4: refused before anything changes, so there is nothing to roll back. The caller maps `PermissionDenied` to
        // `agent.sandbox_unavailable`.
        if let Err((_, why)) = preflight(sandbox) {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, why));
        }
        let sid_text = match identity {
            Some(s) => s.to_string(),
            None => random_sid_string()?,
        };
        let sid = Sid::from_string(&sid_text)?;
        // Held until every label of this agent is in place (see `LedgerLock`).
        let _lock = LedgerLock::acquire();
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
        // A folder the user labelled on purpose is left as it is (and not undone later). One that a live agent labelled (a shared
        // `objects`, `refs`) is recorded here too, so the label stays until the last agent using it has ended.
        let label_line = |p: &Path| format!("{LABEL_LINE}\t{}", p.to_string_lossy());
        let mut shared: Vec<PathBuf> = Vec::new();
        let labels: Vec<PathBuf> = wanted
            .iter()
            .filter(|(_, m)| *m == WRITE_MASK)
            .filter_map(|(p, _)| {
                if !has_explicit_label(p) {
                    Some(p.clone())
                } else if line_in_use_elsewhere(&label_line(p), &ledger) {
                    shared.push(p.clone());
                    Some(p.clone())
                } else {
                    None
                }
            })
            .collect();
        let logon = LogonSid::of_this_process().ok();
        let logon_text = logon
            .as_ref()
            .and_then(|l| l.text().ok())
            .unwrap_or_default();
        write_ledger(
            &ledger,
            &Plan {
                sid: &sid_text,
                grants: &all,
                above: &above,
                labels: &labels,
                logon: &logon_text,
            },
        )?;
        let mut confinement = Self {
            sid,
            granted: Vec::new(),
            granted_here: Vec::new(),
            labelled: Vec::new(),
            logon,
            logon_text,
            ledger,
            token: Token(null_mut()),
        };
        for (path, mask) in wanted {
            // On failure `confinement` is dropped, which undoes what was added so far.
            change_acl(&path, confinement.sid.0, mask, true)?;
            confinement.granted.push(path);
        }
        for path in above {
            // A folder whose permissions the user may not change (`C:\Users`) is left alone: `Users` in the restricting list covers it.
            match change_acl_here(&path, confinement.sid.0, ANCESTOR_MASK, true) {
                Ok(()) => confinement.granted_here.push(path),
                Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {}
                Err(e) => return Err(e),
            }
        }
        for path in labels {
            if shared.contains(&path) {
                // Already labelled by a live agent: share it, do not walk the tree again.
                confinement.labelled.push(path);
                continue;
            }
            if set_low_label(&path, true).is_err() {
                if !self_grant_enabled() {
                    return Err(refused(SandboxRefusal::LabelFailed));
                }
                // The owner may change the DACL but not the owner/label (a data volume's default ACL): give the node's logon SID
                // `WRITE_OWNER` on the tree for as long as the label is there, then label again.
                let logon = confinement
                    .logon
                    .as_ref()
                    .ok_or_else(|| refused(SandboxRefusal::LabelFailed))?;
                change_acl(&path, logon.psid(), WRITE_OWNER, true)?;
                // A folder that cannot be labelled even so is refused (Amendment B, 2).
                set_low_label(&path, true).map_err(|_| refused(SandboxRefusal::LabelFailed))?;
            }
            confinement.labelled.push(path);
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
        let _lock = LedgerLock::acquire();
        // Labels first (setting them may need the self-grant), each only when no other live agent still uses that folder.
        for path in &self.labelled {
            let line = format!("{LABEL_LINE}\t{}", path.to_string_lossy());
            if !line_in_use_elsewhere(&line, &self.ledger)
                && (set_low_label(path, false).is_err() || !clear_explicit_low_labels(path))
            {
                all_removed = false;
            }
        }
        // The self-grant belongs to the folder, like the label: whichever agent ends last takes it back, also when another agent added it.
        if let Some(logon) = &self.logon {
            for path in &self.labelled {
                let line = wo_line(path, &self.logon_text);
                if !line_in_use_elsewhere(&line, &self.ledger)
                    && has_explicit_ace_for(path, logon.psid())
                    && change_acl(path, logon.psid(), 0, false).is_err()
                {
                    all_removed = false;
                }
            }
        }
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

/// The SID of the user this process runs as, as text.
pub(crate) fn current_user_sid_text() -> io::Result<String> {
    use windows_sys::Win32::Security::{TokenUser, TOKEN_USER};
    let mut own: HANDLE = null_mut();
    // SAFETY: the pseudo handle of this process and an out pointer.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut own) } == 0 {
        return Err(last_error());
    }
    let own = Token(own);
    let mut needed = 0u32;
    // SAFETY: the documented size query.
    unsafe { GetTokenInformation(own.0, TokenUser, null_mut(), 0, &mut needed) };
    let mut buf = vec![0u8; (needed as usize).max(std::mem::size_of::<TOKEN_USER>())];
    // SAFETY: `buf` is `needed` bytes.
    if unsafe {
        GetTokenInformation(
            own.0,
            TokenUser,
            buf.as_mut_ptr().cast(),
            buf.len() as u32,
            &mut needed,
        )
    } == 0
    {
        return Err(last_error());
    }
    // SAFETY: GetTokenInformation(TokenUser) wrote a TOKEN_USER at the start of `buf`.
    let user = unsafe { &*(buf.as_ptr().cast::<TOKEN_USER>()) };
    sid_text(user.User.Sid)
}

/// `S-1-...` for a SID.
fn sid_text(sid: PSID) -> io::Result<String> {
    let mut s: *mut u16 = null_mut();
    // SAFETY: a valid SID and an out pointer to a LocalAlloc'd string.
    if unsafe { ConvertSidToStringSidW(sid, &mut s) } == 0 {
        return Err(last_error());
    }
    // SAFETY: a NUL-terminated string from the call above.
    let len = (0..).take_while(|&i| unsafe { *s.add(i) } != 0).count();
    // SAFETY: `len` u16s are valid at `s`.
    let text = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(s, len) });
    // SAFETY: allocated by ConvertSidToStringSidW.
    unsafe { LocalFree(s.cast()) };
    Ok(text)
}

/// The logon SID inside a `TokenLogonSid` buffer (checked non-empty by the caller).
fn groups_logon(buf: &[u8]) -> PSID {
    // SAFETY: a TOKEN_GROUPS with at least one group at the start of `buf`.
    unsafe { (*(buf.as_ptr().cast::<TOKEN_GROUPS>())).Groups[0].Sid }
}

/// Sets `token`'s default DACL (for objects its process creates without one) from an SDDL `D:` string.
fn set_default_dacl(token: HANDLE, sddl: &str) -> io::Result<()> {
    use windows_sys::Win32::Security::{
        GetSecurityDescriptorDacl, TokenDefaultDacl, TOKEN_DEFAULT_DACL,
    };
    let mut sd = null_mut();
    // SAFETY: a valid NUL-terminated string and an out pointer.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            wide(sddl.as_ref()).as_ptr(),
            SDDL_REVISION_1,
            &mut sd,
            null_mut(),
        )
    } == 0
    {
        return Err(last_error());
    }
    let (mut present, mut defaulted, mut dacl) = (0i32, 0i32, null_mut::<ACL>());
    // SAFETY: `sd` is valid until freed below.
    unsafe { GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut defaulted) };
    let info = TOKEN_DEFAULT_DACL { DefaultDacl: dacl };
    // SAFETY: a token we own with default-adjust access; `dacl` lives until `sd` is freed.
    let ok = unsafe {
        SetTokenInformation(
            token,
            TokenDefaultDacl,
            (&info as *const TOKEN_DEFAULT_DACL).cast(),
            std::mem::size_of::<TOKEN_DEFAULT_DACL>() as u32,
        )
    };
    // SAFETY: allocated by the conversion above.
    unsafe { LocalFree(sd) };
    if ok == 0 {
        Err(last_error())
    } else {
        Ok(())
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
            // TOKEN_ADJUST_DEFAULT (0x80): the restricted copy's integrity level is lowered below.
            TOKEN_DUPLICATE | TOKEN_QUERY | TOKEN_ASSIGN_PRIMARY | 0x80,
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
    let restricted = Token(restricted);
    // Amendment B, 2: Low integrity, so anything labelled Medium (everything not labelled) refuses writes whatever its ACL says.
    let low = Sid::from_string("S-1-16-4096")?;
    let label = TOKEN_MANDATORY_LABEL {
        Label: SID_AND_ATTRIBUTES {
            Sid: low.0,
            Attributes: 0x20, // SE_GROUP_INTEGRITY
        },
    };
    // Objects the agent creates itself (the pipes `cmd` makes for `a | b`, events) get the token's default DACL. The user's default
    // names the user and SYSTEM, neither of which passes the restricting check, so the agent could not open what it just made ("Not
    // enough memory resources" from `cmd`). Name the agent's SID and the logon SID there instead.
    let agent_text = sid_text(agent.0)?;
    let logon_ace = if groups.GroupCount > 0 {
        format!("(A;;GA;;;{})", sid_text(groups_logon(&buf))?)
    } else {
        String::new()
    };
    set_default_dacl(
        restricted.0,
        &format!("D:(A;;GA;;;SY)(A;;GA;;;{agent_text}){logon_ace}"),
    )?;
    // The size covers the structure and the 12-byte Low SID it points to.
    let size = (std::mem::size_of::<TOKEN_MANDATORY_LABEL>() + 12) as u32;
    // SAFETY: a token we own with default-adjust access; `label` and `low` outlive the call.
    if unsafe {
        SetTokenInformation(
            restricted.0,
            TokenIntegrityLevel,
            (&label as *const TOKEN_MANDATORY_LABEL).cast(),
            size,
        )
    } == 0
    {
        return Err(last_error());
    }
    Ok(restricted)
}

/// Removes the entries a crashed node left behind. Call it once when the node starts, before any agent is running.
pub(crate) fn recover_stale() {
    let Ok(dir) = std::fs::read_dir(ledger_dir()) else {
        return;
    };
    for entry in dir.flatten() {
        let path = entry.path();
        // One stale ledger at a time under the lock, so a starting agent never counts on a label this is removing.
        let _lock = LedgerLock::acquire();
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        // A ledger of a node that is still running belongs to agents that need their permissions.
        if text.lines().next().is_some_and(owner_alive) {
            continue;
        }
        let mut clean = true;
        // Labels and self-grants first (the label may need the self-grant), each only if no live agent still uses the folder.
        for line in text.lines() {
            let Some((mark, folder)) = line.split_once('\t') else {
                continue;
            };
            let folder_path = Path::new(folder);
            if !folder_path.exists() || line_in_use_elsewhere(line, &path) {
                continue;
            }
            let undone = match mark {
                LABEL_LINE => set_low_label(folder_path, false)
                    .map(|()| clear_explicit_low_labels(folder_path)),
                _ => continue,
            };
            if !matches!(undone, Ok(true)) {
                clean = false;
            }
        }
        for line in text.lines() {
            let Some((mark, folder)) = line.split_once('\t') else {
                continue;
            };
            if mark == WO_LINE {
                // `<path>\t<logon SID of the node that granted it>`: after a reboot that is not this node's logon SID.
                let Some((folder, granter)) = folder.rsplit_once('\t') else {
                    continue;
                };
                let folder_path = Path::new(folder);
                if let Ok(granter) = Sid::from_string(granter) {
                    if folder_path.exists()
                        && !line_in_use_elsewhere(line, &path)
                        && has_explicit_ace_for(folder_path, granter.0)
                        && change_acl(folder_path, granter.0, 0, false).is_err()
                    {
                        clean = false;
                    }
                }
                continue;
            }
            if mark.starts_with('#') {
                continue;
            }
            let Ok(sid) = Sid::from_string(mark) else {
                continue;
            };
            let (folder, here) = match folder.rsplit_once('\t') {
                Some((f, HERE_MARK)) => (f, true),
                _ => (folder, false),
            };
            let folder = Path::new(folder);
            // A folder that is gone has nothing to undo; one the user may not change was skipped when granting.
            if !folder.exists() {
                continue;
            }
            let undone = if here {
                change_acl_here(folder, sid.0, 0, false).or_else(|e| {
                    (e.kind() == io::ErrorKind::PermissionDenied)
                        .then_some(())
                        .ok_or(e)
                })
            } else {
                change_acl(folder, sid.0, 0, false)
            };
            if undone.is_err() {
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
    fn explicit_low_labels_below_a_folder_are_cleared() {
        let root = std::env::temp_dir().join(format!("agora-clear-low-{}", std::process::id()));
        let sub = root.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        let file = sub.join("f.txt");
        std::fs::write(&file, "x").unwrap();
        for p in [&sub, &file] {
            let ok = std::process::Command::new("icacls")
                .arg(p)
                .args(["/setintegritylevel", "L"])
                .output()
                .unwrap()
                .status
                .success();
            assert!(ok);
            assert!(has_explicit_low_label(p));
        }
        assert!(clear_explicit_low_labels(&root));
        assert!(!has_explicit_low_label(&sub) && !has_explicit_low_label(&file));
        let _ = std::fs::remove_dir_all(&root);
    }

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
