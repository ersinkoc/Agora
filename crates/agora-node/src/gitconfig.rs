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
    let out = c.output().ok()?;
    if out.status.code() != Some(0) {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
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
/// `agora doctor` names it on a machine that confines agents (decision 5).
pub fn identity_missing() -> bool {
    ["user.name", "user.email"]
        .iter()
        .any(|k| read_key(k).is_none_or(|v| v.trim().is_empty()))
}

/// The folder of the files where the system's temp folder is out of the agent's reach (macOS, Windows): one of the node's own inside
/// it, made private by the platform ([`agora_platform::ensure_private_dir`]) before anything is written, because the temp folder's
/// own permissions can admit other programs (on Windows, other sandboxes) that could then rewrite what the agent trusts.
fn private_folder() -> PathBuf {
    std::env::temp_dir().join("agora-git-config")
}

/// Makes the folder the file goes in ready, where it is one of the node's own (macOS, Windows): created if missing, then checked and
/// made private through one handle of the folder itself (`agora_platform::ensure_private_dir`: not a link, this user's, mode 0700 or an
/// owner-only ACL). Nothing to do where the file lies beside the worktree. An error means the folder cannot be trusted: no file.
pub fn prepare_folder() -> std::io::Result<()> {
    if agora_platform::PRIVATE_TEMP {
        agora_platform::ensure_private_dir(&private_folder())?;
    }
    Ok(())
}

/// A short, stable (across runs and versions, unlike `DefaultHasher`) number for a path: FNV-1a over its bytes, folded to 32 bits.
fn place_hash(path: &Path) -> u32 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in path.to_string_lossy().bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    (h ^ (h >> 32)) as u32 // truncation intended: the low 32 bits of the folded hash
}

/// Where the file of the agent that works in `worktree` is written. Always outside what the agent may write: where the system's temp
/// folder is out of its reach (`PRIVATE_TEMP`, macOS and Windows) it is there, next to, not inside, the agent's private temp folder;
/// elsewhere (Linux, where `/tmp` is writable to the agent) beside the agent's worktree in the node's worktrees folder, of which the
/// agent has only its own worktree.
pub fn path_for(worktree: &Path) -> PathBuf {
    let name = worktree
        .file_name()
        .map_or_else(|| "agent".into(), |n| n.to_string_lossy().into_owned());
    // The place is part of the name: in the shared temp folder two worktrees called `a` of different projects must not share a file.
    let file = format!("agora-gitconfig-{name}-{:08x}", place_hash(worktree));
    if agora_platform::PRIVATE_TEMP {
        return private_folder().join(file);
    }
    worktree
        .parent()
        .map_or_else(std::env::temp_dir, Path::to_path_buf)
        .join(format!(".{file}"))
}

/// The name the file is first written under: `.new` appended, not `with_extension`, which would cut a worktree name such as `fix.v1`
/// and give it the name of `fix.v2`.
fn temp_name(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".new");
    PathBuf::from(name)
}

/// Writes the file for the agent in `worktree` from the person's configuration, replacing the last one. Written to a new name and
/// renamed over the old, so a link left at the old name is replaced and never followed. Returns the path, or `None` when it could not
/// be written (git then behaves as it did before: an agent that cannot read a global configuration).
pub fn write_for(worktree: &Path) -> Option<PathBuf> {
    write_for_result(worktree).ok()
}

/// [`write_for`] with the reason it failed.
pub fn write_for_result(worktree: &Path) -> std::io::Result<PathBuf> {
    write_values(worktree, &read_allowed())
}

fn write_values(worktree: &Path, values: &[(String, String)]) -> std::io::Result<PathBuf> {
    let path = path_for(worktree);
    prepare_folder()?;
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
    // Another program (a virus scanner, an indexer) may hold the old file for a moment, which makes a rename over it fail with a sharing
    // or access error on Windows: only that is tried again. Any other error ends it, and the old file is never removed first, so the agent
    // is never left with no file where it had one (it keeps the last one, or, failing the first write, none: git then behaves as before).
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

/// The environment that points a confined agent's git at its file (decisions 1 and 3). The node's own environment may carry git
/// settings that apply on top of any global file (`GIT_CONFIG_COUNT` with `GIT_CONFIG_KEY_n`/`VALUE_n`, `GIT_CONFIG_PARAMETERS`, which
/// `git -c` leaves for child gits): the count is set to 0 (the key and value variables are then not read) and the parameters emptied.
pub fn env_for(file: &Path) -> Vec<(String, String)> {
    vec![
        ("GIT_CONFIG_COUNT".into(), "0".into()),
        ("GIT_CONFIG_PARAMETERS".into(), String::new()),
        (
            "GIT_CONFIG_GLOBAL".into(),
            file.to_string_lossy().into_owned(),
        ),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
    ]
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
    fn a_dotted_worktree_name_has_a_temp_name_of_its_own() {
        let dir = std::env::temp_dir().join(format!("agora-gc-dot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            temp_name(Path::new("/x/.agora-gitconfig-fix.v1")),
            Path::new("/x/.agora-gitconfig-fix.v1.new")
        );
        assert_ne!(
            temp_name(&path_for(Path::new("/w/fix.v1"))),
            temp_name(&path_for(Path::new("/w/fix.v2")))
        );
        let (a, b) = (dir.join("fix.v1"), dir.join("fix.v2"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let pa = write_values(&a, &pairs(&[("user.name", "A")])).unwrap();
        let pb = write_values(&b, &pairs(&[("user.name", "B")])).unwrap();
        assert_ne!(pa, pb);
        assert!(std::fs::read_to_string(&pa).unwrap().contains("\"A\""));
        assert!(std::fs::read_to_string(&pb).unwrap().contains("\"B\""));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(pa);
        let _ = std::fs::remove_file(pb);
    }

    #[test]
    fn oversized_values_are_dropped() {
        assert_eq!(
            render(&pairs(&[("user.name", &"x".repeat(MAX_VALUE + 1))])),
            ""
        );
    }

    #[test]
    fn the_file_sits_outside_the_agents_write_grants() {
        let wt = Path::new("/home/u/.agora/worktrees/agent-1");
        let p = path_for(wt);
        assert!(!p.starts_with(wt));
        assert_ne!(p.parent(), Some(wt));
        assert!(p.to_string_lossy().contains("agent-1"));
    }

    #[test]
    fn writing_replaces_a_link_and_the_old_text() {
        let dir = std::env::temp_dir().join(format!("agora-gc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let wt = dir.join("agent-9");
        std::fs::create_dir_all(&wt).unwrap();
        let p = path_for(&wt);
        let victim = dir.join("victim");
        std::fs::write(&victim, "keep").unwrap();
        #[cfg(unix)]
        {
            let _ = std::fs::remove_file(&p);
            std::os::unix::fs::symlink(&victim, &p).unwrap();
        }
        let written = write_values(&wt, &pairs(&[("user.name", "A")])).unwrap();
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
