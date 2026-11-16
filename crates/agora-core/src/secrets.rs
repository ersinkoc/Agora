//! ADR-0080: which files are secret, decided by name only. Pure: no I/O, no platform code.
//!
//! The comparison is case-insensitive everywhere, not only on Windows and macOS: a name differing in case is the same file on
//! those systems, and treating it as secret on Linux too costs nothing and keeps one rule (platform code stays in `agora-platform`).

/// Folders whose whole content is secret.
const SECRET_DIRS: &[&str] = &[".ssh", ".aws", ".gnupg"];
/// Names meant to be shared, which `.env.*` would otherwise catch.
const ENV_SHARED: &[&str] = &[".env.example", ".env.sample", ".env.template"];
/// Whole file names.
const SECRET_NAMES: &[&str] = &[
    ".env",
    "id_rsa",
    "id_dsa",
    "id_ecdsa",
    "id_ed25519",
    ".npmrc",
    ".pypirc",
    ".netrc",
    ".git-credentials",
];
/// File name endings (an `id_rsa.pub` is not among them).
const SECRET_SUFFIXES: &[&str] = &[".pem", ".key", ".p12", ".pfx", ".kdbx"];

/// Whether a path relative to the worktree (`/` or `\` separated) names a secret file: its own name matches ADR-0080's list, it is
/// inside a folder named `.ssh`, `.aws` or `.gnupg`, or it is `.docker/config.json`.
pub fn is_secret_path(rel: &str) -> bool {
    let parts: Vec<String> = rel
        .split(['/', '\\'])
        .filter(|c| !c.is_empty() && *c != ".")
        .map(str::to_lowercase)
        .collect();
    let Some((name, dirs)) = parts.split_last() else {
        return false;
    };
    if dirs.iter().any(|d| SECRET_DIRS.contains(&d.as_str())) {
        return true;
    }
    if name == "config.json" && dirs.last().is_some_and(|d| d == ".docker") {
        return true;
    }
    is_secret_name(name)
}

/// [`is_secret_path`] for one file name.
pub fn is_secret_name(name: &str) -> bool {
    let n = name.to_lowercase();
    if ENV_SHARED.contains(&n.as_str()) {
        return false;
    }
    SECRET_NAMES.contains(&n.as_str())
        || n.starts_with(".env.")
        || SECRET_SUFFIXES.iter().any(|s| n.ends_with(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_adr_list_is_secret() {
        for p in [
            ".env",
            "app/.env",
            ".env.local",
            ".env.production",
            "a/b/.ENV",
            "server.pem",
            "certs/tls.KEY",
            "a.p12",
            "a.pfx",
            "vault.kdbx",
            "id_rsa",
            "keys/id_dsa",
            "id_ecdsa",
            "id_ed25519",
            ".npmrc",
            "web/.npmrc",
            ".pypirc",
            ".netrc",
            ".git-credentials",
            ".ssh/config",
            "home/.ssh/known_hosts",
            ".aws/credentials",
            ".gnupg/pubring.kbx",
            "x\\.ssh\\anything",
            ".docker/config.json",
            "a/.Docker/Config.JSON",
        ] {
            assert!(is_secret_path(p), "{p}");
        }
    }

    #[test]
    fn what_is_meant_to_be_shared_or_only_looks_alike_is_not() {
        for p in [
            "",
            "a.txt",
            ".env.example",
            "sub/.env.sample",
            ".env.template",
            ".ENV.EXAMPLE",
            "id_rsa.pub",
            "id_ed25519.pub",
            "environment.ts",
            ".envrc",
            "env",
            "my.env",
            "keyboard.rs",
            "key",
            "pem",
            "monkey.json",
            ".docker/other.json",
            "config.json",
            "docker/config.json",
            "ssh/config",
            "a.ssh/config",
            ".sshx/config",
            ".git-credentials.md",
            "npmrc",
        ] {
            assert!(!is_secret_path(p), "{p}");
        }
    }
}
