//! `agora update`: replaces this program with the newest release (ADR-0019). The release is checked before anything changes: its
//! SHA-256 against the published `.sha256`, and its signature (`ssh-keygen -Y verify`, the same check `agora hub ssh install` makes
//! on the other machine) against the release key the owner pinned in `<AGORA_HOME>/release-key` or `--release-key`. Running agents
//! are not touched; the new program runs when the node is started again, and agents are resumed by their own session ids.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use agora_i18n::key;

use crate::{home, Opts, Say};

/// Where release files are published, unless `--release-url` says otherwise (the same default `hub ssh install` uses).
const RELEASE_BASE: &str = "https://github.com/ersinkoc/Agora/releases/download";

/// The release target of this program, `None` for a platform that ships no binary.
pub fn target() -> Option<(&'static str, &'static str)> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some(("x86_64-unknown-linux-gnu", "")),
        ("linux", "aarch64") => Some(("aarch64-unknown-linux-gnu", "")),
        ("macos", "x86_64") => Some(("x86_64-apple-darwin", "")),
        ("macos", "aarch64") => Some(("aarch64-apple-darwin", "")),
        ("windows", "x86_64") => Some(("x86_64-pc-windows-msvc", ".exe")),
        _ => None,
    }
}

/// `v1.2.3` at the end of the address a `releases/latest` request ended at, as `1.2.3`.
pub fn version_from_redirect(url: &str) -> Option<String> {
    let tag = url.trim().trim_end_matches('/').rsplit('/').next()?;
    let v = tag.strip_prefix('v')?;
    (!v.is_empty() && v.chars().all(|c| c.is_ascii_digit() || c == '.')).then(|| v.to_string())
}

/// Is `a` a newer dotted version than `b`? Anything that is not numbers counts as not newer (never update on a guess).
pub fn newer(a: &str, b: &str) -> bool {
    let n = |s: &str| -> Option<Vec<u64>> { s.split('.').map(|p| p.parse().ok()).collect() };
    matches!((n(a), n(b)), (Some(a), Some(b)) if a > b)
}

fn curl(args: &[&str]) -> Result<String, String> {
    let out = Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--max-time",
            "300",
        ])
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|_| "curl is not installed".to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn sha256_hex(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let d = ring::digest::digest(&ring::digest::SHA256, &bytes);
    Ok(d.as_ref().iter().map(|b| format!("{b:02x}")).collect())
}

/// The release key the owner pinned, if there is one.
fn release_key(o: &Opts) -> Option<PathBuf> {
    let path = o
        .release_key
        .clone()
        .unwrap_or_else(|| home().join("release-key"));
    path.exists().then_some(path)
}

/// Downloads `version` into `dir`, checks it and returns the path of the new program.
fn fetch(
    base: &str,
    version: &str,
    o: &Opts,
    dir: &Path,
) -> Result<PathBuf, (&'static str, String)> {
    let (triple, ext) = target().ok_or((
        key!("cli-update-failed"),
        "no release for this platform".to_string(),
    ))?;
    let name = format!("agora-{triple}{ext}");
    let url = format!("{}/v{version}/{name}", base.trim_end_matches('/'));
    let fail = |e: String| (key!("cli-update-failed"), e);
    let file = dir.join(&name);
    let file_s = file.to_string_lossy().into_owned();
    curl(&["-o", &file_s, &url]).map_err(fail)?;
    let sum_text = curl(&[&format!("{url}.sha256")]).map_err(fail)?;
    let want = sum_text
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if want.len() != 64 || sha256_hex(&file).map_err(fail)? != want {
        return Err((key!("cli-update-checksum"), String::new()));
    }
    match (release_key(o), o.unsigned) {
        (Some(key_file), _) => {
            let sig = dir.join(format!("{name}.sig"));
            curl(&["-o", &sig.to_string_lossy(), &format!("{url}.sig")]).map_err(fail)?;
            let status = Command::new("ssh-keygen")
                .args(["-Y", "verify", "-f"])
                .arg(&key_file)
                .args(["-I", "agora-release", "-n", "file", "-s"])
                .arg(&sig)
                .stdin(std::fs::File::open(&file).map_err(|e| fail(e.to_string()))?)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map_err(|_| fail("ssh-keygen is not installed".to_string()))?;
            if !status.success() {
                return Err((key!("cli-update-signature"), String::new()));
            }
        }
        (None, true) => {}
        (None, false) => return Err((key!("cli-update-needs-key"), String::new())),
    }
    Ok(file)
}

pub fn update(say: &Say, o: &Opts) -> ExitCode {
    let base = o
        .release_url
        .clone()
        .unwrap_or_else(|| RELEASE_BASE.to_string());
    let current = env!("CARGO_PKG_VERSION");
    // The newest release is where `<base without /download>/latest` ends up; `--version` names one and skips the question.
    let wanted = match &o.version {
        Some(v) => v.trim_start_matches('v').to_string(),
        None => {
            let latest = format!(
                "{}/latest",
                base.trim_end_matches('/').trim_end_matches("/download")
            );
            match curl(&["-o", "/dev/null", "-w", "%{url_effective}", &latest])
                .ok()
                .as_deref()
                .and_then(version_from_redirect)
            {
                Some(v) => v,
                None => {
                    say.err(
                        key!("cli-update-failed"),
                        &[("reason", "could not find the newest release".into())],
                    );
                    return ExitCode::from(1);
                }
            }
        }
    };
    if o.version.is_none() && !newer(&wanted, current) {
        say.out(key!("cli-update-current"), &[("version", current.into())]);
        return ExitCode::SUCCESS;
    }
    if o.check {
        say.out(
            key!("cli-update-available"),
            &[("version", wanted.into()), ("current", current.into())],
        );
        return ExitCode::SUCCESS;
    }
    let Ok(exe) = std::env::current_exe() else {
        say.err(
            key!("cli-update-failed"),
            &[("reason", "could not find this program".into())],
        );
        return ExitCode::from(1);
    };
    let dir = std::env::temp_dir().join(format!("agora-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    if std::fs::create_dir_all(&dir).is_err() {
        say.err(
            key!("cli-update-failed"),
            &[("reason", "could not make a temporary folder".into())],
        );
        return ExitCode::from(1);
    }
    // The new program lands beside the old one first, so the final step is one rename on the same disk.
    let result = fetch(&base, &wanted, o, &dir).and_then(|file| {
        let mut beside = exe.clone().into_os_string();
        beside.push(".new");
        let beside = PathBuf::from(beside);
        std::fs::copy(&file, &beside)
            .and_then(|_| agora_platform::replace_executable(&beside, &exe))
            .map_err(|e| (key!("cli-update-failed"), e.to_string()))
    });
    let _ = std::fs::remove_dir_all(&dir);
    match result {
        Ok(()) => {
            say.out(key!("cli-update-done"), &[("version", wanted.into())]);
            ExitCode::SUCCESS
        }
        Err((id, reason)) => {
            say.err(id, &[("reason", reason.into())]);
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newest_version_comes_from_the_end_of_the_redirect() {
        assert_eq!(
            version_from_redirect("https://github.com/o/r/releases/tag/v0.2.0"),
            Some("0.2.0".into())
        );
        assert_eq!(
            version_from_redirect("https://github.com/o/r/releases/tag/v0.2.0/\n"),
            Some("0.2.0".into())
        );
        assert_eq!(
            version_from_redirect("https://github.com/o/r/releases"),
            None
        );
        assert_eq!(version_from_redirect("https://x/tag/v1.2-beta"), None);
    }

    #[test]
    fn only_a_clearly_newer_version_updates() {
        assert!(newer("0.2.0", "0.1.0") && newer("0.10.0", "0.9.9") && newer("1.0.0", "0.99.0"));
        assert!(
            !newer("0.1.0", "0.1.0")
                && !newer("0.1.0", "0.2.0")
                && !newer("x", "0.1.0")
                && !newer("0.2", "x")
        );
    }

    #[test]
    fn this_platform_has_a_release_or_says_so() {
        // Whatever this runs on, the answer is a known triple or nothing, never a guess.
        if let Some((t, _)) = target() {
            assert!(t.contains('-'));
        }
    }
}
