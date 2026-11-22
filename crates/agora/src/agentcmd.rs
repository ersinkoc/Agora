//! `agora agent install NAME --list FILE --root-key HEX|FILE [--go-back]` (ADR-0087 rung 3): installs one agent program from a
//! signed list on this machine. The checks, the download and the files are `agora_adapters::agent_install`; this is the list file,
//! the question and the words. The owner approves by typing the agent's name on this terminal: there is no `--yes`, and a
//! stdin that is not a terminal refuses (`agent.install.not_approved`). No hub, no window, no protocol message.

use std::io::{BufRead, IsTerminal, Read, Write};
use std::path::Path;
use std::process::ExitCode;

use agora_adapters::agent_install::{
    self as inst, strict_tools, Approval, Change, Code, InstallError, Tools,
};
use agora_adapters::agent_packages::{verify_list, AgentList, ListError, MAX_LIST_BYTES};
use agora_adapters::registry::RegistryError;
use agora_i18n::key;

use crate::{home, Opts, Say};

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn failed(say: &Say, e: &InstallError) -> ExitCode {
    let id = match e.code {
        Code::IndexInvalid => key!("cli-agent-install-err-index-invalid"),
        Code::NotApproved => key!("cli-agent-install-err-not-approved"),
        Code::DownloadFailed => key!("cli-agent-install-err-download-failed"),
        Code::ChecksumMismatch => key!("cli-agent-install-err-checksum-mismatch"),
        Code::SignatureInvalid => key!("cli-agent-install-err-signature-invalid"),
        Code::DiskFull => key!("cli-agent-install-err-disk-full"),
        Code::VerifyFailed => key!("cli-agent-install-err-verify-failed"),
        Code::Timeout => key!("cli-agent-install-err-timeout"),
        Code::UnsupportedPlatform => key!("cli-agent-install-err-unsupported-platform"),
        Code::InProgress => key!("cli-agent-install-err-in-progress"),
        Code::Revoked => key!("cli-agent-install-err-revoked"),
        Code::Downgrade => key!("cli-agent-install-err-downgrade"),
        Code::UnsupportedFormat => key!("cli-agent-install-err-unsupported-format"),
        Code::NotOffered => key!("cli-agent-install-err-not-offered"),
        Code::ToolMissing => key!("cli-agent-install-err-tool-missing"),
        Code::IoFailed => key!("cli-agent-install-err-io-failed"),
        Code::Conflict => key!("cli-agent-install-err-conflict"),
    };
    say.err(
        id,
        &[
            ("code", e.code.code().into()),
            ("detail", e.detail.clone().into()),
        ],
    );
    ExitCode::from(1)
}

/// The most of a root-key file that is read: 64 hex characters and a line ending, with room for surrounding whitespace.
const MAX_ROOT_KEY_FILE_BYTES: usize = 4096;

/// The root key: 64 hex characters given directly, or the file that holds them.
fn root_key(o: &Opts, h: &Path) -> Option<String> {
    let given = o
        .root_key
        .clone()
        .unwrap_or_else(|| h.join("agent-list-root-key"));
    let text = given.to_string_lossy().trim().to_string();
    if text.len() == 64 && text.chars().all(|c| c.is_ascii_hexdigit()) {
        return Some(text);
    }
    // A key file is 64 characters and a line ending: never read more than a small bound of it (F31). One byte past the bound is
    // read so a longer file is refused before trimming, never cut into a key (as `open_list` does for the list).
    let mut bytes = Vec::new();
    std::fs::File::open(&given)
        .and_then(|f| {
            f.take(MAX_ROOT_KEY_FILE_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
        })
        .ok()?;
    if bytes.len() > MAX_ROOT_KEY_FILE_BYTES {
        return None;
    }
    let t = String::from_utf8(bytes).ok()?;
    Some(t.trim().to_string()).filter(|t| !t.is_empty())
}

fn list_error(e: &ListError) -> InstallError {
    let detail = match e {
        ListError::Signed(RegistryError::BadSignature) => "signature".to_string(),
        ListError::Signed(RegistryError::Expired) => "expired".into(),
        ListError::Signed(RegistryError::Rollback { .. }) => "rollback".into(),
        ListError::Signed(_) => "malformed".into(),
        ListError::TooLarge => "too_large".into(),
        ListError::Entry { reason, .. } => reason.code().into(),
        ListError::Duplicate { .. } => "duplicate".into(),
        ListError::UnknownPublisher(_) => "unknown_publisher".into(),
        ListError::Revoked(_) | ListError::NotOffered(_) => "unexpected".into(),
    };
    InstallError {
        code: Code::IndexInvalid,
        detail,
    }
}

fn read_line() -> Option<String> {
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    match std::io::stdin().lock().read_line(&mut line) {
        Ok(n) if n > 0 => Some(line),
        _ => None,
    }
}

fn open_list(o: &Opts, h: &Path) -> Result<AgentList, InstallError> {
    let (Some(file), Some(root)) = (&o.list, root_key(o, h)) else {
        return Err(InstallError {
            code: Code::IndexInvalid,
            detail: "no_list_or_key".into(),
        });
    };
    let unreadable = || InstallError {
        code: Code::IndexInvalid,
        detail: "unreadable".into(),
    };
    // Read at most one byte past the cap, so an oversize or endless file is refused as too large, not read into memory (F31).
    let mut bytes = Vec::new();
    std::fs::File::open(file)
        .and_then(|f| f.take(MAX_LIST_BYTES as u64 + 1).read_to_end(&mut bytes))
        .map_err(|_| unreadable())?;
    if bytes.len() > MAX_LIST_BYTES {
        return Err(list_error(&ListError::TooLarge));
    }
    let text = String::from_utf8(bytes).map_err(|_| unreadable())?;
    let list = verify_list(&text, &root, now_ms(), inst::last_seen_version(h))
        .map_err(|e| list_error(&e))?;
    // A list this machine accepted is the newest it has seen: an older one is refused from now on.
    inst::remember_list_version(h, list.version)?;
    Ok(list)
}

pub fn install(say: &Say, o: &Opts) -> ExitCode {
    let Some(name) = o.positional.first().cloned() else {
        say.err(key!("cli-bad-arguments"), &[]);
        return ExitCode::from(2);
    };
    let h = home();
    let list = match open_list(o, &h) {
        Ok(l) => l,
        Err(e) => {
            if e.detail == "no_list_or_key" {
                say.err(key!("cli-agent-install-no-list"), &[]);
                return ExitCode::from(1);
            }
            return failed(say, &e);
        }
    };
    let Some(platform) = agora_platform::platform_key() else {
        return failed(
            say,
            &InstallError {
                code: Code::UnsupportedPlatform,
                detail: "this_system".into(),
            },
        );
    };
    let plan = match inst::plan(&list, &name, &platform, &h) {
        Ok(p) => p,
        Err(e) => return failed(say, &e),
    };
    if plan.change == Change::Same {
        say.out(
            key!("cli-agent-install-current"),
            &[
                ("name", plan.name.clone().into()),
                ("version", plan.version.clone().into()),
            ],
        );
        return ExitCode::SUCCESS;
    }
    let signature = match &plan.signature {
        None => say.text(key!("cli-agent-install-signature-none"), &[]),
        Some((url, key)) => say.text(
            key!("cli-agent-install-signature-by"),
            &[("key", key.clone().into()), ("url", url.clone().into())],
        ),
    };
    say.out(
        key!("cli-agent-install-about"),
        &[
            ("display", plan.display.clone().into()),
            ("name", plan.name.clone().into()),
            ("version", plan.version.clone().into()),
            ("platform", plan.platform.clone().into()),
            ("url", plan.url.clone().into()),
            ("sha256", plan.sha256.clone().into()),
            ("publisher", plan.publisher.clone().into()),
            ("key", plan.publisher_key.clone().into()),
            ("dir", plan.target_dir.display().to_string().into()),
            ("signature", signature.into()),
            ("verify", plan.verify_args.join(" ").into()),
        ],
    );
    // The approval is a person at a terminal: a pipe, a file or a closed stdin cannot give it.
    if !std::io::stdin().is_terminal() {
        return failed(
            say,
            &InstallError {
                code: Code::NotApproved,
                detail: "not_a_terminal".into(),
            },
        );
    }
    let go_back = if plan.change == Change::Downgrade {
        if !o.go_back {
            return failed(
                say,
                &InstallError {
                    code: Code::Downgrade,
                    detail: plan.installed.clone().unwrap_or_default(),
                },
            );
        }
        say.out(
            key!("cli-agent-install-ask-go-back"),
            &[
                (
                    "installed",
                    plan.installed.clone().unwrap_or_default().into(),
                ),
                ("version", plan.version.clone().into()),
            ],
        );
        read_line()
    } else {
        None
    };
    say.out(
        key!("cli-agent-install-ask-name"),
        &[("name", plan.name.clone().into())],
    );
    let typed = read_line().unwrap_or_default();
    let approval = match Approval::typed(&plan, &typed, go_back.as_deref()) {
        Ok(a) => a,
        Err(e) => return failed(say, &e),
    };
    let (resolver, fetcher, signatures) = strict_tools();
    let tools = Tools::new(&resolver, &fetcher, &signatures);
    match inst::install(&plan, &approval, &h, &tools) {
        Ok(done) => {
            say.out(
                key!("cli-agent-install-done"),
                &[
                    ("name", plan.name.into()),
                    ("version", done.version.into()),
                    ("launcher", done.launcher.display().to_string().into()),
                ],
            );
            ExitCode::SUCCESS
        }
        Err(e) => failed(say, &e),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn key_from_file(content: &[u8]) -> Option<String> {
        let d = std::env::temp_dir().join(format!("agora-root-key-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let file = d.join(format!("key-{}", content.len()));
        std::fs::write(&file, content).unwrap();
        let o = Opts {
            root_key: Some(file.clone()),
            ..Opts::default()
        };
        let got = root_key(&o, &d);
        let _ = std::fs::remove_file(&file);
        got
    }

    /// The key file is read to at most its cap; a longer file is refused, never cut and trimmed into a key (F31 review).
    #[test]
    fn the_root_key_file_is_refused_past_its_cap() {
        assert_eq!(
            key_from_file(format!("{KEY}\n").as_bytes()),
            Some(KEY.into())
        );
        // Exactly the cap: the key and whitespace to the last byte.
        let mut at_cap = format!("{KEY}\n").into_bytes();
        at_cap.resize(MAX_ROOT_KEY_FILE_BYTES, b' ');
        assert_eq!(key_from_file(&at_cap), Some(KEY.into()));
        // One byte past the cap is refused, even though the part inside the cap trims to a valid-looking key.
        let mut over = at_cap.clone();
        over.push(b'x');
        assert_eq!(key_from_file(&over), None);
        let mut over_ws = at_cap;
        over_ws.push(b'\n');
        assert_eq!(key_from_file(&over_ws), None);
        assert_eq!(key_from_file(b" \n"), None);
    }
}
