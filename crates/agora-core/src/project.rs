//! Project identity: optional portable UUID (ADR-0052), Git discovery (ADR-0003), or node-local folder.

use std::{fmt, net::Ipv6Addr};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectError {
    /// Root commit is not 40 (SHA-1) or 64 (SHA-256) hex digits.
    InvalidRootCommit,
    /// The remote URL could not be understood.
    InvalidRemote,
}

/// Logical project identity; Git discovery and portable UUIDs are independent of machine.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProjectId {
    remote: Option<String>,
    root_commit: String,
    explicit: Option<String>,
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
            explicit: None,
        })
    }

    /// A portable logical project id, independent of a Git repository's detected identity.
    pub fn portable(id: &str) -> Result<Self, ProjectError> {
        let valid = id.len() == 36
            && id.bytes().enumerate().all(|(i, b)| {
                if [8, 13, 18, 23].contains(&i) {
                    b == b'-'
                } else {
                    b.is_ascii_hexdigit()
                }
            });
        if !valid {
            return Err(ProjectError::InvalidRemote);
        }
        Ok(Self {
            remote: None,
            root_commit: String::new(),
            explicit: Some(format!("id:{}", id.to_ascii_lowercase())),
        })
    }

    /// A node-local folder identity. This has no Git root commit or inferred remote.
    pub fn directory(node: &str, path: &str) -> Self {
        Self {
            remote: None,
            root_commit: String::new(),
            explicit: Some(format!("folder:{node}:{path}")),
        }
    }

    pub fn remote(&self) -> Option<&str> {
        self.remote.as_deref()
    }

    /// Empty for a logical project or a non-Git folder; these do not have a Git root commit.
    pub fn root_commit(&self) -> &str {
        &self.root_commit
    }
}

impl fmt::Display for ProjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(id) = &self.explicit {
            return f.write_str(id);
        }
        match &self.remote {
            Some(r) => write!(f, "{r}#{}", self.root_commit),
            None => write!(f, "#{}", self.root_commit),
        }
    }
}

/// Normalize a git remote to `host/owner/repo`: scheme, user info, port, a trailing `.git`, and trailing
/// slashes are dropped; the host is lowercased. The path keeps its case, since a self-hosted server may tell
/// `Owner/Repo` from `owner/repo`, except on the hosts in [`CASE_INSENSITIVE_HOSTS`], which do not: there the
/// path is lowercased and a trailing `.git` is dropped whatever its case.
/// Handles `https://`, `http://`, `ssh://`, `git://`, and scp-like `user@host:path` forms.
pub fn normalize_remote(url: &str) -> Result<String, ProjectError> {
    let url = url.trim();
    let (host_part, path) = if let Some((scheme, rest)) = url.split_once("://") {
        let scheme = scheme.to_ascii_lowercase();
        if !matches!(scheme.as_str(), "https" | "http" | "ssh" | "git") {
            return Err(ProjectError::InvalidRemote);
        }
        // A URI fragment is client-side metadata, never part of the repository path sent to the Git server.
        let rest = rest.split_once('#').map_or(rest, |(remote, _)| remote);
        let (authority, path) = rest.split_once('/').ok_or(ProjectError::InvalidRemote)?;
        (authority.to_string(), path.to_string())
    } else if let Some((prefix, suffix)) = url.split_once(':') {
        if prefix.len() == 1
            && prefix.as_bytes()[0].is_ascii_alphabetic()
            && suffix.starts_with(['/', '\\'])
        {
            return Err(ProjectError::InvalidRemote);
        }
        // scp-like: [user@]host:path, with the separator after any bracketed host.
        let (authority, path) = if prefix
            .rsplit('@')
            .next()
            .is_some_and(|h| h.starts_with('['))
        {
            let end = url.find(']').ok_or(ProjectError::InvalidRemote)? + 1;
            let (authority, path) = url.split_at(end);
            (
                authority,
                path.strip_prefix(':').ok_or(ProjectError::InvalidRemote)?,
            )
        } else {
            (prefix, suffix)
        };
        if authority.contains('/') {
            return Err(ProjectError::InvalidRemote);
        }
        (authority.to_string(), path.to_string())
    } else {
        return Err(ProjectError::InvalidRemote);
    };
    if host_part
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '\\' | '?' | '#'))
    {
        return Err(ProjectError::InvalidRemote);
    }
    let valid_port = |port: &str| {
        !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) && port.parse::<u16>().is_ok()
    };
    let host_port = host_part.rsplit('@').next().unwrap_or(&host_part);
    let host = if host_port.starts_with('[') {
        let end = host_port.find(']').ok_or(ProjectError::InvalidRemote)? + 1;
        let (host, port) = host_port.split_at(end);
        let address = host[1..host.len() - 1]
            .parse::<Ipv6Addr>()
            .map_err(|_| ProjectError::InvalidRemote)?;
        if !port.is_empty() && port.strip_prefix(':').is_none_or(|port| !valid_port(port)) {
            return Err(ProjectError::InvalidRemote);
        }
        format!("[{address}]")
    } else {
        let host = match host_port.split_once(':') {
            Some((host, port)) if valid_port(port) && !host.contains(':') => host,
            Some(_) => return Err(ProjectError::InvalidRemote),
            None => host_port,
        };
        host.to_ascii_lowercase()
    };
    let path = path.trim_matches('/');
    let stripped = strip_git_suffix(&host, path);
    if stripped.is_empty() || stripped.ends_with('/') {
        return Err(ProjectError::InvalidRemote);
    }
    let path = stripped.trim_end_matches('/');
    if host.is_empty()
        || host.chars().any(|c| c.is_whitespace() || c.is_control())
        || path.is_empty()
        || path.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(ProjectError::InvalidRemote);
    }
    let path = fold_path(&host, path);
    Ok(format!("{host}/{path}"))
}

/// ADR-0115: Git hosts that treat owner and repository names without regard to case, so a repository renamed only in case
/// answers at either spelling. Exact host names only: a subdomain (a self-hosted `gitlab.example.com`) keeps its case. It is data
/// here, never a node setting, since two nodes with different lists would split one project again; adding a host is one line and a
/// test.
pub const CASE_INSENSITIVE_HOSTS: &[&str] =
    &["github.com", "gitlab.com", "bitbucket.org", "codeberg.org"];

/// Drop one trailing `.git`: case-insensitively on [`CASE_INSENSITIVE_HOSTS`] hosts, exactly as before elsewhere. This runs
/// once, before [`fold_path`], so a repository literally named `repo.git` keeps that name after its `.git` suffix is removed.
fn strip_git_suffix<'a>(host: &str, path: &'a str) -> &'a str {
    if !CASE_INSENSITIVE_HOSTS.contains(&host) {
        return path.strip_suffix(".git").unwrap_or(path);
    }
    let Some(start) = path.len().checked_sub(4) else {
        return path;
    };
    path.get(start..)
        .filter(|suffix| suffix.eq_ignore_ascii_case(".git"))
        .map_or(path, |_| &path[..start])
}

/// ADR-0115: a remote's path as its project id spells it: lowercased on a [`CASE_INSENSITIVE_HOSTS`] host (`host` already
/// lowercase), unchanged on any other.
fn fold_path<'a>(host: &str, path: &'a str) -> std::borrow::Cow<'a, str> {
    if CASE_INSENSITIVE_HOSTS.contains(&host) && path.bytes().any(|b| b.is_ascii_uppercase()) {
        std::borrow::Cow::Owned(path.to_ascii_lowercase())
    } else {
        std::borrow::Cow::Borrowed(path)
    }
}

/// ADR-0115: a project id as text (`host/path#root`, as [`ProjectId`] prints it) with its path folded as [`normalize_remote`] now
/// folds it, so an id an older node made (`github.com/Owner/Repo#…`) names the same project as a newer node's. Any other id (a
/// portable `id:…`, a `folder:…`, a `#root` without a remote, another host) comes back unchanged and borrowed.
pub fn fold_project_id(id: &str) -> std::borrow::Cow<'_, str> {
    let Some((remote, root)) = id.rsplit_once('#') else {
        return std::borrow::Cow::Borrowed(id);
    };
    let Some((host, path)) = remote.split_once('/') else {
        return std::borrow::Cow::Borrowed(id);
    };
    let folded = fold_path(host, path);
    let normalized = if CASE_INSENSITIVE_HOSTS.contains(&host) {
        strip_git_suffix(host, &folded)
    } else {
        &folded
    };
    if normalized == path && matches!(&folded, std::borrow::Cow::Borrowed(_)) {
        std::borrow::Cow::Borrowed(id)
    } else {
        std::borrow::Cow::Owned(format!("{host}/{normalized}#{root}"))
    }
}

/// ADR-0115: whether two project ids name one project once both are folded ([`fold_project_id`]).
pub fn same_project(a: &str, b: &str) -> bool {
    a == b || fold_project_id(a) == fold_project_id(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn all_variants_normalize_the_same() {
        // ADR-0115: GitHub's paths are folded too; the scheme, user, port and `.git` were always dropped.
        let want = "github.com/owner/repo";
        for u in [
            "git@github.com:Owner/Repo.git",
            "https://github.com/Owner/Repo.git",
            "https://github.com/Owner/Repo",
            "https://github.com/Owner/Repo/",
            "https://user:token@GitHub.com/Owner/Repo.git",
            "https://github.com/Owner/Repo.GIT",
            "git@github.com:Owner/Repo.GIT",
            "ssh://git@github.com/Owner/Repo.git",
            "ssh://git@github.com:22/Owner/Repo.git",
            "git://github.com/Owner/Repo.git",
            "http://github.com:8080/Owner/Repo",
        ] {
            assert_eq!(normalize_remote(u).as_deref(), Ok(want), "{u}");
        }
    }

    #[test]
    fn only_the_outer_git_suffix_is_dropped() {
        assert_eq!(
            normalize_remote("https://github.com/Owner/Repo.git.git").as_deref(),
            Ok("github.com/owner/repo.git")
        );
        assert_eq!(
            normalize_remote("https://git.example.com/Owner/Repo.git.git").as_deref(),
            Ok("git.example.com/Owner/Repo.git")
        );
    }

    #[test]
    fn nested_paths_are_kept() {
        assert_eq!(
            normalize_remote("git@gitlab.com:group/sub/repo.git").as_deref(),
            Ok("gitlab.com/group/sub/repo")
        );
    }

    #[test]
    fn a_case_insensitive_hosts_path_is_one_project_whatever_its_case() {
        // ADR-0115: the field report's two clones, and every remote form of them.
        for (a, b) in [
            ("https://github.com/O/R", "git@github.com:o/r.git"),
            ("https://github.com/O/R", "https://github.com/o/r"),
            (
                "ssh://git@GitHub.com/Ersinkoc/Agora.git",
                "https://github.com/ersinkoc/agora",
            ),
            (
                "https://gitlab.com/Group/Sub/Repo.git",
                "git@gitlab.com:group/sub/repo.git",
            ),
            (
                "https://bitbucket.org/Team/Repo",
                "git@bitbucket.org:team/repo.git",
            ),
            (
                "https://codeberg.org/Forge/Repo",
                "ssh://git@codeberg.org/forge/repo.git",
            ),
            (
                "https://github.com/Owner/Repo.GIT",
                "https://github.com/owner/repo",
            ),
            (
                "git@gitlab.com:Group/Sub/Repo.GIT",
                "git@gitlab.com:group/sub/repo.git",
            ),
        ] {
            assert_eq!(normalize_remote(a), normalize_remote(b), "{a} vs {b}");
            let (pa, pb) = (
                ProjectId::new(Some(a), ROOT).unwrap_or_else(|_| unreachable!()),
                ProjectId::new(Some(b), ROOT).unwrap_or_else(|_| unreachable!()),
            );
            assert_eq!(pa, pb, "{a} vs {b}");
            assert_eq!(pa.to_string(), pa.to_string().to_ascii_lowercase());
        }
        assert_eq!(
            normalize_remote("git@gitlab.com:Group/Sub/Repo.git").as_deref(),
            Ok("gitlab.com/group/sub/repo")
        );
    }

    #[test]
    fn any_other_host_keeps_the_case_of_its_path() {
        // A self-hosted server may tell `O/R` from `o/r`: two projects, as before ADR-0115. A subdomain of a listed host is
        // another host.
        for (a, b) in [
            ("ssh://git@myhost/O/R", "ssh://git@myhost/o/r"),
            (
                "https://gitlab.example.com/O/R",
                "https://gitlab.example.com/o/r",
            ),
            ("https://gist.github.com/O/R", "https://gist.github.com/o/r"),
            (
                "git@github.example.org:O/R.git",
                "git@github.example.org:o/r.git",
            ),
            (
                "https://gist.github.com/O/R.GIT",
                "https://gist.github.com/o/r.git",
            ),
            (
                "https://git.example.com/O/R.GIT",
                "https://git.example.com/o/r",
            ),
        ] {
            assert_ne!(normalize_remote(a), normalize_remote(b), "{a} vs {b}");
        }
        assert_eq!(
            normalize_remote("ssh://git@MyHost/O/R").as_deref(),
            Ok("myhost/O/R")
        );
    }

    #[test]
    fn an_older_nodes_id_folds_to_the_newer_nodes_id() {
        let new = ProjectId::new(Some("https://github.com/ersinkoc/agora"), ROOT)
            .unwrap_or_else(|_| unreachable!())
            .to_string();
        let old = format!("github.com/ersinkoc/Agora#{ROOT}");
        assert_eq!(fold_project_id(&old), new);
        assert!(same_project(&old, &new));
        let old_upper_suffix = format!("github.com/ersinkoc/Agora.GIT#{ROOT}");
        assert_eq!(fold_project_id(&old_upper_suffix), new);
        assert!(same_project(&old_upper_suffix, &new));
        assert!(matches!(
            fold_project_id(&new),
            std::borrow::Cow::Borrowed(_)
        ));
        for kept in [
            format!("myhost/O/R#{ROOT}"),
            format!("gist.github.com/O/R#{ROOT}"),
            format!("#{ROOT}"),
            "id:0f0e0d0c-0b0a-4908-8706-050403020100".to_string(),
            "folder:mac:/Users/Me/Repo".to_string(),
            "folder:mac:/Users/Me/github.com/X#Y".to_string(),
            "github.com".to_string(),
            String::new(),
        ] {
            assert!(
                matches!(fold_project_id(&kept), std::borrow::Cow::Borrowed(_)),
                "{kept}"
            );
        }
        assert!(!same_project(
            &format!("myhost/O/R#{ROOT}"),
            &format!("myhost/o/r#{ROOT}")
        ));
        // Another repository, or the same name with another root commit, is still another project.
        assert!(!same_project(
            &old,
            &format!("github.com/ersinkoc/Agora2#{ROOT}")
        ));
        assert!(!same_project(
            &old,
            &format!("github.com/ersinkoc/agora#{}", "f".repeat(40))
        ));
    }

    #[test]
    fn the_host_list_is_exactly_the_four_hosts() {
        assert_eq!(
            CASE_INSENSITIVE_HOSTS,
            ["github.com", "gitlab.com", "bitbucket.org", "codeberg.org"]
        );
    }

    #[test]
    fn rejects_garbage() {
        for u in [
            "",
            "repo",
            "https://github.com",
            "https://github.com/",
            "https://github.com/Owner/.GIT",
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

    #[test]
    fn f416_http_url_fragments_do_not_change_project_identity() {
        // github.com folds the path's case (ADR-0115); a fragment never reaches the project id.
        assert_eq!(
            normalize_remote("https://github.com/Owner/Repo.git#readme").as_deref(),
            Ok("github.com/owner/repo")
        );
        assert_eq!(
            normalize_remote("https://myhost/Owner/Repo.git#readme").as_deref(),
            Ok("myhost/Owner/Repo")
        );
    }

    #[test]
    fn verifies_f416_remote_fragment_boundaries() {
        for (remote, expected) in [
            ("https://github.com/o/r.git#readme", "github.com/o/r"),
            ("ssh://git@github.com/o/r#", "github.com/o/r"),
            (
                "git://github.com/o/r?download=1#top",
                "github.com/o/r?download=1",
            ),
            ("git@github.com:o/r#literal", "github.com/o/r#literal"),
        ] {
            assert_eq!(
                normalize_remote(remote).as_deref(),
                Ok(expected),
                "{remote}"
            );
        }
        println!("FIX VERIFIED");
    }
}
