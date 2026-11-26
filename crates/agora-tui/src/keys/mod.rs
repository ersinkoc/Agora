//! ADR-0068 decision 6 in the terminal client: keys and commands are data. Every action a key does in the shell is a command with a
//! stable id ([`commands`]); the person's keybindings file ([`file`], the web window's format, `docs/ux/keybindings.md`) changes their
//! keys; [`map`] says what one key press does with the keys in force. Pure, except [`file::load`], which reads one bounded file.

pub mod commands;
pub mod file;
pub mod map;
pub mod strokes;

pub use map::{Keymap, Press};

use agora_i18n::{key, Arg};

use crate::render::Text;

/// What the keys in force leave out, in words, one line each: the file's keys the terminal client keeps to itself, and the commands it
/// does not have (kept in the file for the web window). Empty when the file is used whole.
pub fn notes(map: &Keymap, t: Text<'_>) -> Vec<String> {
    let mut out: Vec<String> = map
        .skipped
        .iter()
        .map(|s| {
            t(
                match s.reason {
                    commands::KeptReason::Answers => key!("tui-keys-kept-answers"),
                    commands::KeptReason::Moving => key!("tui-keys-kept-moving"),
                },
                &[
                    ("command", Arg::Str(s.command.clone())),
                    ("key", Arg::Str(s.key.clone())),
                ],
            )
        })
        .collect();
    if !map.unknown.is_empty() {
        out.push(t(
            key!("tui-keys-unknown"),
            &[("commands", Arg::Str(map.unknown.join(", ")))],
        ));
    }
    out
}
