//! Project identity: normalized remote URL + root commit hash (ADR-0003), independent of machine.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectError {
    /// Root commit is not 40 (SHA-1) or 64 (SHA-256) hex digits.
    InvalidRootCommit,
    /// The remote URL could not be understood.
    InvalidRemote,
}

/// Machine-independent project identity. A project without a remote uses the root commit only.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProjectId {
    remote: Option<String>,
    root_commit: String,
}

impl ProjectId {
    pub fn new(remote_url: Option<&str>, root_commit: &str) -> Result<Self, ProjectError> {
        let root = root_commit.trim().to_ascii_lowercase();
        if !matches!(root.len(), 40 | 64) || !root.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ProjectError::InvalidRootCommit);
        }
        let remote = remote_url.map(normalize_remote).transpose()?;
        Ok(Self {
            remote,
            root_commit: root,
        })
    }

    pub fn remote(&self) -> Option<&str> {
        self.remote.as_deref()
    }

    pub fn root_commit(&self) -> &str {
        &self.root_commit
    }
}

impl fmt::Display for ProjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.remote {
            Some(r) => write!(f, "{r}#{}", self.root_commit),
            None => write!(f, "#{}", self.root_commit),
        }
    }
}

/// Normalize a git remote to `host/owner/repo`: scheme, user info, port, a trailing `.git`, and trailing
/// slashes are dropped; the host is lowercased; the path keeps its case.
/// Handles `https://`, `http://`, `ssh://`, `git://`, and scp-like `user@host:path` forms.
pub fn normalize_remote(url: &str) -> Result<String, ProjectError> {
    let url = url.trim();
    let (host_part, path) = if let Some((_, rest)) = url.split_once("://") {
        let (authority, path) = rest.split_once('/').ok_or(ProjectError::InvalidRemote)?;
        (authority.to_string(), path.to_string())
    } else if let Some((authority, path)) = url.split_once(':') {
        // scp-like: [user@]host:path (no slash before the colon)
        if authority.contains('/') {
            return Err(ProjectError::InvalidRemote);
        }
        (authority.to_string(), path.to_string())
    } else {
        return Err(ProjectError::InvalidRemote);
    };
    let host_port = host_part.rsplit('@').next().unwrap_or(&host_part);
    let host = host_port
        .split(':')
        .next()
        .unwrap_or(host_port)
        .to_ascii_lowercase();
    let path = path.trim_matches('/');
    let path = path
        .strip_suffix(".git")
        .unwrap_or(path)
        .trim_end_matches('/');
    if host.is_empty() || path.is_empty() {
        return Err(ProjectError::InvalidRemote);
    }
    Ok(format!("{host}/{path}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn all_variants_normalize_the_same() {
        let want = "github.com/Owner/Repo";
        for u in [
            "git@github.com:Owner/Repo.git",
            "https://github.com/Owner/Repo.git",
            "https://github.com/Owner/Repo",
            "https://github.com/Owner/Repo/",
            "https://user:token@GitHub.com/Owner/Repo.git",
            "ssh://git@github.com/Owner/Repo.git",
            "ssh://git@github.com:22/Owner/Repo.git",
            "git://github.com/Owner/Repo.git",
            "http://github.com:8080/Owner/Repo",
        ] {
            assert_eq!(normalize_remote(u).as_deref(), Ok(want), "{u}");
        }
    }

    #[test]
    fn nested_paths_are_kept() {
        assert_eq!(
            normalize_remote("git@gitlab.com:group/sub/repo.git").as_deref(),
            Ok("gitlab.com/group/sub/repo")
        );
    }

    #[test]
    fn rejects_garbage() {
        for u in [
            "",
            "repo",
            "https://github.com",
            "https://github.com/",
            "/local/path:x",
        ] {
            assert_eq!(normalize_remote(u), Err(ProjectError::InvalidRemote), "{u}");
        }
    }

    #[test]
    fn same_repo_different_url_forms_are_one_project() {
        let a =
            ProjectId::new(Some("git@github.com:o/r.git"), ROOT).unwrap_or_else(|_| unreachable!());
        let b = ProjectId::new(Some("https://github.com/o/r"), &ROOT.to_uppercase())
            .unwrap_or_else(|_| unreachable!());
        assert_eq!(a, b);
        assert_eq!(a.to_string(), format!("github.com/o/r#{ROOT}"));
    }

    #[test]
    fn no_remote_uses_root_commit_only() {
        let p = ProjectId::new(None, ROOT).unwrap_or_else(|_| unreachable!());
        assert_eq!(p.remote(), None);
        assert_eq!(p.to_string(), format!("#{ROOT}"));
    }

    #[test]
    fn rejects_bad_root_commit() {
        assert_eq!(
            ProjectId::new(None, "abc"),
            Err(ProjectError::InvalidRootCommit)
        );
        assert_eq!(
            ProjectId::new(None, &"g".repeat(40)),
            Err(ProjectError::InvalidRootCommit)
        );
    }
}
