//! The global git configuration a confined agent gets (ADR-0078).
//!
//! A confined agent cannot read the person's `~/.gitconfig` (it can hold credentials and settings that make git run a program), and
//! git stops on an unreadable one. So the node writes a small file of its own, with the values of a fixed list of harmless keys, and
//! points git at it (`GIT_CONFIG_GLOBAL`, `GIT_CONFIG_NOSYSTEM=1`). Nothing else of the person's configuration is copied.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The only keys copied (ADR-0078, decision 2). A key is added only by a later ADR.
pub const ALLOWED: &[&str] = &[
    "user.name",
    "user.email",
    "init.defaultBranch",
    "core.autocrlf",
    "core.ignorecase",
    "core.filemode",
    "pull.rebase",
    "push.default",
    "push.autoSetupRemote",
    "color.ui",
];

/// Free text keys: a person's name and address may hold spaces and any printable character. Every other key is a word or a
/// boolean, so its value must be one.
const FREE_TEXT: &[&str] = &["user.name", "user.email"];

const MAX_VALUE: usize = 512;

/// Whether `key` is on the allow-list. Git compares section and variable names without case, so the comparison does too: `User.Name`
/// is the same key.
pub fn is_allowed(key: &str) -> bool {
    ALLOWED.iter().any(|a| a.eq_ignore_ascii_case(key))
}

/// A value that can be written into a quoted git config value as it is, once `\` and `"` are escaped; `None` for one that cannot (a
/// control character, which would end the line or be read as an escape, an empty or oversized value, or, for a key that is not free
/// text, anything but letters, digits, `_`, `-` and `.`).
fn clean(key: &str, value: &str) -> Option<String> {
    if value.is_empty() || value.len() > MAX_VALUE || value.chars().any(char::is_control) {
        return None;
    }
    if !FREE_TEXT.iter().any(|f| f.eq_ignore_ascii_case(key))
        && !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    {
        return None;
    }
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        if matches!(c, '\\' | '"') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    Some(out)
}

/// The text of the file for these `(key, value)` pairs, as read from the person's configuration: only allow-listed keys with a value
/// that can be written safely, grouped by section, in the order of [`ALLOWED`]. Empty when nothing qualifies.
pub fn render(values: &[(String, String)]) -> String {
    let mut out = String::new();
    let mut section = "";
    for allowed in ALLOWED {
        let Some((_, value)) = values.iter().find(|(k, _)| k.eq_ignore_ascii_case(allowed)) else {
            continue;
        };
        let Some(quoted) = clean(allowed, value) else {
            continue;
        };
        let Some((sec, name)) = allowed.split_once('.') else {
            continue;
        };
        if sec != section {
            out.push_str(&format!("[{sec}]\n"));
            section = sec;
        }
        out.push_str(&format!("\t{name} = {quoted}\n"));
    }
    out
}

/// One key of the person's own global configuration, asked of git itself, hardened: `--global` only (no repository, no system file),
/// `--no-includes` (an `include` or `includeIf` is not followed), `-z` (the value comes back exactly, to its last byte), no prompt, no
/// pager. `None` when the key is not set or git cannot answer.
pub fn read_key(key: &str) -> Option<String> {
    let out = key_command(key)?.output().ok()?;
    key_value(out.status, out.stdout)
}

/// The hardened `git config` command [`read_key`] runs for `key` (`None` for a key not on the allow-list), for a caller that runs it
/// under its own limits (`agora doctor`, audit F68).
pub(crate) fn key_command(key: &str) -> Option<Command> {
    if !is_allowed(key) {
        return None;
    }
    let mut c = Command::new("git");
    agora_platform::no_console_window(&mut c);
    c.args(["config", "--global", "--no-includes", "-z", "--get", key])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C")
        // Nothing the node was started with may stand in for the person's own file.
        .env_remove("GIT_CONFIG_GLOBAL")
        .env_remove("GIT_CONFIG_SYSTEM")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_CONFIG_PARAMETERS")
        .env_remove("GIT_DIR")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_WORK_TREE");
    if let Some(dir) = std::env::temp_dir().to_str() {
        // Not inside any repository, whatever the node's own folder is.
        c.current_dir(dir);
    }
    Some(c)
}

/// The value in what [`key_command`] printed, `None` when it did not end successfully (the key is not set, or git cannot answer).
pub(crate) fn key_value(status: std::process::ExitStatus, stdout: Vec<u8>) -> Option<String> {
    if status.code() != Some(0) {
        return None;
    }
    let text = String::from_utf8(stdout).ok()?;
    Some(text.strip_suffix('\0').unwrap_or(&text).to_string())
}

/// Every allow-listed key that the person has set, with its value.
pub fn read_allowed() -> Vec<(String, String)> {
    ALLOWED
        .iter()
        .filter_map(|k| read_key(k).map(|v| ((*k).to_string(), v)))
        .collect()
}

/// Whether the agents have no git identity: no `user.name` or no `user.email` that [`read_key`] finds, which is exactly what is copied
/// (so an identity only in an `include` or the system file is reported missing, because a confined agent would not get it either).
/// `agora doctor` names it on a machine that confines agents (decision 5), asking through [`key_command`] with a time limit.
pub fn identity_missing() -> bool {
    identity_missing_with(read_key)
}

/// [`identity_missing`] with each key read by `read` (the doctor's reads are time-limited, audit F68).
pub(crate) fn identity_missing_with(mut read: impl FnMut(&str) -> Option<String>) -> bool {
    IDENTITY
        .iter()
        .any(|k| read(k).is_none_or(|v| v.trim().is_empty()))
}

/// The keys that make up the git identity a confined agent commits with.
const IDENTITY: [&str; 2] = ["user.name", "user.email"];

/// A short, stable (across runs and versions, unlike `DefaultHasher`) number for a path: FNV-1a over its bytes, folded to 32 bits.
pub(crate) fn place_hash(path: &Path) -> u32 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in path.to_string_lossy().bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    (h ^ (h >> 32)) as u32 // truncation intended: the low 32 bits of the folded hash
}

/// A staging file beside the final file, so the rename stays on the same filesystem.
fn temp_name(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".new");
    PathBuf::from(name)
}

/// The node-owned directory for generated Git configuration (ADR-0098).
pub fn directory(data_dir: &Path) -> PathBuf {
    data_dir.join("git-config")
}

/// Writes a fresh file for this launch. A concurrent launch in the same worktree keeps its own
/// file and the read grant for its inode or ACL.
pub fn write_for(data_dir: &Path) -> Option<PathBuf> {
    write_for_result(data_dir).ok()
}

/// [`write_for`] with the reason it failed.
pub fn write_for_result(data_dir: &Path) -> std::io::Result<PathBuf> {
    let dir = directory(data_dir);
    agora_platform::ensure_private_dir(&dir)?;
    let mut nonce = [0u8; 16];
    getrandom::getrandom(&mut nonce).map_err(|e| std::io::Error::other(e.to_string()))?;
    let path = dir.join(launch_name(owner(), u128::from_ne_bytes(nonce)));
    write_to(&path, &read_allowed())
}

/// The node process that writes launch files: its pid and start time (`None` where the start time cannot be read).
fn owner() -> Option<(u32, u64)> {
    static OWNER: std::sync::OnceLock<Option<(u32, u64)>> = std::sync::OnceLock::new();
    *OWNER.get_or_init(|| {
        let pid = std::process::id();
        agora_platform::process_start_time(pid).map(|start| (pid, start))
    })
}

/// #933: the name of a launch's file: `agent-<pid>-<start>-<32 hex>`, the writing node's pid and start time, so a sweep can tell a
/// file whose node is gone from one a live node (even another node on the same data folder) still uses; `agent-<32 hex>` when the
/// owner is not known (written before #933, or where a start time cannot be read), which no sweep removes.
pub fn launch_name(owner: Option<(u32, u64)>, nonce: u128) -> String {
    match owner {
        Some((pid, start)) => format!("agent-{pid}-{start}-{nonce:032x}"),
        None => format!("agent-{nonce:032x}"),
    }
}

/// What a name in the folder is: `None` when it is not a launch's file (or its `.new` staging copy), `Some(None)` for one with no
/// owner ([`launch_name`]), `Some(Some((pid, start)))` for one a node wrote. Nothing but a launch's file is the node's to remove.
fn launch_owner(name: &str) -> Option<Option<(u32, u64)>> {
    let base = name.strip_suffix(".new").unwrap_or(name);
    let rest = base.strip_prefix("agent-")?;
    let hex = |h: &str| h.len() == 32 && h.bytes().all(|b| b.is_ascii_hexdigit());
    let digits = |d: &str| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit());
    let mut parts = rest.split('-');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(h), None, None, None) if hex(h) => Some(None),
        (Some(pid), Some(start), Some(h), None) if digits(pid) && digits(start) && hex(h) => {
            Some(Some((pid.parse().ok()?, start.parse().ok()?)))
        }
        _ => None,
    }
}

/// The folder `git-config` under `data_dir`, held open: the data folder is opened once by path, the `git-config` step never follows a
/// link (one there is refused).
fn held_directory(data_dir: &Path) -> std::io::Result<agora_platform::ConfinedDir> {
    let name = directory(Path::new(""));
    agora_platform::ConfinedDir::open_root(data_dir)?.subdir(name.as_os_str(), false)
}

/// One launch's generated file (#933): it is removed when this is dropped. The node keeps it beside what the launch started (the
/// terminal's handle, the adapter's entry), and drops it once that process is ended and collected, is replaced by a restart (which
/// writes a fresh file of its own), or never started (a start that fails after the file was written).
#[derive(Debug)]
pub struct LaunchFile(PathBuf);

impl LaunchFile {
    /// Takes ownership of the file at `path`, which [`write_for`] made for one launch.
    pub fn new(path: PathBuf) -> Self {
        Self(path)
    }

    /// Where the file is.
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for LaunchFile {
    fn drop(&mut self) {
        // Through the held folder, as the sweep removes (a link where the folder was is not followed). Already gone (removed by
        // hand, or the folder with it) is fine: there is nothing left to remove.
        let (Some(folder), Some(name)) = (self.0.parent(), self.0.file_name()) else {
            return;
        };
        let (Some(root), Some(sub)) = (folder.parent(), folder.file_name()) else {
            return;
        };
        if let Ok(dir) =
            agora_platform::ConfinedDir::open_root(root).and_then(|r| r.subdir(sub, false))
        {
            let _ = dir.remove_file(name);
        }
    }
}

/// #933: removes from the node's generated Git folder under `data_dir` the launch files (and staging copies) no live node uses: one
/// this node wrote whose name is not in `keep` (the files of the agents it runs), and one whose writing node is gone (its pid no longer
/// names a process started at the time in the name: a run that was killed, or a machine that stopped). A file of another live node
/// (two nodes on one data folder) is kept, and so is one with no owner in its name. Only this folder, listed and emptied through one
/// held handle: the legacy `~/.agora/git-config` copies are not touched (ADR-0098, decision 4), a `git-config` that is a link is left
/// alone, and so is anything that is not a file. Returns how many were removed.
pub fn sweep(data_dir: &Path, keep: &[&std::ffi::OsStr]) -> usize {
    sweep_as(data_dir, keep, owner(), agora_platform::process_start_time)
}

/// [`sweep`] for the node `me`, asking `started` when a process began.
fn sweep_as(
    data_dir: &Path,
    keep: &[&std::ffi::OsStr],
    me: Option<(u32, u64)>,
    started: impl Fn(u32) -> Option<u64>,
) -> usize {
    let Ok(dir) = held_directory(data_dir) else {
        return 0;
    };
    let Ok(items) = dir.list() else {
        return 0;
    };
    let mut removed = 0;
    for item in items {
        let Some(Some(owner)) = item.name.to_str().and_then(launch_owner) else {
            continue;
        };
        let gone = if Some(owner) == me {
            !keep.contains(&item.name.as_os_str())
        } else {
            started(owner.0) != Some(owner.1)
        };
        if gone && !item.is_dir && dir.remove_file(&item.name).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Writes the file at `path` (its folder made ready by the caller).
fn write_to(path: &Path, values: &[(String, String)]) -> std::io::Result<PathBuf> {
    let path = path.to_path_buf();
    let tmp = temp_name(&path);
    let _ = std::fs::remove_file(&tmp);
    let text = render(values);
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(text.as_bytes())?;
    }
    // A scanner may briefly hold the staging file on Windows. Retry sharing and access errors.
    let mut last = None;
    for _ in 0..5 {
        match std::fs::rename(&tmp, &path) {
            Ok(()) => return Ok(path),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ResourceBusy
                ) =>
            {
                last = Some(e);
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(e) => {
                last = Some(e);
                break;
            }
        }
    }
    let _ = std::fs::remove_file(&tmp);
    Err(last.unwrap_or_else(|| std::io::Error::other("rename failed")))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter()
            .map(|(a, b)| ((*a).to_string(), (*b).to_string()))
            .collect()
    }

    #[test]
    fn only_the_listed_keys_pass() {
        for k in ALLOWED {
            assert!(is_allowed(k), "{k}");
        }
        assert!(is_allowed("User.Email"));
        for k in [
            "credential.helper",
            "credential.https://example.com.helper",
            "includeIf.gitdir:/x/.path",
            "include.path",
            "url.https://t@example.com/.insteadOf",
            "alias.x",
            "alias.!x",
            "core.sshCommand",
            "core.hooksPath",
            "core.fsmonitor",
            "core.pager",
            "core.editor",
            "http.extraHeader",
            "commit.gpgsign",
            "gpg.program",
            "user.signingkey",
            "user.name.extra",
            "",
        ] {
            assert!(!is_allowed(k), "{k}");
        }
    }

    #[test]
    fn dangerous_keys_never_reach_the_text() {
        let text = render(&pairs(&[
            ("user.name", "Ada Example"),
            ("user.email", "ada@example.com"),
            ("credential.helper", "store"),
            ("includeIf.gitdir:/x/.path", "/etc/evil"),
            (
                "url.https://t@example.com/.insteadOf",
                "https://example.com/",
            ),
            ("alias.x", "!rm -rf /"),
            ("core.sshCommand", "ssh -i /k"),
            ("commit.gpgsign", "true"),
            ("core.hooksPath", "/h"),
        ]));
        assert_eq!(
            text,
            "[user]\n\tname = \"Ada Example\"\n\temail = \"ada@example.com\"\n"
        );
    }

    #[test]
    fn an_unset_key_is_left_out_and_nothing_is_an_empty_file() {
        assert_eq!(render(&[]), "");
        let t = render(&pairs(&[("init.defaultBranch", "main")]));
        assert_eq!(t, "[init]\n\tdefaultBranch = \"main\"\n");
    }

    #[test]
    fn a_value_cannot_add_keys_or_sections() {
        let text = render(&pairs(&[
            ("user.name", "Eve\n[credential]\n\thelper = !x"),
            ("user.email", "a@b\r\n[core]\n\tsshCommand = x"),
            ("pull.rebase", "true\n[alias]\n\tx = !y"),
            ("core.autocrlf", "input\0"),
            ("push.default", "simple]"),
        ]));
        assert_eq!(text, "");
        // A quote and a backslash are escaped, so they cannot end the value early.
        let text = render(&pairs(&[(
            "user.name",
            r#"Eve" ; [core] sshCommand = x \"#,
        )]));
        assert_eq!(
            text,
            "[user]\n\tname = \"Eve\\\" ; [core] sshCommand = x \\\\\"\n"
        );
        // The comment characters stay inside the quotes.
        assert!(render(&pairs(&[("user.name", "A # B ; C")])).contains("\"A # B ; C\""));
    }

    #[test]
    fn oversized_values_are_dropped() {
        assert_eq!(
            render(&pairs(&[("user.name", &"x".repeat(MAX_VALUE + 1))])),
            ""
        );
    }

    #[test]
    fn only_the_nodes_own_launch_names_are_swept() {
        let hex = "0123456789abcdef0123456789ABCDEF";
        assert_eq!(launch_owner(&format!("agent-{hex}")), Some(None));
        assert_eq!(
            launch_owner(&format!("agent-12-345-{hex}.new")),
            Some(Some((12, 345)))
        );
        assert_eq!(
            launch_name(Some((7, 9)), 0xab),
            format!("agent-7-9-{:032x}", 0xab)
        );
        assert_eq!(
            launch_owner(&launch_name(Some((7, 9)), 0xab)),
            Some(Some((7, 9)))
        );
        assert_eq!(launch_owner(&launch_name(None, 1)), Some(None));
        for name in [
            "agent-",
            "agent-123",
            "neighbor",
            "agent-0123456789abcdef0123456789abcdeg",
            "agent-0123456789abcdef0123456789abcdef0",
            "agent-0123456789abcdef0123456789abcdef.old",
            "xagent-0123456789abcdef0123456789abcdef",
            "agent-1--0123456789abcdef0123456789abcdef",
            "agent-x-1-0123456789abcdef0123456789abcdef",
            "agent-1-2-3-0123456789abcdef0123456789abcdef",
            "agent-99999999999-1-0123456789abcdef0123456789abcdef",
        ] {
            assert_eq!(launch_owner(name), None, "{name}");
        }
    }

    #[test]
    fn a_sweep_removes_only_files_whose_node_is_gone_or_that_this_node_no_longer_uses() {
        let data = std::env::temp_dir().join(format!("agora-gc-sweep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        let folder = directory(&data);
        std::fs::create_dir_all(&folder).unwrap();
        let me = Some((10, 100));
        let names = [
            launch_name(me, 1),              // this node's, in use
            launch_name(me, 2),              // this node's, no longer used
            launch_name(Some((20, 200)), 3), // a live node's
            launch_name(Some((30, 300)), 4), // a node that is gone
            format!("{}.new", launch_name(Some((30, 300)), 5)),
            launch_name(Some((20, 999)), 6), // pid 20 now names another process
            launch_name(None, 7),            // no owner
            "neighbor".to_string(),
        ];
        for n in &names {
            std::fs::write(folder.join(n), "x").unwrap();
        }
        let keep = [std::ffi::OsStr::new(&names[0])];
        let started = |pid| (pid == 20).then_some(200);
        assert_eq!(sweep_as(&data, &keep, me, started), 4);
        let mut left: Vec<String> = std::fs::read_dir(&folder)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        let mut want = vec![
            names[0].clone(),
            names[2].clone(),
            names[6].clone(),
            names[7].clone(),
        ];
        want.sort();
        assert_eq!(left, want);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn a_launch_file_goes_with_its_owner() {
        let dir = std::env::temp_dir().join(format!("agora-gc-launch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("agent-x");
        std::fs::write(&p, "x").unwrap();
        let file = LaunchFile::new(p.clone());
        assert_eq!(file.path(), p);
        drop(file);
        assert!(!p.exists());
        // One removed already is not an error.
        drop(LaunchFile::new(p));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn writing_replaces_a_link_and_the_old_text() {
        let dir = std::env::temp_dir().join(format!("agora-gc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let wt = dir.join("agent-9");
        std::fs::create_dir_all(&wt).unwrap();
        let p = dir.join("agent-9.gitconfig");
        let victim = dir.join("victim");
        std::fs::write(&victim, "keep").unwrap();
        #[cfg(unix)]
        {
            let _ = std::fs::remove_file(&p);
            std::os::unix::fs::symlink(&victim, &p).unwrap();
        }
        let written = write_to(&p, &pairs(&[("user.name", "A")])).unwrap();
        assert_eq!(written, p);
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            "[user]\n\tname = \"A\"\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&p);
    }
}
