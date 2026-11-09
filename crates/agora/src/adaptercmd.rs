//! `agora adapter install|list|remove` (ADR-0044): adapters and manifests from a signed registry. The index and the package are
//! checked by `agora_adapters::registry` and read by `agora_adapters::package`; this is the download, the question and the files.
//! Nothing that is installed runs here.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use agora_adapters::package::{self, PackageError};
use agora_adapters::registry::{self, RegistryError};
use agora_i18n::key;

use crate::{home, Opts, Say};

fn curl(url: &str) -> Result<Vec<u8>, String> {
    let https = url.starts_with("https://");
    let loopback = ["http://127.0.0.1", "http://localhost", "http://[::1]"]
        .iter()
        .any(|p| {
            url.strip_prefix(p)
                .is_some_and(|r| r.is_empty() || r.starts_with([':', '/']))
        });
    if !https && !loopback {
        return Err("the registry address must be https".into());
    }
    let out = Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--max-time",
            "300",
        ])
        .arg(url)
        .stdin(Stdio::null())
        .output()
        .map_err(|_| "curl is not installed".to_string())?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn read_trim(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn private_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)?;
    agora_platform::restrict_to_owner(path)
}

fn fail(say: &Say, id: &'static str, reason: String) -> ExitCode {
    say.err(id, &[("reason", reason.into())]);
    ExitCode::from(1)
}

fn registry_error(e: &RegistryError) -> String {
    match e {
        RegistryError::Malformed => "the registry sent something unreadable".into(),
        RegistryError::BadSignature => "the signature does not match".into(),
        RegistryError::Expired => "the registry index has expired".into(),
        RegistryError::Rollback { .. } => "the registry index is older than one seen before".into(),
        RegistryError::UnknownPackage(n) => format!("the registry has no package {n}"),
        RegistryError::UnknownPublisher(p) => format!("unknown publisher {p}"),
        RegistryError::PublisherRevoked(p) => format!("publisher {p} has been revoked"),
        RegistryError::HashMismatch => "the package does not match the index".into(),
    }
}

fn package_error(e: &PackageError) -> String {
    format!("the package is not acceptable ({e:?})")
}

fn approved(h: &Path, id: &str, key: &str) -> bool {
    read_trim(&h.join("publishers")).is_some_and(|t| {
        t.lines().any(|l| {
            let mut p = l.split_whitespace();
            p.next() == Some(id) && p.next() == Some(key)
        })
    })
}

fn ask() -> bool {
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).is_ok()
        && matches!(
            line.trim().to_ascii_lowercase().as_str(),
            "y" | "yes" | "e" | "evet"
        )
}

pub fn install(say: &Say, o: &Opts) -> ExitCode {
    let h = home();
    let Some(name) = o.positional.first().cloned() else {
        say.err(key!("cli-bad-arguments"), &[]);
        return ExitCode::from(2);
    };
    if !package::valid_name(&name) {
        return fail(
            say,
            key!("cli-adapter-failed"),
            "not a valid package name".into(),
        );
    }
    let base = o
        .registry
        .clone()
        .or_else(|| read_trim(&crate::hubcli::hub_dir(o).join("registry-url")))
        .filter(|u| !u.is_empty());
    let Some(base) = base else {
        return fail(say, key!("cli-adapter-no-registry"), String::new());
    };
    let key_file = o
        .root_key
        .clone()
        .unwrap_or_else(|| h.join("registry-root-key"));
    let Some(root) = read_trim(&key_file) else {
        return fail(say, key!("cli-adapter-no-key"), String::new());
    };
    let base = base.trim_end_matches('/').to_string();
    let seen: u64 = read_trim(&h.join("registry-version"))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let index_text = match curl(&format!("{base}/index.json")) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) => return fail(say, key!("cli-adapter-failed"), e),
    };
    let index = match registry::verify_index(&index_text, &root, now_ms(), seen) {
        Ok(i) => i,
        Err(e) => return fail(say, key!("cli-adapter-failed"), registry_error(&e)),
    };
    let Some(entry) = index.packages.iter().find(|p| p.name == name).cloned() else {
        return fail(
            say,
            key!("cli-adapter-failed"),
            registry_error(&RegistryError::UnknownPackage(name)),
        );
    };
    if !package::valid_name(&entry.name)
        || !entry
            .version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        || entry.version.is_empty()
    {
        return fail(
            say,
            key!("cli-adapter-failed"),
            "the index names an unsafe package".into(),
        );
    }
    let stem = format!("{base}/packages/{}-{}", entry.name, entry.version);
    let bytes = match curl(&format!("{stem}.tar")) {
        Ok(b) => b,
        Err(e) => return fail(say, key!("cli-adapter-failed"), e),
    };
    let sig = match curl(&format!("{stem}.sig")) {
        Ok(b) => String::from_utf8_lossy(&b).trim().to_string(),
        Err(e) => return fail(say, key!("cli-adapter-failed"), e),
    };
    if let Err(e) = index.verify_package(&name, &bytes, &sig) {
        return fail(say, key!("cli-adapter-failed"), registry_error(&e));
    }
    let entries = match package::unpack(&bytes) {
        Ok(e) => e,
        Err(e) => return fail(say, key!("cli-adapter-failed"), package_error(&e)),
    };
    let dir = package::package_dir(&h, &name, &entry.version);
    let manifest = match package::registered_manifest(&name, &entries, &dir) {
        Ok(m) => m,
        Err(e) => return fail(say, key!("cli-adapter-failed"), package_error(&e)),
    };
    let manifests = o.manifests.clone().unwrap_or_else(|| h.join("manifests"));
    let target = manifests.join(format!("{name}.toml"));
    if target.exists() && !h.join("adapters").join(&name).exists() {
        return fail(say, key!("cli-adapter-exists"), String::new());
    }
    let pub_key = index
        .publishers()
        .get(entry.publisher.as_str())
        .map(|p| p.key.clone())
        .unwrap_or_default();
    let known = approved(&h, &entry.publisher, &pub_key);
    let command = package::adapter_command(&entries)
        .map(|c| c.join(" "))
        .unwrap_or_default();
    say.out(
        key!("cli-adapter-about"),
        &[
            ("name", name.clone().into()),
            ("version", entry.version.clone().into()),
            ("publisher", entry.publisher.clone().into()),
            ("key", pub_key.clone().into()),
            ("hash", entry.sha256.clone().into()),
            ("command", command.into()),
        ],
    );
    if !(known && o.yes) {
        say.out(
            if known {
                key!("cli-adapter-ask-known")
            } else {
                key!("cli-adapter-ask-new")
            },
            &[],
        );
        if !ask() {
            say.out(key!("cli-adapter-declined"), &[]);
            return ExitCode::from(1);
        }
    }
    let write = || -> std::io::Result<()> {
        private_dir(&h)?;
        let parent = dir.parent().unwrap_or(&h).to_path_buf();
        private_dir(&parent)?;
        let tmp: PathBuf = parent.join(format!(".tmp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp)?;
        for e in &entries {
            let to = tmp.join(&e.path);
            if let Some(p) = to.parent() {
                std::fs::create_dir_all(p)?;
            }
            std::fs::write(&to, &e.data)?;
            agora_platform::set_executable(&to, e.executable)?;
        }
        std::fs::write(tmp.join("publisher"), format!("{}\n", entry.publisher))?;
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::rename(&tmp, &dir)?;
        for old in std::fs::read_dir(&parent)?.flatten() {
            if old.path() != dir {
                let _ = std::fs::remove_dir_all(old.path());
            }
        }
        std::fs::create_dir_all(&manifests)?;
        std::fs::write(&target, &manifest)?;
        std::fs::write(h.join("registry-version"), format!("{}\n", index.version))?;
        std::fs::write(h.join("registry-index"), &index_text)?;
        if !known {
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(h.join("publishers"))?;
            writeln!(f, "{} {}", entry.publisher, pub_key)?;
        }
        Ok(())
    };
    match write() {
        Ok(()) => {
            say.out(
                key!("cli-adapter-installed"),
                &[("name", name.into()), ("version", entry.version.into())],
            );
            ExitCode::SUCCESS
        }
        Err(e) => fail(say, key!("cli-adapter-failed"), e.to_string()),
    }
}

pub fn list(say: &Say, _o: &Opts) -> ExitCode {
    let h = home();
    let revoked = read_trim(&h.join("registry-index"))
        .map(|t| registry::revoked_publishers(&t))
        .unwrap_or_default();
    let mut any = false;
    for n in std::fs::read_dir(h.join("adapters"))
        .into_iter()
        .flatten()
        .flatten()
    {
        for v in std::fs::read_dir(n.path()).into_iter().flatten().flatten() {
            let publisher = read_trim(&v.path().join("publisher")).unwrap_or_default();
            any = true;
            let id = if revoked.contains(&publisher) {
                key!("cli-adapter-listed-revoked")
            } else {
                key!("cli-adapter-listed")
            };
            say.out(
                id,
                &[
                    ("name", n.file_name().to_string_lossy().into_owned().into()),
                    (
                        "version",
                        v.file_name().to_string_lossy().into_owned().into(),
                    ),
                    ("publisher", publisher.into()),
                ],
            );
        }
    }
    if !any {
        say.out(key!("cli-adapter-none"), &[]);
    }
    ExitCode::SUCCESS
}

pub fn remove(say: &Say, o: &Opts) -> ExitCode {
    let h = home();
    let Some(name) = o
        .positional
        .first()
        .cloned()
        .filter(|n| package::valid_name(n))
    else {
        say.err(key!("cli-bad-arguments"), &[]);
        return ExitCode::from(2);
    };
    let dir = h.join("adapters").join(&name);
    if !dir.exists() {
        return fail(
            say,
            key!("cli-adapter-failed"),
            format!("{name} is not installed"),
        );
    }
    let manifests = o.manifests.clone().unwrap_or_else(|| h.join("manifests"));
    let r = std::fs::remove_dir_all(&dir).and_then(|()| {
        match std::fs::remove_file(manifests.join(format!("{name}.toml"))) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    });
    match r {
        Ok(()) => {
            say.out(key!("cli-adapter-removed"), &[("name", name.into())]);
            ExitCode::SUCCESS
        }
        Err(e) => fail(say, key!("cli-adapter-failed"), e.to_string()),
    }
}
