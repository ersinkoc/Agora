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
/// #898: whether a read grant left out on `path` is said for the first time by this process. The same folders (`System32`, `Program
/// Files`) were said at every confined start; each is now said once, whatever the agent can still read there (that is not decided here).
fn first_time_left_out(path: &Path) -> bool {
    static SAID: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<PathBuf>>> =
        std::sync::OnceLock::new();
    let said = SAID.get_or_init(Default::default);
    // A poisoned lock only loses the deduplication: say it again rather than never.
    match said.lock() {
        Ok(mut seen) => seen.insert(path.to_path_buf()),
        Err(_) => true,
    }
}
/// For a folder whose label the user cannot set (a data volume's default ACL, Modify through `Authenticated Users` only). `false`, as
/// accepted ADR-0050 Amendment B, decision 2 reads: such a folder is refused before start (`label_failed`). `true` would give the
/// node's logon SID `WRITE_OWNER` there while the label is needed (so a checkout on `D:` works); that path is kept and was measured, but
/// switching it on needs a maintainer-accepted amendment first.
const SELF_GRANT_WRITE_OWNER: bool = false;

fn self_grant_enabled() -> bool {
    SELF_GRANT_WRITE_OWNER
}

/// Ledger line markers.
const HERE_MARK: &str = "here";
const LABEL_LINE: &str = "#label";
const WO_LINE: &str = "#wo";
/// The owner line of a ledger whose spawn has ended with entries it could not take back: no live process, so it is stale.
const ENDED_OWNER: &str = "#owner\t0\t0";
/// A ledger folder whose name is not valid Unicode is written as this mark followed by its UTF-16 units in hex. A lossy spelling would
/// name a folder that does not exist, and recovery takes a missing folder as gone, so its label and entries would stay for good. No
/// real path starts with `?`, so ledgers written before this read the same.
const WIDE_MARK: &str = "?wide:";

/// A folder as ledger lines spell it.
fn ledger_path(p: &Path) -> std::borrow::Cow<'_, str> {
    use std::os::windows::ffi::OsStrExt;
    match p.to_str() {
        Some(s) => s.into(),
        None => {
            let hex: String = p
                .as_os_str()
                .encode_wide()
                .map(|u| format!("{u:04x}"))
                .collect();
            format!("{WIDE_MARK}{hex}").into()
        }
    }
}

/// The folder a ledger line names (the reverse of [`ledger_path`]).
fn path_from_ledger(s: &str) -> PathBuf {
    use std::os::windows::ffi::OsStringExt;
    if let Some(hex) = s.strip_prefix(WIDE_MARK) {
        let units: Option<Vec<u16>> = (0..hex.len())
            .step_by(4)
            .map(|i| {
                hex.get(i..i + 4)
                    .and_then(|h| u16::from_str_radix(h, 16).ok())
            })
            .collect();
        if let Some(units) = units {
            return PathBuf::from(std::ffi::OsString::from_wide(&units));
        }
    }
    PathBuf::from(s)
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
pub(crate) fn random_sid_string() -> io::Result<String> {
    let n = random_numbers()?;
    Ok(format!("S-1-5-21-{}-{}-{}-{}", n[0], n[1], n[2], n[3]))
}

fn random_numbers() -> io::Result<[u32; 4]> {
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
    Ok(n)
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

/// Takes the Low label of an allowed folder back, and the explicit Low labels below it. A folder whose own label was never set (the
/// start was refused on it, or failed before) has nothing of its own to undo: removing a label it does not carry would need the very
/// `WRITE_OWNER` that may be missing, and would keep its ledger forever.
fn take_label_back(path: &Path) -> bool {
    if has_explicit_low_label(path) && set_low_label(path, false).is_err() {
        return false;
    }
    clear_explicit_low_labels(path)
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

/// Folders below a worktree whose hard links are made on purpose and are not checked for them (#571): package stores link every
/// package file into `node_modules` (pnpm: about 85 % of a checkout's files), and git hard-links objects in a `--local` clone.
const LINKS_NOT_CHECKED: &[&str] = &["node_modules", "target", ".git"];

/// Whether `path` is a regular file with more than one name (a hard link): asked of a handle opened for its attributes only, without
/// following a link, so nothing is read.
fn has_other_names(path: &Path) -> io::Result<bool> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
    };
    let file = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let (links, reparse) = crate::windows::link_facts(&file)?;
    Ok(!reparse && links > 1)
}

/// The first file or folder at or below `root` whose own DACL lets `Users` or `Everyone` write (Amendment B, 4). Walks the whole tree:
/// a single child with such a grant would let any confined agent write there through the Low label.
///
/// With `check_links` (the agent's worktree), also the first regular file with another name elsewhere (#571): the agent's ACE and the
/// Low label are propagated to the file object itself, so a hard link made outside confinement would let the agent write the file's
/// other place too. [`LINKS_NOT_CHECKED`] folders are walked for grants but not for links.
fn broad_writer_below(
    root: &Path,
    check_links: bool,
) -> io::Result<Option<(PathBuf, SandboxRefusal)>> {
    let refused: Vec<Sid> = REFUSED_WRITERS
        .iter()
        .map(|s| Sid::from_string(s))
        .collect::<io::Result<_>>()?;
    let need_owner = !self_grant_enabled();
    let mut stack = vec![(root.to_path_buf(), check_links)];
    while let Some((p, links)) = stack.pop() {
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
                // Only a real folder of such a name is exempt: a file named `node_modules` (or a link, a junction) is checked as any
                // other (#591 review).
                let exempt_folder = e.file_type().is_ok_and(|t| t.is_dir() && !t.is_symlink())
                    && LINKS_NOT_CHECKED
                        .iter()
                        .any(|n| e.file_name().eq_ignore_ascii_case(n));
                stack.push((e.path(), links && !exempt_folder));
            }
        } else if links && meta.is_file() && has_other_names(&p)? {
            return Ok(Some((p, SandboxRefusal::WritableHasLinks)));
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
    // A NULL DACL (or none) lets everyone do everything: the broadest grant there is.
    let mut found = err == 0 && dacl.is_null();
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

/// Amendment B, 4: refuses a profile whose writable folders (or anything below them) let `Users` or `Everyone` write, and (#571) a
/// worktree (`Sandbox::reopen`) holding a file with another name elsewhere, or a write entry that is itself such a file (a manifest's
/// or the policy's `sandbox.write` file, granted like a folder). Returns the first such path.
pub(crate) fn preflight(sandbox: &Sandbox) -> Result<(), (PathBuf, SandboxRefusal)> {
    for p in sandbox.write.iter().filter(|p| p.exists()) {
        match broad_writer_below(p, sandbox.reopen.contains(p) || p.is_file()) {
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
    // The other ledgers sit next to this one.
    let Some(Ok(dir)) = own.parent().map(std::fs::read_dir) else {
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
        let wanted = line_key(line);
        if text.lines().next().is_some_and(owner_alive)
            && text.lines().any(|l| line_key(l) == wanted)
        {
            return true;
        }
    }
    false
}

/// A ledger line in the form two spellings of one folder share: paths compare without case and without the `\\?\` prefix. (8.3 short
/// names are not resolved: the node writes every path the same way.)
fn line_key(line: &str) -> String {
    line.replace("\\\\?\\", "").to_lowercase()
}

/// Where the ledgers live: one `<sid>-<spawn>.acl` file per running agent process.
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
    format!("{WO_LINE}\t{}\t{logon}", ledger_path(path))
}

fn write_ledger(file: &Path, plan: &Plan) -> io::Result<()> {
    let mut text = owner_line();
    for p in plan.grants {
        text.push_str(&format!("{}\t{}\n", plan.sid, ledger_path(p)));
    }
    for p in plan.above {
        text.push_str(&format!("{}\t{}\t{HERE_MARK}\n", plan.sid, ledger_path(p)));
    }
    // A label (and the self-grant that may be needed to set it) is per folder, shared by the agents that use it.
    for p in plan.labels {
        text.push_str(&wo_line(p, plan.logon));
        text.push('\n');
        text.push_str(&format!("{LABEL_LINE}\t{}\n", ledger_path(p)));
    }
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(file, text)
}

#[cfg(test)]
thread_local! {
    /// Tests only: fail a start right after a folder's label went on, like a change that went through and still reported an error.
    static FAIL_AFTER_LABEL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// One agent's confinement: its SID, the folders that carry it, and the token to start the process with. Dropping it removes the
/// entries again.
pub(crate) struct Confinement {
    sid: Sid,
    sid_text: String,
    granted: Vec<PathBuf>,
    /// Folders above the granted ones, each with an entry on that folder only.
    granted_here: Vec<PathBuf>,
    /// Folders this agent labelled Low (or found labelled by a live agent and shares).
    labelled: Vec<PathBuf>,
    logon: Option<LogonSid>,
    logon_text: String,
    ledger: PathBuf,
    token: Token,
    git_config: Option<PathBuf>,
}

impl Confinement {
    /// `identity` is the agent's SID from [`crate::new_agent_identity`], made before its mesh endpoint so both name the same SID; `None`
    /// makes a fresh one.
    pub(crate) fn new(sandbox: &Sandbox, identity: Option<&str>) -> io::Result<Self> {
        Self::in_ledger_dir(sandbox, &ledger_dir(), identity)
    }

    fn in_ledger_dir(sandbox: &Sandbox, dir: &Path, identity: Option<&str>) -> io::Result<Self> {
        let sid_text = match identity {
            Some(s) => s.to_string(),
            None => random_sid_string()?,
        };
        let sid = Sid::from_string(&sid_text)?;
        // Held until every label of this agent is in place (see `LedgerLock`), and over the check below, so no other agent's start or
        // end changes these folders in between.
        let _lock = LedgerLock::acquire();
        // Amendment B, 4: refused before anything changes, so there is nothing to roll back. The caller maps `PermissionDenied` to
        // `agent.sandbox_unavailable`.
        if let Err((_, why)) = preflight(sandbox) {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, why));
        }
        // One ledger per spawn: a relaunch keeps the agent's SID (its mesh endpoint admits exactly that SID), and must neither
        // overwrite a ledger an earlier spawn kept after a failed removal nor have its own deleted when that spawn ends.
        let spawn = random_numbers()?;
        let ledger = dir.join(format!("{sid_text}-{:08x}{:08x}.acl", spawn[0], spawn[1]));
        let git_config = sandbox.git_config_file();
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
        let mut destinations = all.clone();
        destinations.extend(git_config.iter().cloned());
        let mut above = ancestors_of(&destinations);
        above.extend(git_config.iter().cloned());
        // A folder the user labelled on purpose is left as it is (and not undone later). One that a live agent labelled (a shared
        // `objects`, `refs`) is recorded here too, so the label stays until the last agent using it has ended.
        let label_line = |p: &Path| format!("{LABEL_LINE}\t{}", ledger_path(p));
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
            sid_text: sid_text.clone(),
            granted: Vec::new(),
            granted_here: Vec::new(),
            labelled: Vec::new(),
            logon,
            logon_text,
            ledger,
            token: Token(null_mut()),
            git_config: None,
        };
        // Every path is recorded in `confinement` *before* it is changed: on failure `confinement` is dropped, which undoes whatever was
        // added so far, including a change that went half through.
        for (path, mask) in wanted {
            confinement.granted.push(path.clone());
            match change_acl(&path, confinement.sid.0, mask, true) {
                Ok(()) => {}
                // A folder to read whose permissions the user may not change (`System32`, where `cmd.exe` lives, or `Program Files`):
                // it is left out instead of failing the start. That gives the agent less, never more: such a folder is read through
                // `Users`/`Everyone` in the restricting list, as the `above` folders are, and one that does not grant them stays closed.
                // A folder to write still fails the start.
                // It stays in `granted`: `drop` and recovery take back only an entry that is there, so should the call ever have written
                // part of it before failing, that part is still taken back.
                Err(e) if mask == READ_MASK && e.kind() == io::ErrorKind::PermissionDenied => {
                    if first_time_left_out(&path) {
                        eprintln!(
                            "agora-platform: confinement: read grant on {} left out: {e}",
                            path.display()
                        );
                    }
                }
                Err(e) => return Err(e),
            }
        }
        for path in above {
            if git_config.as_ref() == Some(&path) {
                // Only this file is readable. Its ancestors get traversal rights.
                confinement.granted_here.push(path.clone());
                if change_acl_here(&path, confinement.sid.0, FILE_GENERIC_READ, true).is_ok() {
                    confinement.git_config = Some(path);
                }
                continue;
            }
            // A folder whose permissions the user may not change (`C:\Users`) is left alone: `Users` in the restricting list covers it.
            match change_acl_here(&path, confinement.sid.0, ANCESTOR_MASK, true) {
                Ok(()) => confinement.granted_here.push(path),
                Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {}
                Err(e) => return Err(e),
            }
        }
        for path in labels {
            // Recorded first (see above): the label, and the self-grant that may come with it, are taken back by `drop`.
            confinement.labelled.push(path.clone());
            if shared.contains(&path) {
                // Already labelled by a live agent: share it, do not walk the tree again.
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
            #[cfg(test)]
            if FAIL_AFTER_LABEL.with(std::cell::Cell::get) {
                return Err(refused(SandboxRefusal::LabelFailed));
            }
        }
        confinement.token = restricted_token(&confinement.sid)?;
        Ok(confinement)
    }

    /// The primary token to start the agent's process with.
    pub(crate) fn token(&self) -> HANDLE {
        self.token.0
    }

    pub(crate) fn git_config(&self) -> Option<&Path> {
        self.git_config.as_deref()
    }
}

impl Drop for Confinement {
    fn drop(&mut self) {
        let mut all_removed = true;
        let _lock = LedgerLock::acquire();
        // Labels first (setting them may need the self-grant), each only when no other live agent still uses that folder.
        for path in &self.labelled {
            let line = format!("{LABEL_LINE}\t{}", ledger_path(path));
            if !line_in_use_elsewhere(&line, &self.ledger) && !take_label_back(path) {
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
        // A relaunch of the same agent keeps its SID; while that spawn's ledger lists the same entry, it is still in use. The line names
        // no access level: if two spawns of one agent got the same folder at different levels (a policy change between them), the
        // merged, stronger entry stays until the last of them ends. It belongs to the same agent, never to another one.
        let sid_line = |p: &Path| format!("{}\t{}", self.sid_text, ledger_path(p));
        for path in &self.granted {
            // A path recorded but never changed (the start failed on it) has no entry to take back.
            if !line_in_use_elsewhere(&sid_line(path), &self.ledger)
                && has_explicit_ace_for(path, self.sid.0)
                && change_acl(path, self.sid.0, 0, false).is_err()
            {
                all_removed = false;
            }
        }
        for path in &self.granted_here {
            let line = format!("{}\t{HERE_MARK}", sid_line(path));
            if !line_in_use_elsewhere(&line, &self.ledger)
                && change_acl_here(path, self.sid.0, 0, false).is_err()
            {
                all_removed = false;
            }
        }
        // A failed removal keeps the ledger so recovery retries; the SID dies with the agent either way. Its owner line is marked ended:
        // this spawn no longer uses any of it, so a later spawn of the same agent does not count these entries as in use (and takes
        // them back when it ends), and recovery treats the ledger as stale.
        if all_removed {
            let _ = std::fs::remove_file(&self.ledger);
        } else if let Ok(text) = std::fs::read_to_string(&self.ledger) {
            let rest = text.split_once('\n').map_or("", |(_, rest)| rest);
            let _ = std::fs::write(&self.ledger, format!("{ENDED_OWNER}\n{rest}"));
        }
    }
}

/// The SID of the user this process runs as, as text.
pub(crate) fn current_user_sid_text() -> io::Result<String> {
    // SAFETY: the pseudo handle of this process needs no closing.
    user_sid_text_of(unsafe { GetCurrentProcess() })
}

/// Whether process `pid` may be taken for this user's own (M2.6, T79: the node checks that the pipe it reached is served by its own
/// user's holder): [`trusted_as_this_user`] on both tokens. A process whose token cannot be read (another user's, as a rule) is not.
pub(crate) fn process_runs_as_this_user(pid: u32) -> bool {
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    // SAFETY: plain call; the handle is closed below.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return false;
    }
    let theirs = token_facts_of(process);
    // SAFETY: opened above, closed once.
    unsafe { windows_sys::Win32::Foundation::CloseHandle(process) };
    // SAFETY: the pseudo handle of this process needs no closing.
    let mine = token_facts_of(unsafe { GetCurrentProcess() });
    match (theirs, mine) {
        (Ok(theirs), Ok(mine)) => trusted_as_this_user(&theirs, &mine),
        _ => false,
    }
}

/// What the holder check needs to know of a process's token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TokenFacts {
    /// The user's SID, as text.
    pub(crate) user: String,
    /// A restricted token (one with restricting SIDs, as a confined agent's: ADR-0050).
    pub(crate) restricted: bool,
    /// The mandatory integrity level's RID (Low 0x1000, Medium 0x2000, High 0x3000, System 0x4000).
    pub(crate) integrity: u32,
    /// An AppContainer token.
    pub(crate) app_container: bool,
}

/// M2.6 (T79): a process the node may take for this user's own holder: the same user, not restricted, not in an AppContainer, and at
/// this process's integrity level or higher. A confined agent runs as the same user but restricted and at Low integrity (ADR-0050
/// Amendment B), so it can never pass for the holder even when it creates the holder's pipe first.
pub(crate) fn trusted_as_this_user(theirs: &TokenFacts, mine: &TokenFacts) -> bool {
    theirs.user == mine.user
        && !theirs.restricted
        && !theirs.app_container
        && theirs.integrity >= mine.integrity
}

/// The [`TokenFacts`] of `process` (a handle with PROCESS_QUERY_LIMITED_INFORMATION, or this process's pseudo handle).
fn token_facts_of(process: HANDLE) -> io::Result<TokenFacts> {
    use windows_sys::Win32::Security::{
        GetSidSubAuthority, GetSidSubAuthorityCount, IsTokenRestricted, TokenIsAppContainer,
    };
    let mut token: HANDLE = null_mut();
    // SAFETY: a process handle as above and an out pointer.
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        return Err(last_error());
    }
    let token = Token(token);
    let user = token_user_sid_text(token.0)?;
    // SAFETY: a token with TOKEN_QUERY.
    let restricted = unsafe { IsTokenRestricted(token.0) } != 0;
    let mut container = 0u32;
    let mut got = 0u32;
    // SAFETY: a u32 out-buffer of the size given (TokenIsAppContainer answers a DWORD).
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenIsAppContainer,
            (&mut container as *mut u32).cast(),
            4,
            &mut got,
        )
    } == 0
    {
        return Err(last_error());
    }
    let mut needed = 0u32;
    // SAFETY: the documented size query.
    unsafe { GetTokenInformation(token.0, TokenIntegrityLevel, null_mut(), 0, &mut needed) };
    let bytes = (needed as usize).max(std::mem::size_of::<TOKEN_MANDATORY_LABEL>());
    let mut buf = vec![0u64; bytes.div_ceil(8)];
    // SAFETY: `buf` is at least `needed` bytes, pointer-aligned.
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenIntegrityLevel,
            buf.as_mut_ptr().cast(),
            (buf.len() * 8) as u32,
            &mut needed,
        )
    } == 0
    {
        return Err(last_error());
    }
    // SAFETY: GetTokenInformation(TokenIntegrityLevel) wrote a TOKEN_MANDATORY_LABEL at the start of `buf`; its SID's last
    // sub-authority is the level's RID.
    let integrity = unsafe {
        let label = &*(buf.as_ptr().cast::<TOKEN_MANDATORY_LABEL>());
        let count = *GetSidSubAuthorityCount(label.Label.Sid);
        if count == 0 {
            return Err(io::Error::other("integrity label"));
        }
        *GetSidSubAuthority(label.Label.Sid, u32::from(count) - 1)
    };
    Ok(TokenFacts {
        user,
        restricted,
        integrity,
        app_container: container != 0,
    })
}

/// The user SID of `token` (TOKEN_QUERY), as text.
fn token_user_sid_text(token: HANDLE) -> io::Result<String> {
    use windows_sys::Win32::Security::{TokenUser, TOKEN_USER};
    let mut needed = 0u32;
    // SAFETY: the documented size query.
    unsafe { GetTokenInformation(token, TokenUser, null_mut(), 0, &mut needed) };
    let bytes = (needed as usize).max(std::mem::size_of::<TOKEN_USER>());
    let mut buf = vec![0u64; bytes.div_ceil(8)];
    // SAFETY: `buf` is at least `needed` bytes, pointer-aligned.
    if unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            buf.as_mut_ptr().cast(),
            (buf.len() * 8) as u32,
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

/// The SID of the user `process` runs as, as text.
fn user_sid_text_of(process: HANDLE) -> io::Result<String> {
    use windows_sys::Win32::Security::{TokenUser, TOKEN_USER};
    let mut own: HANDLE = null_mut();
    // SAFETY: a process handle with PROCESS_QUERY_LIMITED_INFORMATION (or this process's pseudo handle) and an out pointer.
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut own) } == 0 {
        return Err(last_error());
    }
    let own = Token(own);
    let mut needed = 0u32;
    // SAFETY: the documented size query.
    unsafe { GetTokenInformation(own.0, TokenUser, null_mut(), 0, &mut needed) };
    // u64 units: the structure read from it needs pointer alignment, which a byte vector does not promise.
    let bytes = (needed as usize).max(std::mem::size_of::<TOKEN_USER>());
    let mut buf = vec![0u64; bytes.div_ceil(8)];
    // SAFETY: `buf` is `needed` bytes.
    if unsafe {
        GetTokenInformation(
            own.0,
            TokenUser,
            buf.as_mut_ptr().cast(),
            (buf.len() * 8) as u32,
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

/// The SID that owns what this process creates (its token's default owner: the user, or the Administrators group when the process runs
/// elevated), as text.
pub(crate) fn current_owner_sid_text() -> io::Result<String> {
    use windows_sys::Win32::Security::{TokenOwner, TOKEN_OWNER};
    let mut own: HANDLE = null_mut();
    // SAFETY: the pseudo handle of this process and an out pointer.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut own) } == 0 {
        return Err(last_error());
    }
    let own = Token(own);
    let mut needed = 0u32;
    // SAFETY: the documented size query.
    unsafe { GetTokenInformation(own.0, TokenOwner, null_mut(), 0, &mut needed) };
    // u64 units: the structure read from it needs pointer alignment, which a byte vector does not promise.
    let bytes = (needed as usize).max(std::mem::size_of::<TOKEN_OWNER>());
    let mut buf = vec![0u64; bytes.div_ceil(8)];
    // SAFETY: `buf` is `needed` bytes.
    if unsafe {
        GetTokenInformation(
            own.0,
            TokenOwner,
            buf.as_mut_ptr().cast(),
            (buf.len() * 8) as u32,
            &mut needed,
        )
    } == 0
    {
        return Err(last_error());
    }
    // SAFETY: GetTokenInformation(TokenOwner) wrote a TOKEN_OWNER at the start of `buf`.
    let owner = unsafe { &*(buf.as_ptr().cast::<TOKEN_OWNER>()) };
    sid_text(owner.Owner)
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
    // Objects the agent creates itself (its own process and threads, the pipes `cmd` makes for `a | b`, events) get the token's default
    // DACL. The user's default names the user and SYSTEM, and the user alone does not pass the restricting check, so the agent could
    // not open what it just made ("Not enough memory resources" from `cmd`). The user's SID passes the first check and the agent's SID
    // the second; no other confined agent holds both, so none can open this agent's process (review of #273, 1: the logon SID, which
    // every confined agent of the session holds in both lists, is not named).
    let agent_text = sid_text(agent.0)?;
    let user_text = current_user_sid_text()?;
    set_default_dacl(
        restricted.0,
        &format!("D:(A;;GA;;;SY)(A;;GA;;;{user_text})(A;;GA;;;{agent_text})"),
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
    recover_in(&ledger_dir());
}

fn recover_in(ledgers: &Path) {
    let Ok(dir) = std::fs::read_dir(ledgers) else {
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
            let folder_path = &path_from_ledger(folder);
            if !folder_path.exists() || line_in_use_elsewhere(line, &path) {
                continue;
            }
            if mark == LABEL_LINE && !take_label_back(folder_path) {
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
                let folder_path = &path_from_ledger(folder);
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
            // A live relaunch of the same agent (same SID) still uses this entry.
            if line_in_use_elsewhere(line, &path) {
                continue;
            }
            let (folder, here) = match folder.rsplit_once('\t') {
                Some((f, HERE_MARK)) => (f, true),
                _ => (folder, false),
            };
            let folder = &path_from_ledger(folder);
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

    fn icacls(args: &[&str]) -> String {
        let out = std::process::Command::new("icacls")
            .args(args)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("agora-wc-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("ledgers")).unwrap();
        root
    }

    fn files_in(dir: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .collect()
    }

    /// Review of #273, 3: a start that fails after it has changed something (the first folder granted, the second refusing the DACL
    /// change) leaves no grant of the agent anywhere, and no ledger once everything is taken back. This guards the contract; the one
    /// step that could change a folder and still fail (the self-grant before a label) is unreachable while the self-grant is off.
    #[test]
    fn a_start_failing_halfway_leaves_no_grant_and_no_ledger() {
        let root = scratch("halfway");
        let ledgers = root.join("ledgers");
        let (first, locked) = (root.join("first"), root.join("locked"));
        for d in [&first, &locked] {
            std::fs::create_dir_all(d.join("sub")).unwrap();
        }
        let user = format!("*{}", current_user_sid_text().unwrap());
        let locked_s = locked.to_string_lossy().into_owned();
        // The owner may always change a DACL unless an `OWNER RIGHTS` entry says otherwise; the deny takes the inherited grant away.
        icacls(&[
            &locked_s,
            "/deny",
            &format!("{user}:(WDAC)"),
            "/grant",
            "*S-1-3-4:(RX)",
        ]);
        let identity = random_sid_string().unwrap();
        let sandbox = Sandbox {
            read: vec![],
            write: vec![first.clone(), locked.clone()],
            ..Default::default()
        };
        let result = Confinement::in_ledger_dir(&sandbox, &ledgers, Some(&identity));
        let failed = result.is_err();
        drop(result);
        let left = format!(
            "{}{}",
            icacls(&[&first.to_string_lossy(), "/T", "/C"]),
            icacls(&[&locked_s, "/T", "/C"])
        );
        let ledger_files = files_in(&ledgers);
        // Undo the test's own deny before asserting, so a failure does not leave the folder locked.
        icacls(&[&locked_s, "/remove:d", &user, "/remove:g", "*S-1-3-4"]);
        let _ = std::fs::remove_dir_all(&root);
        assert!(failed, "the start must fail");
        assert!(!left.contains(&identity), "left: {left}");
        assert!(!left.contains("Mandatory Label"), "left: {left}");
        assert!(ledger_files.is_empty(), "ledgers left: {ledger_files:?}");
    }

    /// A folder the user may change but not label (Modify, no `WRITE_OWNER`: what a data volume's default ACL gives).
    fn unlabelable(dir: &Path) -> String {
        let user = format!("*{}", current_user_sid_text().unwrap());
        icacls(&[
            &dir.to_string_lossy(),
            "/inheritance:r",
            "/grant:r",
            &format!("{user}:(OI)(CI)M"),
            "/grant:r",
            "*S-1-5-18:(OI)(CI)F",
        ])
    }

    /// Re-review of #273, 1: a `label_failed` refusal leaves no ledger and no grant, and a label that was never set is nothing to take
    /// back (taking it back would need the `WRITE_OWNER` that is missing, and kept the ledger forever).
    #[test]
    fn a_label_failed_refusal_leaves_no_ledger_and_no_grant() {
        let root = scratch("label-failed");
        let ledgers = root.join("ledgers");
        let work = root.join("work");
        std::fs::create_dir_all(work.join("sub")).unwrap();
        unlabelable(&work);
        if may_write_owner(&work) {
            // An account that may relabel anything anyway (an elevated administrator, as on CI runners): nothing to refuse here.
            eprintln!("skipped: this account may set labels on every folder");
            let _ = std::fs::remove_dir_all(&root);
            return;
        }
        let identity = random_sid_string().unwrap();
        let sandbox = Sandbox {
            read: vec![],
            write: vec![work.clone()],
            ..Default::default()
        };
        // The check that runs before any change already finds it (re-review note 2).
        let checked = preflight(&sandbox).err().map(|(_, why)| why);
        let err = Confinement::in_ledger_dir(&sandbox, &ledgers, Some(&identity))
            .err()
            .map(|e| crate::sandbox_refusal(&e));
        let left = format!(
            "{}{}",
            icacls(&[&work.to_string_lossy(), "/T", "/C"]),
            icacls(&[&root.to_string_lossy()])
        );
        let ledger_files = files_in(&ledgers);
        // The label was never set here: nothing to undo, where removing one fails for want of `WRITE_OWNER`.
        let nothing_to_undo = take_label_back(&work);
        let removing_fails = set_low_label(&work, false).is_err();
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(checked, Some(SandboxRefusal::LabelFailed));
        assert_eq!(err, Some(Some(SandboxRefusal::LabelFailed)));
        assert!(!left.contains(&identity), "left: {left}");
        assert!(ledger_files.is_empty(), "ledgers left: {ledger_files:?}");
        assert!(nothing_to_undo);
        assert!(removing_fails, "this folder should not be labelable");
    }

    /// Re-review of #273, 4: a change that went through and still failed the start (the label is on, then an error) is taken back,
    /// because the folder was recorded before it was changed. Recording it after the change, as before, left the label behind.
    #[test]
    fn a_label_that_went_on_before_the_start_failed_is_taken_back() {
        let root = scratch("after-label");
        let ledgers = root.join("ledgers");
        let work = root.join("work");
        std::fs::create_dir_all(work.join("sub")).unwrap();
        std::fs::write(work.join("sub").join("f.txt"), "x").unwrap();
        let identity = random_sid_string().unwrap();
        let sandbox = Sandbox {
            read: vec![],
            write: vec![work.clone()],
            ..Default::default()
        };
        FAIL_AFTER_LABEL.with(|f| f.set(true));
        let result = Confinement::in_ledger_dir(&sandbox, &ledgers, Some(&identity));
        FAIL_AFTER_LABEL.with(|f| f.set(false));
        let failed = result.is_err();
        drop(result);
        let left = icacls(&[&work.to_string_lossy(), "/T", "/C"]);
        let ledger_files = files_in(&ledgers);
        let _ = std::fs::remove_dir_all(&root);
        assert!(failed);
        assert!(!left.contains("Mandatory Label"), "left: {left}");
        assert!(!left.contains(&identity), "left: {left}");
        assert!(ledger_files.is_empty(), "ledgers left: {ledger_files:?}");
    }

    /// A crashed node's ledger for a folder whose name is not valid Unicode: recovery finds that folder again and undoes it (a lossy
    /// spelling named no folder, so recovery took it as gone, dropped the ledger, and the label and entry stayed for good).
    #[test]
    fn recovery_undoes_a_folder_whose_name_is_not_valid_unicode() {
        use std::os::windows::ffi::OsStringExt;
        let root = std::env::temp_dir().join(format!("agora-wide-ledger-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let ledgers = root.join("ledgers");
        let mut name: Vec<u16> = "odd-".encode_utf16().collect();
        name.push(0xD800);
        let odd = root.join("work").join(std::ffi::OsString::from_wide(&name));
        std::fs::create_dir_all(&odd).unwrap();
        assert_eq!(path_from_ledger(&ledger_path(&odd)), odd);
        assert_eq!(path_from_ledger(&ledger_path(&root)), root);

        let sandbox = Sandbox::new(vec![], vec![odd.clone()]);
        let c = Confinement::in_ledger_dir(&sandbox, &ledgers, None).unwrap();
        let sid = Sid::from_string(&c.sid_text).unwrap();
        let ledger = c.ledger.clone();
        // A crash: nothing is taken back, and the owner is gone.
        std::mem::forget(c);
        assert!(has_explicit_label(&odd) && has_explicit_ace_for(&odd, sid.0));
        let text = std::fs::read_to_string(&ledger).unwrap();
        let rest = text.split_once('\n').unwrap().1;
        std::fs::write(&ledger, format!("{ENDED_OWNER}\n{rest}")).unwrap();

        recover_in(&ledgers);
        assert!(!has_explicit_label(&odd), "the Low label stayed");
        assert!(
            !has_explicit_ace_for(&odd, sid.0),
            "the agent's entry stayed"
        );
        assert!(!ledger.exists(), "the ledger stayed although all is undone");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Audit 15 follow-up: `agora node run` recovers before it takes the data-folder lock, so a second node started next to a running
    /// one recovers while the first one's agents are confined. The running node's ledger names a live owner, so nothing of it is undone.
    #[test]
    fn recovery_leaves_a_running_nodes_permissions_alone() {
        let root = scratch("live-owner");
        let ledgers = root.join("ledgers");
        let work = root.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let sandbox = Sandbox::new(vec![], vec![work.clone()]);
        // This test process is the running node.
        let c = Confinement::in_ledger_dir(&sandbox, &ledgers, None).unwrap();
        let sid = Sid::from_string(&c.sid_text).unwrap();
        assert!(has_explicit_label(&work) && has_explicit_ace_for(&work, sid.0));

        // The second node's recovery.
        recover_in(&ledgers);
        assert!(has_explicit_label(&work), "the Low label was taken back");
        assert!(
            has_explicit_ace_for(&work, sid.0),
            "the running agent's entry was taken back"
        );
        assert!(c.ledger.exists(), "the running node's ledger was dropped");

        // The running node's own stop still undoes everything.
        let ledger = c.ledger.clone();
        drop(c);
        assert!(!has_explicit_label(&work) && !has_explicit_ace_for(&work, sid.0));
        assert!(!ledger.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Review of #273, 4: a NULL DACL lets everyone do everything, so it is refused like a `Users`/`Everyone` write grant.
    #[test]
    fn a_null_dacl_is_a_broad_write_grant() {
        let root = scratch("null-dacl");
        let open = root.join("work").join("open");
        std::fs::create_dir_all(&open).unwrap();
        let mut sd = null_mut();
        // SAFETY: a valid NUL-terminated string and an out pointer; freed below.
        let made = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide("D:NO_ACCESS_CONTROL".as_ref()).as_ptr(),
                SDDL_REVISION_1,
                &mut sd,
                null_mut(),
            )
        };
        assert_ne!(made, 0);
        // SAFETY: a NUL-terminated path and the descriptor made above.
        let set = unsafe {
            SetFileSecurityW(
                wide(open.as_os_str()).as_ptr(),
                DACL_SECURITY_INFORMATION,
                sd,
            )
        };
        // SAFETY: allocated by the conversion above.
        unsafe { LocalFree(sd) };
        assert_ne!(set, 0);
        let hit = broad_writer_below(&root.join("work"), false).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(hit, Some((open, SandboxRefusal::BroadWriteGrant)));
    }

    /// #571: a hard link in the worktree to a file outside it, made before the agent starts, would carry the agent's grant to that
    /// file. Such a worktree is refused, naming the link; the same link in `node_modules`, `target` or `.git` (package stores and git
    /// link on purpose) or in a write place that is not the worktree is not.
    #[test]
    fn a_worktree_holding_a_hard_link_to_a_file_outside_is_refused() {
        let root = scratch("links571");
        let work = root.join("work");
        for d in ["node_modules/pkg", "target", ".git/objects", "src"] {
            std::fs::create_dir_all(work.join(d)).unwrap();
        }
        std::fs::create_dir_all(root.join("outside")).unwrap();
        std::fs::write(root.join("outside").join("zshrc"), "rc").unwrap();
        std::fs::write(work.join("src").join("plain.txt"), "x").unwrap();
        for d in ["node_modules/pkg", "target", ".git/objects"] {
            std::fs::hard_link(
                root.join("outside").join("zshrc"),
                work.join(d).join("linked"),
            )
            .unwrap();
        }
        // Package stores and git: walked for grants, never refused for their links.
        assert_eq!(broad_writer_below(&work, true).unwrap(), None);
        let link = work.join("src").join("y");
        std::fs::hard_link(root.join("outside").join("zshrc"), &link).unwrap();
        let hit = broad_writer_below(&work, true).unwrap();
        // Another write place (not the worktree) is not checked for links.
        let elsewhere = broad_writer_below(&work, false).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(hit, Some((link, SandboxRefusal::WritableHasLinks)));
        assert_eq!(elsewhere, None);
    }

    /// #591 review: the exemption is for folders only. A hard-linked regular file named `node_modules`, `target` or `.git` is
    /// refused like any other.
    #[test]
    fn a_linked_file_named_like_an_exempt_folder_is_still_refused() {
        for name in ["node_modules", "target", ".git", "NODE_MODULES"] {
            let root = scratch(&format!("links571n-{}", name.trim_start_matches('.')));
            let work = root.join("work");
            std::fs::create_dir_all(&work).unwrap();
            std::fs::write(root.join("outside"), "rc").unwrap();
            std::fs::hard_link(root.join("outside"), work.join(name)).unwrap();
            let hit = broad_writer_below(&work, true).unwrap();
            let _ = std::fs::remove_dir_all(&root);
            assert_eq!(
                hit,
                Some((work.join(name), SandboxRefusal::WritableHasLinks)),
                "{name}"
            );
        }
    }

    /// #571 (review): a write entry that is itself a file (a manifest's `~/.claude.json`) is granted like a folder, so one with
    /// another name elsewhere is refused too; a plain file entry is not.
    #[test]
    fn a_write_entry_that_is_a_linked_file_is_refused() {
        let root = scratch("links571f");
        std::fs::write(root.join("plain.json"), "{}").unwrap();
        std::fs::write(root.join("real.json"), "{}").unwrap();
        std::fs::hard_link(root.join("real.json"), root.join("entry.json")).unwrap();
        let profile = |entry: &str| Sandbox {
            read: vec![],
            write: vec![root.join(entry)],
            ..Default::default()
        };
        let plain = preflight(&profile("plain.json"));
        let linked = preflight(&profile("entry.json"));
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(plain, Ok(()));
        assert_eq!(
            linked,
            Err((root.join("entry.json"), SandboxRefusal::WritableHasLinks))
        );
    }

    /// Review of #273, 5: a relaunch keeps the agent's identity (its mesh endpoint admits exactly that SID), so two confinements of one
    /// identity can overlap. Each has its own ledger, and the one that ends first leaves the other's grant in place.
    #[test]
    fn a_late_end_of_an_earlier_spawn_leaves_the_next_spawns_grant() {
        let root = scratch("respawn");
        let ledgers = root.join("ledgers");
        let work = root.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let identity = random_sid_string().unwrap();
        let sandbox = Sandbox {
            read: vec![],
            write: vec![work.clone()],
            ..Default::default()
        };
        let earlier = Confinement::in_ledger_dir(&sandbox, &ledgers, Some(&identity)).unwrap();
        let next = Confinement::in_ledger_dir(&sandbox, &ledgers, Some(&identity)).unwrap();
        let two_ledgers = files_in(&ledgers).len();
        drop(earlier);
        let after_earlier = icacls(&[&work.to_string_lossy()]);
        let ledgers_after_earlier = files_in(&ledgers).len();
        drop(next);
        let after_both = icacls(&[&work.to_string_lossy()]);
        let ledgers_after_both = files_in(&ledgers);
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(two_ledgers, 2);
        assert!(after_earlier.contains(&identity), "{after_earlier}");
        assert!(after_earlier.contains("Mandatory Label"), "{after_earlier}");
        assert_eq!(ledgers_after_earlier, 1);
        assert!(!after_both.contains(&identity), "{after_both}");
        assert!(!after_both.contains("Mandatory Label"), "{after_both}");
        assert!(ledgers_after_both.is_empty(), "{ledgers_after_both:?}");
    }

    #[test]
    fn a_ledger_is_alive_while_its_owner_runs_and_stale_otherwise() {
        // This very process owns the line it just wrote.
        assert!(owner_alive(owner_line().trim_end()));
        // A pid nobody has, a wrong start time for a live pid, and a line that is not an owner line.
        assert!(!owner_alive("#owner\t4294967290\t1"));
        // A ledger kept after a failed removal is marked ended, hence stale.
        assert!(!owner_alive(ENDED_OWNER));
        let pid = std::process::id();
        assert!(!owner_alive(&format!("#owner\t{pid}\t1")));
        assert!(!owner_alive("S-1-5-21-1-2-3-4\tC:\\x"));
        assert!(!owner_alive(""));
    }

    /// A folder the user may read and empty but whose permissions they may not change (an `OWNER RIGHTS` entry takes the owner's
    /// implicit `WRITE_DAC` away), and that grants nothing to `Users` or `Everyone`: what `System32` or `Program Files` is to a user.
    fn unchangeable(dir: &Path) {
        let user = format!("*{}", current_user_sid_text().unwrap());
        icacls(&[
            &dir.to_string_lossy(),
            "/inheritance:r",
            "/grant:r",
            &format!("{user}:(OI)(CI)(RX,D,DC)"),
            "/grant:r",
            "*S-1-5-18:(OI)(CI)F",
            "/grant:r",
            "*S-1-3-4:(OI)(CI)(RX,D,DC)",
        ]);
    }

    /// `cmd /c rmdir`: `remove_dir_all` first tries to clear attributes, which [`unchangeable`] does not allow.
    fn remove(dir: &Path) {
        let _ = std::process::Command::new("cmd")
            .args(["/c", "rmdir", "/s", "/q"])
            .arg(dir)
            .output();
    }

    /// Whether this process may rewrite `dir`'s DACL: opening it for `WRITE_DAC` asks the same access check without changing anything.
    /// A test that needs a folder the user may not change asks this first: an elevated administrator (a CI runner) may change more.
    fn may_change_dacl(dir: &Path) -> bool {
        use std::os::windows::fs::OpenOptionsExt;
        const WRITE_DAC: u32 = 0x0004_0000;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        std::fs::OpenOptions::new()
            .access_mode(WRITE_DAC)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(dir)
            .is_ok()
    }

    /// The folders of `candidates` this process may not re-permission, each said when it is not one.
    fn unchangeable_of(candidates: &[&Path]) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for c in candidates {
            if may_change_dacl(c) {
                eprintln!(
                    "premise does not hold here: this process may change the DACL of {}",
                    c.display()
                );
            } else {
                out.push(c.to_path_buf());
            }
        }
        out
    }

    /// #898: a read grant left out is said once per folder and process, not at every confined start; another folder is still said.
    #[test]
    fn a_read_grant_left_out_is_said_once_per_folder() {
        let root = scratch("read-said-once");
        let (a, b) = (root.join("a"), root.join("b"));
        assert!(first_time_left_out(&a));
        assert!(!first_time_left_out(&a));
        assert!(!first_time_left_out(&a));
        assert!(first_time_left_out(&b));
        remove(&root);
    }

    /// A folder to read whose permissions the user may not change (`System32`, where `cmd.exe` is) is left out, and the start goes on;
    /// before, `ERROR_ACCESS_DENIED` failed every confined start whose program lives there or in `Program Files`.
    #[test]
    fn a_read_grant_the_user_may_not_make_is_left_out_and_the_start_goes_on() {
        let root = scratch("read-unchangeable");
        let ledgers = root.join("ledgers");
        let (work, locked) = (root.join("work"), root.join("locked"));
        std::fs::create_dir_all(&work).unwrap();
        std::fs::create_dir_all(&locked).unwrap();
        unchangeable(&locked);
        let system32 = PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32");
        let refused = unchangeable_of(&[&locked, &system32]);
        if refused.is_empty() {
            remove(&root);
            eprintln!("skipped: no folder here that this process may not re-permission");
            return;
        }
        let identity = random_sid_string().unwrap();
        let sandbox = Sandbox {
            read: refused.clone(),
            write: vec![work.clone()],
            ..Default::default()
        };
        let c = Confinement::in_ledger_dir(&sandbox, &ledgers, Some(&identity));
        let started = c.as_ref().map(|_| ()).map_err(ToString::to_string);
        let on_refused: Vec<String> = refused
            .iter()
            .map(|p| icacls(&[&p.to_string_lossy()]))
            .collect();
        let on_work = icacls(&[&work.to_string_lossy()]);
        drop(c);
        let ledger_files = files_in(&ledgers);
        remove(&root);
        if let Err(e) = started {
            panic!("the start must go on: {e}");
        }
        for acl in &on_refused {
            assert!(!acl.contains(&identity), "{acl}");
        }
        assert!(
            on_work.contains(&identity),
            "the folder to write is granted: {on_work}"
        );
        assert!(ledger_files.is_empty(), "ledgers left: {ledger_files:?}");
    }

    /// Only reads are left out: a folder to write whose permissions the user may not change still fails the start, with nothing left.
    #[test]
    fn a_write_grant_the_user_may_not_make_still_fails_the_start() {
        let root = scratch("write-unchangeable");
        let ledgers = root.join("ledgers");
        let locked = root.join("locked");
        std::fs::create_dir_all(&locked).unwrap();
        unchangeable(&locked);
        let system32 = PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32");
        // The scratch folder first: only where it may be re-permissioned here is `System32` the folder asked to write.
        let Some(target) = unchangeable_of(&[&locked, &system32]).into_iter().next() else {
            remove(&root);
            eprintln!("skipped: no folder here that this process may not re-permission");
            return;
        };
        let identity = random_sid_string().unwrap();
        let sandbox = Sandbox {
            read: vec![],
            write: vec![target.clone()],
            ..Default::default()
        };
        let result = Confinement::in_ledger_dir(&sandbox, &ledgers, Some(&identity));
        let failed = result.is_err();
        drop(result);
        let on_target = icacls(&[&target.to_string_lossy()]);
        let ledger_files = files_in(&ledgers);
        remove(&root);
        assert!(failed, "the start must fail ({})", target.display());
        assert!(!on_target.contains(&identity), "{on_target}");
        assert!(ledger_files.is_empty(), "ledgers left: {ledger_files:?}");
    }

    /// The premise probe itself: the owner of a plain folder may re-permission it, and nothing is changed by asking.
    #[test]
    fn the_owner_may_change_a_plain_folders_dacl() {
        let root = scratch("probe");
        let before = icacls(&[&root.to_string_lossy()]);
        assert!(may_change_dacl(&root));
        assert_eq!(icacls(&[&root.to_string_lossy()]), before);
        remove(&root);
    }
}

/// M2.6 (T79): who may pass for this user's holder.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod holder_trust_tests {
    use super::*;

    fn me() -> TokenFacts {
        TokenFacts {
            user: "S-1-5-21-1-2-3-1001".into(),
            restricted: false,
            integrity: 0x2000,
            app_container: false,
        }
    }

    #[test]
    fn only_an_unrestricted_process_of_this_user_at_this_level_or_above_passes() {
        let mine = me();
        assert!(trusted_as_this_user(&mine, &mine));
        assert!(trusted_as_this_user(
            &TokenFacts {
                integrity: 0x3000,
                ..me()
            },
            &mine
        ));
        for theirs in [
            TokenFacts {
                user: "S-1-5-21-1-2-3-1002".into(),
                ..me()
            },
            // A confined agent: the same user, restricted, Low.
            TokenFacts {
                restricted: true,
                integrity: 0x1000,
                ..me()
            },
            TokenFacts {
                restricted: true,
                ..me()
            },
            TokenFacts {
                integrity: 0x1000,
                ..me()
            },
            TokenFacts {
                app_container: true,
                ..me()
            },
        ] {
            assert!(!trusted_as_this_user(&theirs, &mine), "{theirs:?}");
        }
    }

    /// A real process with a restricted, Low-integrity copy of this process's token (as a confined agent gets) never passes for this
    /// user's holder, though its user is this user; this process does.
    #[test]
    fn a_restricted_low_process_of_this_user_does_not_pass() {
        use windows_sys::Win32::Security::{SECURITY_MAX_SID_SIZE, TOKEN_ADJUST_DEFAULT};
        use windows_sys::Win32::System::Threading::{
            CreateProcessAsUserW, TerminateProcess, CREATE_NO_WINDOW, PROCESS_INFORMATION,
            STARTUPINFOW,
        };
        assert!(process_runs_as_this_user(std::process::id()));
        let mut own: HANDLE = null_mut();
        // SAFETY: this process's pseudo handle and an out pointer.
        assert!(
            unsafe {
                OpenProcessToken(
                    GetCurrentProcess(),
                    TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY | TOKEN_ADJUST_DEFAULT,
                    &mut own,
                )
            } != 0
        );
        let own = Token(own);
        // Everyone as the restricting SID (any restricting SID makes the token restricted).
        let mut everyone = vec![0u8; SECURITY_MAX_SID_SIZE as usize];
        let mut everyone_psid: PSID = null_mut();
        // SAFETY: a NUL-terminated SID string and an out pointer (freed with LocalFree below).
        assert!(
            unsafe {
                ConvertStringSidToSidW(
                    wide(std::ffi::OsStr::new("S-1-1-0")).as_ptr(),
                    &mut everyone_psid,
                )
            } != 0
        );
        let _ = &mut everyone;
        let restricting = [SID_AND_ATTRIBUTES {
            Sid: everyone_psid,
            Attributes: 0,
        }];
        let mut restricted: HANDLE = null_mut();
        // SAFETY: a token with TOKEN_DUPLICATE and a one-entry restricting list.
        assert!(
            unsafe {
                CreateRestrictedToken(
                    own.0,
                    DISABLE_MAX_PRIVILEGE,
                    0,
                    null(),
                    0,
                    null(),
                    1,
                    restricting.as_ptr(),
                    &mut restricted,
                )
            } != 0
        );
        let restricted = Token(restricted);
        let mut low_psid: PSID = null_mut();
        // SAFETY: as above.
        assert!(
            unsafe {
                ConvertStringSidToSidW(
                    wide(std::ffi::OsStr::new("S-1-16-4096")).as_ptr(),
                    &mut low_psid,
                )
            } != 0
        );
        let label = TOKEN_MANDATORY_LABEL {
            Label: SID_AND_ATTRIBUTES {
                Sid: low_psid,
                Attributes: 0x20, // SE_GROUP_INTEGRITY
            },
        };
        // SAFETY: a valid label structure and its size.
        assert!(
            unsafe {
                SetTokenInformation(
                    restricted.0,
                    TokenIntegrityLevel,
                    (&label as *const TOKEN_MANDATORY_LABEL).cast(),
                    std::mem::size_of::<TOKEN_MANDATORY_LABEL>() as u32,
                )
            } != 0
        );
        // SAFETY: allocated by ConvertStringSidToSidW.
        unsafe {
            LocalFree(everyone_psid as _);
            LocalFree(low_psid as _);
        }
        let mut cmd = wide(std::ffi::OsStr::new("ping.exe -n 30 127.0.0.1"));
        // SAFETY: an all-zero STARTUPINFOW with its size, an out PROCESS_INFORMATION.
        let mut si: STARTUPINFOW = unsafe { std::mem::zeroed() };
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: a restricted copy of this process's own token (no privilege needed to assign it), a mutable command line.
        assert!(
            unsafe {
                CreateProcessAsUserW(
                    restricted.0,
                    null(),
                    cmd.as_mut_ptr(),
                    null(),
                    null(),
                    0,
                    CREATE_NO_WINDOW,
                    null(),
                    null(),
                    &si,
                    &mut pi,
                )
            } != 0,
            "{}",
            io::Error::last_os_error()
        );
        let passes = process_runs_as_this_user(pi.dwProcessId);
        // SAFETY: handles from CreateProcessAsUserW, ended and closed once.
        unsafe {
            TerminateProcess(pi.hProcess, 1);
            CloseHandle(pi.hThread);
            CloseHandle(pi.hProcess);
        }
        assert!(
            !passes,
            "a restricted Low process passed for this user's holder"
        );
    }
}
