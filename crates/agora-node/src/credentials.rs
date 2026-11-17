//! ADR-0084: the login a confined agent gets from a secret the node holds. The owner stores a secret on this machine
//! (`agora node secret set NAME`), the node policy says which kind may receive which names (`credentials.<kind> = NAME, ...`), and the
//! node puts exactly those into the environment of an agent of that kind, under the names the kind's manifest declares in
//! `[accounts] credential_env`. Every declared name of every other kind is taken out of the environment the node inherited.
//!
//! The store is a folder of owner-only files (`<data>/secrets/<NAME>`), made with `agora_platform::ensure_private_dir` and
//! `write_private`. A value is never logged, never in an event, never in a command line, never sent to the hub, and never printed:
//! [`Secret`] has no `Display` and prints `<redacted>` for `Debug`. Nothing here reads a Keychain, a credential store, or another
//! program's files (decision 6). No agent kind appears in this file: the names come from manifests.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

pub use agora_adapters::manifest::credential_name_ok as name_ok;

/// The store's folder inside the node's data folder.
pub const SECRETS_DIR: &str = "secrets";
/// Longest value accepted, in bytes (a token is a few hundred).
pub const MAX_VALUE: usize = 8192;
/// Most names one `credentials.<kind>` line may list.
pub const MAX_LISTED: usize = agora_adapters::manifest::MAX_CREDENTIAL_ENV;

/// A secret's value. It prints as `<redacted>`, has no `Display`, and is read only through [`Secret::expose`].
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

impl Secret {
    /// A value as the owner gave it: one line, not empty, no control characters, at most [`MAX_VALUE`] bytes. One trailing line
    /// break (what a pipe adds) is dropped; nothing else is trimmed.
    pub fn new(raw: &str) -> Result<Self, CredentialError> {
        let v = raw
            .strip_suffix("\r\n")
            .or_else(|| raw.strip_suffix('\n'))
            .unwrap_or(raw);
        if v.is_empty() || v.len() > MAX_VALUE || v.chars().any(char::is_control) {
            return Err(CredentialError::ValueInvalid);
        }
        Ok(Self(v.to_string()))
    }

    /// The value, for the one place that puts it into an agent's environment.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

/// Why a store operation failed. Each has a stable token (`code`) the CLI maps to its own text; none carries a value.
#[derive(Debug)]
pub enum CredentialError {
    NameInvalid,
    /// Another stored name differs from this one only in letters' case (they would be one file on some systems).
    NameCollides,
    ValueInvalid,
    Io(io::Error),
}

impl CredentialError {
    /// Stable code: `credential.name_invalid`, `credential.name_collides`, `credential.value_invalid`, `credential.store_unavailable`.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NameInvalid => "credential.name_invalid",
            Self::NameCollides => "credential.name_collides",
            Self::ValueInvalid => "credential.value_invalid",
            Self::Io(_) => "credential.store_unavailable",
        }
    }
}

impl std::fmt::Display for CredentialError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

/// The folder of secrets. Holds a path only, so printing it can never show a value.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretStore {
    dir: PathBuf,
}

impl std::fmt::Debug for SecretStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretStore")
            .field("dir", &self.dir)
            .field("values", &"<redacted>")
            .finish()
    }
}

impl SecretStore {
    /// The store of the node whose data folder is `data_dir`.
    pub fn in_data(data_dir: &Path) -> Self {
        Self {
            dir: data_dir.join(SECRETS_DIR),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Stores (or replaces) `name`. The file is owner-only from its creation.
    pub fn set(&self, name: &str, value: &Secret) -> Result<(), CredentialError> {
        if !name_ok(name) {
            return Err(CredentialError::NameInvalid);
        }
        if self
            .list()
            .iter()
            .any(|n| n != name && n.eq_ignore_ascii_case(name))
        {
            return Err(CredentialError::NameCollides);
        }
        agora_platform::ensure_private_dir(&self.dir).map_err(CredentialError::Io)?;
        agora_platform::write_private(&self.dir.join(name), value.expose().as_bytes())
            .map_err(CredentialError::Io)
    }

    /// The names of the stored secrets, sorted. Never a value.
    pub fn list(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| name_ok(n))
            .collect();
        names.sort();
        names
    }

    /// Whether `name` is stored (no value is read).
    pub fn has(&self, name: &str) -> bool {
        name_ok(name)
            && std::fs::symlink_metadata(self.dir.join(name)).is_ok_and(|m| m.file_type().is_file())
    }

    /// Removes `name`; whether there was one.
    pub fn remove(&self, name: &str) -> Result<bool, CredentialError> {
        if !name_ok(name) {
            return Err(CredentialError::NameInvalid);
        }
        match std::fs::remove_file(self.dir.join(name)) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(CredentialError::Io(e)),
        }
    }

    /// The value of `name`, when stored and well-formed. A file that is a link, or holds anything [`Secret::new`] refuses, is no secret.
    pub fn get(&self, name: &str) -> Option<Secret> {
        if !self.has(name) {
            return None;
        }
        let text = std::fs::read_to_string(self.dir.join(name)).ok()?;
        Secret::new(&text).ok()
    }
}

/// `credentials.<kind> = NAME, NAME`: plain names only, at most [`MAX_LISTED`]. One entry that is not a plain name, a duplicate, or too
/// many entries gives `None` (the line passes nothing), as `sandbox.pass_env` does.
pub fn parse_listed(v: &str) -> Option<Vec<String>> {
    let names: Vec<String> = v
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    let mut seen = std::collections::HashSet::new();
    let ok = !names.is_empty()
        && names.len() <= MAX_LISTED
        && names
            .iter()
            .all(|n| name_ok(n) && seen.insert(n.to_ascii_uppercase()));
    ok.then_some(names)
}

/// What the node holds for starting agents: where the secrets are, and the policy's `credentials.<kind>` lines.
#[derive(Debug, Clone, Default)]
pub struct Credentials {
    store: Option<SecretStore>,
    /// kind -> names the owner lets it receive.
    listed: BTreeMap<String, Vec<String>>,
}

/// What one start does to the environment: names to leave out of the inherited environment, and values to set.
pub struct Plan {
    pub scrub: Vec<String>,
    pub set: Vec<(String, Secret)>,
}

impl Credentials {
    pub fn new(store: SecretStore, listed: BTreeMap<String, Vec<String>>) -> Self {
        Self {
            store: Some(store),
            listed,
        }
    }

    /// The plan for an agent of `kind`. `declared` is every kind's `credential_env` (from the manifests the node has loaded).
    /// - `set`: each name the policy lists for `kind` that `kind` declares, valued from the store, else from the node's own
    ///   environment (`env`); a name with no value anywhere is left out.
    /// - `scrub`: every declared name of any kind that this kind is not listed to receive (decision 5).
    pub fn plan(
        &self,
        kind: &str,
        declared: &BTreeMap<String, Vec<String>>,
        env: impl Fn(&str) -> Option<String>,
    ) -> Plan {
        let own = declared.get(kind).map_or(&[][..], Vec::as_slice);
        let allowed: Vec<&String> = self
            .listed
            .get(kind)
            .into_iter()
            .flatten()
            .filter(|n| own.contains(n))
            .collect();
        let set: Vec<(String, Secret)> = allowed
            .iter()
            .filter_map(|n| {
                let value = self
                    .store
                    .as_ref()
                    .and_then(|s| s.get(n))
                    .or_else(|| env(n).and_then(|v| Secret::new(&v).ok()))?;
                Some(((*n).clone(), value))
            })
            .collect();
        let mut scrub: Vec<String> = declared
            .values()
            .flatten()
            .filter(|n| !allowed.contains(n))
            .cloned()
            .collect();
        scrub.sort();
        scrub.dedup();
        Plan { scrub, set }
    }

    /// The names the policy lists for `kind` that its manifest does not declare (refused: named by `agora doctor`).
    pub fn undeclared(&self, kind: &str, own: &[String]) -> Vec<String> {
        self.listed
            .get(kind)
            .into_iter()
            .flatten()
            .filter(|n| !own.contains(n))
            .cloned()
            .collect()
    }

    pub fn listed(&self, kind: &str) -> &[String] {
        self.listed.get(kind).map_or(&[], Vec::as_slice)
    }

    pub fn store(&self) -> Option<&SecretStore> {
        self.store.as_ref()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("agora-cred-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn declared() -> BTreeMap<String, Vec<String>> {
        [
            ("a".to_string(), vec!["A_TOKEN".to_string()]),
            (
                "b".to_string(),
                vec!["B_TOKEN".to_string(), "B_KEY".to_string()],
            ),
        ]
        .into_iter()
        .collect()
    }

    #[test]
    fn a_value_is_one_clean_line() {
        assert_eq!(Secret::new("abc\n").unwrap().expose(), "abc");
        assert_eq!(Secret::new("abc\r\n").unwrap().expose(), "abc");
        assert_eq!(Secret::new(" a b ").unwrap().expose(), " a b ");
        for bad in ["", "\n", "a\nb", "a\0b", "a\tb", &"x".repeat(MAX_VALUE + 1)] {
            assert!(Secret::new(bad).is_err());
        }
    }

    #[test]
    fn a_secret_never_prints() {
        let s = Secret::new("sk-very-secret-123").unwrap();
        assert_eq!(format!("{s:?}"), "<redacted>");
        let d = tmp("debug");
        let store = SecretStore::in_data(&d);
        store.set("A_TOKEN", &s).unwrap();
        let shown = format!(
            "{store:?} {:?}",
            Credentials::new(store.clone(), BTreeMap::new())
        );
        assert!(!shown.contains("sk-very-secret"), "{shown}");
        let err = format!(
            "{:?} {}",
            CredentialError::ValueInvalid,
            CredentialError::NameInvalid
        );
        assert!(!err.contains("sk-"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_store_round_trips_and_lists_names_only() {
        let d = tmp("store");
        let store = SecretStore::in_data(&d);
        assert!(store.list().is_empty());
        store.set("B_KEY", &Secret::new("one").unwrap()).unwrap();
        store.set("A_TOKEN", &Secret::new("two").unwrap()).unwrap();
        store
            .set("A_TOKEN", &Secret::new("three").unwrap())
            .unwrap();
        assert_eq!(store.list(), ["A_TOKEN", "B_KEY"]);
        assert_eq!(store.get("A_TOKEN").unwrap().expose(), "three");
        assert!(store.has("B_KEY") && !store.has("C"));
        assert!(store.get("C").is_none());
        // A stray file that is not a plain name is not listed.
        std::fs::write(store.dir().join("A.tmp-1"), "x").unwrap();
        std::fs::write(store.dir().join("not a name"), "x").unwrap();
        assert_eq!(store.list(), ["A_TOKEN", "B_KEY"]);
        assert!(matches!(
            store.set("PATH", &Secret::new("x").unwrap()),
            Err(CredentialError::NameInvalid)
        ));
        assert!(matches!(
            store.set("../x", &Secret::new("x").unwrap()),
            Err(CredentialError::NameInvalid)
        ));
        // Windows device names and names that differ only in case are refused.
        for bad in ["CON", "nul", "COM1", "lpt9", "Aux"] {
            assert!(matches!(
                store.set(bad, &Secret::new("x").unwrap()),
                Err(CredentialError::NameInvalid)
            ));
        }
        assert!(store.set("COM0", &Secret::new("x").unwrap()).is_ok());
        assert!(store.remove("COM0").unwrap());
        assert!(matches!(
            store.set("a_token", &Secret::new("x").unwrap()),
            Err(CredentialError::NameCollides)
        ));
        assert_eq!(store.list(), ["A_TOKEN", "B_KEY"]);
        assert!(store.remove("A_TOKEN").unwrap());
        assert!(!store.remove("A_TOKEN").unwrap());
        assert_eq!(store.list(), ["B_KEY"]);
        assert!(matches!(
            store.remove("a/b"),
            Err(CredentialError::NameInvalid)
        ));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&store.dir().join("B_KEY")), 0o600);
            assert_eq!(mode(store.dir()), 0o700);
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_is_no_secret() {
        let d = tmp("link");
        let store = SecretStore::in_data(&d);
        store.set("A_TOKEN", &Secret::new("x").unwrap()).unwrap();
        std::fs::write(d.join("elsewhere"), "stolen").unwrap();
        std::os::unix::fs::symlink(d.join("elsewhere"), store.dir().join("B_KEY")).unwrap();
        assert!(store.get("B_KEY").is_none());
        assert_eq!(store.list(), ["A_TOKEN"]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn policy_lines_parse_to_plain_names_or_nothing() {
        assert_eq!(
            parse_listed("A_TOKEN, B_KEY"),
            Some(vec!["A_TOKEN".into(), "B_KEY".into()])
        );
        assert_eq!(parse_listed("A_TOKEN,"), Some(vec!["A_TOKEN".into()]));
        let many = (0..=MAX_LISTED)
            .map(|i| format!("N{i}"))
            .collect::<Vec<_>>()
            .join(",");
        for bad in [
            "", " ", "A=1", "A*", "A B", "1A", "A, $B", "PATH", "A,a", "A, B=c", &many,
        ] {
            assert_eq!(parse_listed(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_plan_gives_a_kind_its_own_listed_names_and_scrubs_the_rest() {
        let d = tmp("plan");
        let store = SecretStore::in_data(&d);
        store
            .set("A_TOKEN", &Secret::new("stored-a").unwrap())
            .unwrap();
        store
            .set("B_TOKEN", &Secret::new("stored-b").unwrap())
            .unwrap();
        let listed = [("a".to_string(), vec!["A_TOKEN".to_string()])]
            .into_iter()
            .collect();
        let c = Credentials::new(store, listed);
        let env = |n: &str| (n == "B_KEY").then(|| "from-env".to_string());
        let a = c.plan("a", &declared(), env);
        assert_eq!(a.set.len(), 1);
        assert_eq!(
            (a.set[0].0.as_str(), a.set[0].1.expose()),
            ("A_TOKEN", "stored-a")
        );
        // A's own name is kept in the inherited environment; the other kind's names are not.
        assert_eq!(a.scrub, ["B_KEY", "B_TOKEN"]);
        // Kind b has no line: it gets nothing, and everything declared is scrubbed (stored B_TOKEN is not passed).
        let b = c.plan("b", &declared(), env);
        assert!(b.set.is_empty());
        assert_eq!(b.scrub, ["A_TOKEN", "B_KEY", "B_TOKEN"]);
        // A kind nobody declared for (a shell) gets nothing and loses all.
        let s = c.plan("shell", &declared(), env);
        assert!(s.set.is_empty());
        assert_eq!(s.scrub.len(), 3);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_nodes_environment_is_the_fallback_and_a_stored_secret_wins() {
        let d = tmp("fallback");
        let store = SecretStore::in_data(&d);
        let listed = [(
            "b".to_string(),
            vec!["B_TOKEN".to_string(), "B_KEY".to_string()],
        )]
        .into_iter()
        .collect();
        let c = Credentials::new(store.clone(), listed);
        let env = |n: &str| Some(format!("env-{n}"));
        let p = c.plan("b", &declared(), env);
        assert_eq!(p.set.len(), 2);
        assert_eq!(p.set[0].1.expose(), "env-B_TOKEN");
        store
            .set("B_TOKEN", &Secret::new("stored").unwrap())
            .unwrap();
        let p = c.plan("b", &declared(), env);
        assert_eq!(p.set[0].1.expose(), "stored");
        assert_eq!(p.set[1].1.expose(), "env-B_KEY");
        // No value anywhere: nothing set.
        assert!(c.plan("b", &declared(), |_| None).set.len() == 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_listed_name_the_kind_does_not_declare_is_refused() {
        let listed = [(
            "a".to_string(),
            vec!["A_TOKEN".to_string(), "B_TOKEN".to_string()],
        )]
        .into_iter()
        .collect();
        let c = Credentials::new(
            SecretStore::in_data(Path::new("/nonexistent-agora")),
            listed,
        );
        assert_eq!(c.undeclared("a", &["A_TOKEN".to_string()]), ["B_TOKEN"]);
        let p = c.plan("a", &declared(), |n| Some(format!("env-{n}")));
        assert_eq!(p.set.len(), 1);
        assert_eq!(p.set[0].0, "A_TOKEN");
        assert!(p.scrub.contains(&"B_TOKEN".to_string()));
    }
}
