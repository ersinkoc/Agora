//! Windows agent confinement (ADR-0050): a restricted token with one restricting SID per agent.
//!
//! Windows grants an access to a process with a restricted token only when the normal check **and** the check against the restricting
//! SIDs both pass. The restricting list is this agent's own random SID, the two `ALL (RESTRICTED) APPLICATION PACKAGES` groups and the
//! logon SID. It must **not** contain `Everyone` or `Users`: a folder whose ACL grants those groups would then satisfy both checks
//! without any entry for the agent (issue #230). The application-package groups are what Windows itself grants on `C:\Windows` and
//! `Program Files`, so the system and installed tools stay readable. The node adds an ACL entry for the agent's SID to exactly the
//! folders the agent may use; everything else (the profile, other drives, the node's keys, another agent's folders, a folder that
//! happens to be writable by every user) is refused.
//!
//! The entries are recorded in a ledger file *before* they are added and removed when the agent ends; [`recover_stale`] undoes what a
//! crashed node left behind. The feature stays off ([`enabled`]) until the maintainer switches it on after real-machine checks.

use std::io;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, LocalFree, HANDLE};
use windows_sys::Win32::Security::Authorization::{
    BuildTrusteeWithSidW, ConvertStringSidToSidW, GetNamedSecurityInfoW, SetEntriesInAclW,
    SetNamedSecurityInfoW, EXPLICIT_ACCESS_W, GRANT_ACCESS, REVOKE_ACCESS, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::Cryptography::{
    BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
};
use windows_sys::Win32::Security::{
    CreateRestrictedToken, GetTokenInformation, TokenLogonSid, DACL_SECURITY_INFORMATION,
    DISABLE_MAX_PRIVILEGE, PSID, SID_AND_ATTRIBUTES, TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE,
    TOKEN_GROUPS, TOKEN_QUERY,
};
use windows_sys::Win32::Storage::FileSystem::{
    DELETE, FILE_DELETE_CHILD, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

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

fn write_ledger(file: &Path, sid: &str, paths: &[PathBuf]) -> io::Result<()> {
    let mut text = String::new();
    for p in paths {
        text.push_str(sid);
        text.push('\t');
        text.push_str(&p.to_string_lossy());
        text.push('\n');
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
        write_ledger(&ledger, &sid_text, &all)?;
        let mut confinement = Self {
            sid,
            granted: Vec::new(),
            ledger,
            token: Token(null_mut()),
        };
        for (path, mask) in wanted {
            // On failure `confinement` is dropped, which undoes what was added so far.
            change_acl(&path, confinement.sid.0, mask, true)?;
            confinement.granted.push(path);
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
        // A failed removal keeps the ledger so the next start retries; the SID dies with the agent either way.
        if all_removed {
            let _ = std::fs::remove_file(&self.ledger);
        }
    }
}

/// The current process's token, restricted: every privilege dropped, and the restricting list is the agent's SID, `ALL APPLICATION
/// PACKAGES`, `ALL RESTRICTED APPLICATION PACKAGES` and the logon SID (without that last one the console and desktop refuse the process).
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

    // The groups Windows grants read and execute to on the system folders and `Program Files` for sandboxed programs.
    let packages = Sid::from_string("S-1-15-2-1")?;
    let restricted_packages = Sid::from_string("S-1-15-2-2")?;
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
            Sid: packages.0,
            Attributes: 0,
        },
        SID_AND_ATTRIBUTES {
            Sid: restricted_packages.0,
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
        let mut clean = true;
        for line in text.lines() {
            let Some((sid_text, folder)) = line.split_once('\t') else {
                continue;
            };
            let Ok(sid) = Sid::from_string(sid_text) else {
                continue;
            };
            let folder = Path::new(folder);
            // A folder that is gone has nothing to undo.
            if folder.exists() && change_acl(folder, sid.0, 0, false).is_err() {
                clean = false;
            }
        }
        if clean {
            let _ = std::fs::remove_file(&path);
        }
    }
}
