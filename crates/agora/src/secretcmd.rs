//! `agora node secret set|list|remove` (ADR-0084): the owner's secrets on this machine, for the agents the node policy lists them
//! for. The value comes from standard input or a no-echo prompt, never from an argument; it is never printed, never logged, and
//! never sent anywhere: the command talks to files in the node's private folder and nothing else (not the node, not the hub).

use std::io::{IsTerminal, Read};
use std::path::PathBuf;
use std::process::ExitCode;

use agora_i18n::key;
use agora_node::credentials::{CredentialError, Secret, SecretStore, MAX_VALUE};

use crate::{home, Opts, Say};

fn data_dir(o: &Opts) -> PathBuf {
    o.data.clone().unwrap_or_else(|| home().join("data"))
}

/// What a person typed or piped: one value, at most [`MAX_VALUE`] bytes (a longer one is refused, not cut).
fn read_value(say: &Say, name: &str) -> Result<String, ExitCode> {
    if std::io::stdin().is_terminal() {
        // The prompt appears only once echo is off: an answer typed or pasted right after it is never shown.
        let prompt = say.text(key!("cli-secret-prompt"), &[("name", name.into())]);
        return agora_platform::read_secret_line_with_prompt(&prompt).map_err(|e| {
            if e.kind() == std::io::ErrorKind::Interrupted {
                ExitCode::from(130)
            } else {
                say.err(key!("cli-secret-failed"), &[]);
                ExitCode::from(1)
            }
        });
    }
    let mut text = String::new();
    let limit = u64::try_from(MAX_VALUE + 4).unwrap_or(u64::MAX);
    if std::io::stdin()
        .lock()
        .take(limit)
        .read_to_string(&mut text)
        .is_err()
    {
        say.err(key!("cli-secret-bad-value"), &[]);
        return Err(ExitCode::from(1));
    }
    Ok(text)
}

fn failed(say: &Say, e: &CredentialError) -> ExitCode {
    // The code names what went wrong; no value is anywhere near an error.
    match e {
        CredentialError::NameInvalid => say.err(key!("cli-secret-bad-name"), &[]),
        CredentialError::ValueInvalid => say.err(key!("cli-secret-bad-value"), &[]),
        CredentialError::Io(_) => say.err(key!("cli-secret-failed"), &[]),
    }
    ExitCode::from(1)
}

pub fn secret(say: &Say, o: &Opts) -> ExitCode {
    let store = SecretStore::in_data(&data_dir(o));
    let args: Vec<&str> = o.positional.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["list"] => {
            let names = store.list();
            if names.is_empty() {
                say.out(key!("cli-secret-none"), &[]);
            }
            for n in names {
                say.out(key!("cli-secret-name"), &[("name", n.into())]);
            }
            ExitCode::SUCCESS
        }
        ["set", name] => {
            if !agora_node::credentials::name_ok(name) {
                return failed(say, &CredentialError::NameInvalid);
            }
            let raw = match read_value(say, name) {
                Ok(v) => v,
                Err(code) => return code,
            };
            let result = Secret::new(&raw).and_then(|v| store.set(name, &v));
            match result {
                Ok(()) => {
                    say.out(key!("cli-secret-set"), &[("name", (*name).into())]);
                    ExitCode::SUCCESS
                }
                Err(e) => failed(say, &e),
            }
        }
        ["remove", name] => match store.remove(name) {
            Ok(true) => {
                say.out(key!("cli-secret-removed"), &[("name", (*name).into())]);
                ExitCode::SUCCESS
            }
            Ok(false) => {
                say.err(key!("cli-secret-not-found"), &[("name", (*name).into())]);
                ExitCode::from(1)
            }
            Err(e) => failed(say, &e),
        },
        _ => {
            say.err(key!("cli-bad-arguments"), &[]);
            say.err(key!("cli-usage"), &[]);
            ExitCode::from(2)
        }
    }
}
