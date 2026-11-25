use agora_core::{normalize_remote, ProjectError, ProjectId};

const ROOT: &str = "0123456789abcdef0123456789abcdef01234567";

#[test]
fn dns_and_ipv4_controls_keep_existing_normalization() {
    for (remote, expected) in [
        (
            "https://git.example:8443/Owner/Repo.git",
            "git.example/Owner/Repo",
        ),
        ("git@git.example:Owner/Repo.git", "git.example/Owner/Repo"),
        (
            "ssh://git@192.0.2.1:22/Owner/Repo.git",
            "192.0.2.1/Owner/Repo",
        ),
        (
            "git@git.example:Owner/Repo:archive.git",
            "git.example/Owner/Repo:archive",
        ),
        (
            "git@git.example:Owner/repo@[tag]:archive.git",
            "git.example/Owner/repo@[tag]:archive",
        ),
    ] {
        assert_eq!(
            normalize_remote(remote).as_deref(),
            Ok(expected),
            "{remote}"
        );
    }
}

#[test]
fn ipv6_url_remotes_keep_the_entire_host() {
    for remote in [
        "https://[2001:DB8::1]/Owner/Repo.git",
        "http://[2001:db8::1]:8080/Owner/Repo/",
        "ssh://git@[2001:db8::1]:22/Owner/Repo.git",
        "git://[2001:db8::1]/Owner/Repo",
    ] {
        assert_eq!(
            normalize_remote(remote).as_deref(),
            Ok("[2001:db8::1]/Owner/Repo"),
            "{remote}"
        );
    }
    assert_eq!(
        normalize_remote("https://[::1]/r.git").as_deref(),
        Ok("[::1]/r")
    );
    assert_eq!(
        normalize_remote("ssh://git@[::ffff:192.0.2.1]:22/r.git").as_deref(),
        Ok("[::ffff:192.0.2.1]/r")
    );
}

#[test]
fn ipv6_scp_remotes_split_after_the_closing_bracket() {
    for remote in [
        "git@[2001:DB8::1]:Owner/Repo.git",
        "[2001:db8::1]:Owner/Repo.git",
    ] {
        assert_eq!(
            normalize_remote(remote).as_deref(),
            Ok("[2001:db8::1]/Owner/Repo"),
            "{remote}"
        );
    }
    assert_eq!(
        normalize_remote("git@[::1]:Owner/Repo:archive.git").as_deref(),
        Ok("[::1]/Owner/Repo:archive")
    );
}

#[test]
fn different_ipv6_hosts_do_not_merge_projects_with_the_same_root() -> Result<(), ProjectError> {
    let a = ProjectId::new(Some("ssh://git@[2001:db8::1]/Owner/Repo.git"), ROOT)?;
    let b = ProjectId::new(Some("ssh://git@[2001:db8::2]/Owner/Repo.git"), ROOT)?;
    assert_ne!(
        a, b,
        "different remote hosts must identify different projects"
    );
    assert_ne!(a.to_string(), b.to_string());
    let same = ProjectId::new(Some("git@[2001:DB8::1]:Owner/Repo.git"), ROOT)?;
    assert_eq!(a, same, "URL and SCP forms of the same remote must agree");
    Ok(())
}

#[test]
fn unclosed_ipv6_brackets_are_invalid_remotes() {
    for remote in [
        "https://[2001:db8::1/Owner/Repo.git",
        "git@[2001:db8::1:Owner/Repo.git",
    ] {
        assert_eq!(
            normalize_remote(remote),
            Err(ProjectError::InvalidRemote),
            "{remote}"
        );
    }
}
