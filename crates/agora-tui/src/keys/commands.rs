//! The terminal client's commands (ADR-0068 decision 6): every action a key does in the shell, by a stable id, with where it acts and
//! its default keys. An id the web window also has means the same action there (`palette.open`, `attention.next`, `agent.start`,
//! `pane.tab-1`, `layout.preset-2x2`, ...), so one keybindings file serves both; an id only one client has binds nothing in the other.
//! Ids are never renamed or reused. The defaults are the keys the shell had before the file existed.

use crate::layout::engine::Preset;
use crate::layout::tree::Direction;

/// Where a command's keys act.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context {
    /// Only while the agent list has the keyboard.
    List,
    /// While the list or the details have it.
    Window,
    /// Also inside an agent's tab, for a key with Alt (every other key typed there is the agent's).
    Anywhere,
}

/// What a command does; the shell ([`crate::app::App`]) runs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Palette,
    NextNeedsYou,
    /// The tab (or, in a tiled layout, the pane group's tab) at this place, from 0.
    Tab(usize),
    Preset(Preset),
    Maximize,
    /// The pane after (true) or before.
    StepPane(bool),
    Split(Direction),
    ClosePane,
    SwapNext,
    MoveToNext,
    TakeOut,
    Grow(Direction, bool),
    SaveLayoutAs,
    ResetLayout,
    Mouse,
    Inspector,
    CloseTab,
    CleanUpOld,
    Quit,
    /// One of the caller's letters for the selected agent (start, stop, review, ...): `Action::Command`.
    Letter(char),
}

/// One command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandDef {
    pub id: String,
    pub context: Context,
    pub act: Act,
    /// The keys it has when the file changes nothing.
    pub keys: Vec<&'static str>,
}

fn def(id: &str, context: Context, act: Act, keys: &[&'static str]) -> CommandDef {
    CommandDef {
        id: id.to_string(),
        context,
        act,
        keys: keys.to_vec(),
    }
}

/// The caller's letters as commands: id, letter, default key. Answering a request (y, n, A) is not among them: no command answers.
const LETTERS: [(&str, char, &str); 14] = [
    ("agent.start", 's', "s"),
    ("agent.start-in-folder", 'f', "f"),
    ("agent.stop", 'x', "x"),
    ("agent.resume", 'u', "u"),
    ("agent.start-again", 'U', "shift+u"),
    ("agent.review", 'r', "r"),
    ("agent.files", 'o', "o"),
    ("board.open", 'b', "b"),
    ("agent.remove", 'd', "d"),
    ("device.ask", 'a', "a"),
    ("devices.approve", 'v', "v"),
    ("typing.grant", 'g', "g"),
    ("machines.pair", 'p', "p"),
    ("theme.choose", 't', "t"),
];

/// Every command of the terminal client, in the order the keybindings page lists them.
pub fn commands() -> Vec<CommandDef> {
    use Context::{Anywhere, List, Window};
    let mut all = vec![
        def(
            "palette.open",
            Anywhere,
            Act::Palette,
            &["ctrl+k", "alt+k", "alt+shift+k"],
        ),
        def("attention.next", List, Act::NextNeedsYou, &["shift+n"]),
    ];
    const TABS: [&str; 9] = [
        "alt+1", "alt+2", "alt+3", "alt+4", "alt+5", "alt+6", "alt+7", "alt+8", "alt+9",
    ];
    for (i, key) in TABS.iter().enumerate() {
        all.push(def(
            &format!("pane.tab-{}", i + 1),
            Anywhere,
            Act::Tab(i),
            &[key],
        ));
    }
    for (name, preset, key) in [
        ("1x1", Preset::One, "1"),
        ("2x1", Preset::TwoByOne, "2"),
        ("1x2", Preset::OneByTwo, "3"),
        ("2x2", Preset::TwoByTwo, "4"),
        ("focus", Preset::Focus, "5"),
        ("3x2", Preset::ThreeByTwo, "6"),
    ] {
        all.push(def(
            &format!("layout.preset-{name}"),
            Window,
            Act::Preset(preset),
            &[key],
        ));
    }
    all.extend([
        def("layout.maximize", Window, Act::Maximize, &["z"]),
        def("layout.pane-previous", Window, Act::StepPane(false), &["h"]),
        def("layout.pane-next", Window, Act::StepPane(true), &["l"]),
        def(
            "layout.split-right",
            Window,
            Act::Split(Direction::Row),
            &[],
        ),
        def(
            "layout.split-down",
            Window,
            Act::Split(Direction::Column),
            &[],
        ),
        def("layout.close-pane", Window, Act::ClosePane, &[]),
        def("layout.swap-next", Window, Act::SwapNext, &[]),
        def("layout.move-next", Window, Act::MoveToNext, &[]),
        def("layout.take-out", Window, Act::TakeOut, &[]),
        def("layout.wider", Window, Act::Grow(Direction::Row, true), &[]),
        def(
            "layout.narrower",
            Window,
            Act::Grow(Direction::Row, false),
            &[],
        ),
        def(
            "layout.taller",
            Window,
            Act::Grow(Direction::Column, true),
            &[],
        ),
        def(
            "layout.shorter",
            Window,
            Act::Grow(Direction::Column, false),
            &[],
        ),
        def("layout.save-as", Window, Act::SaveLayoutAs, &[]),
        def("layout.reset", Window, Act::ResetLayout, &[]),
        def(
            "window.mouse",
            Anywhere,
            Act::Mouse,
            &["m", "alt+m", "alt+shift+m"],
        ),
        def("window.details", Window, Act::Inspector, &["i"]),
        def("tab.close", List, Act::CloseTab, &["w"]),
        def("machines.clean-up", Window, Act::CleanUpOld, &[]),
        def("app.quit", Window, Act::Quit, &["q"]),
    ]);
    for (id, letter, key) in LETTERS {
        all.push(def(id, Window, Act::Letter(letter), &[key]));
    }
    all
}

/// Why a key is fixed, as `docs/ux/keybindings.md` names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FixedReason {
    SafeMode,
    Navigation,
    Terminal,
    Editing,
}

/// The web window's fixed keys (`keys/commands.ts` `FIXED_KEYS`): no file may bind them, in either client, so a file is valid or not
/// the same way in both.
pub const FIXED_KEYS: [(&str, FixedReason); 30] = [
    ("ctrl+alt+shift+s", FixedReason::SafeMode),
    ("alt+shift+meta+s", FixedReason::SafeMode),
    ("tab", FixedReason::Navigation),
    ("shift+tab", FixedReason::Navigation),
    ("escape", FixedReason::Navigation),
    ("enter", FixedReason::Navigation),
    ("space", FixedReason::Navigation),
    ("up", FixedReason::Navigation),
    ("down", FixedReason::Navigation),
    ("left", FixedReason::Navigation),
    ("right", FixedReason::Navigation),
    ("ctrl+shift+c", FixedReason::Terminal),
    ("ctrl+insert", FixedReason::Terminal),
    ("ctrl+shift+f", FixedReason::Terminal),
    ("ctrl+shift+v", FixedReason::Terminal),
    ("shift+insert", FixedReason::Terminal),
    ("meta+c", FixedReason::Terminal),
    ("meta+f", FixedReason::Terminal),
    ("meta+v", FixedReason::Terminal),
    ("ctrl+a", FixedReason::Editing),
    ("ctrl+c", FixedReason::Editing),
    ("ctrl+v", FixedReason::Editing),
    ("ctrl+x", FixedReason::Editing),
    ("ctrl+z", FixedReason::Editing),
    ("ctrl+y", FixedReason::Editing),
    ("ctrl+shift+z", FixedReason::Editing),
    ("meta+a", FixedReason::Editing),
    ("meta+x", FixedReason::Editing),
    ("meta+z", FixedReason::Editing),
    ("shift+meta+z", FixedReason::Editing),
];

/// The web's one fixed command (`window.safe-mode`): a file that names it is refused in both clients.
pub const FIXED_COMMANDS: [&str; 1] = ["window.safe-mode"];

/// Why the terminal client keeps a key to itself although a file may bind it for the web window: such a binding is left out here (and
/// said), never used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeptReason {
    /// y, n, A and Alt+Y, Alt+N, Alt+A (with Shift too) answer a request (T44: no key in a file may stand in for them).
    Answers,
    /// Ctrl+] and F12 leave an agent's tab; j and k, Home and End move in the list.
    Moving,
}

/// The keys the terminal client keeps (see [`KeptReason`]).
pub const KEPT_KEYS: [(&str, KeptReason); 15] = [
    ("y", KeptReason::Answers),
    ("n", KeptReason::Answers),
    ("shift+a", KeptReason::Answers),
    ("alt+y", KeptReason::Answers),
    ("alt+n", KeptReason::Answers),
    ("alt+a", KeptReason::Answers),
    ("alt+shift+y", KeptReason::Answers),
    ("alt+shift+n", KeptReason::Answers),
    ("alt+shift+a", KeptReason::Answers),
    ("ctrl+]", KeptReason::Moving),
    ("f12", KeptReason::Moving),
    ("j", KeptReason::Moving),
    ("k", KeptReason::Moving),
    ("home", KeptReason::Moving),
    ("end", KeptReason::Moving),
];

/// A command id: dotted lower-case words (`layout.preset-2x2`), at most 64 characters (the web's `ID`).
pub fn is_command_id(id: &str) -> bool {
    if id.is_empty() || id.len() > 64 {
        return false;
    }
    id.split('.').enumerate().all(|(i, word)| {
        let mut c = word.chars();
        let first_ok = match c.next() {
            Some(f) if i == 0 => f.is_ascii_lowercase(),
            Some(f) => f.is_ascii_lowercase() || f.is_ascii_digit(),
            None => false,
        };
        first_ok && c.all(|x| x.is_ascii_lowercase() || x.is_ascii_digit() || x == '-')
    })
}

/// A command no file may name: answering a request (the web's `NEVER_COMMAND`, T44).
pub fn answers_a_request(id: &str) -> bool {
    ["request", "approval", "approve", "deny", "allow"]
        .iter()
        .any(|w| id == *w || id.strip_prefix(w).is_some_and(|rest| rest.starts_with('.')))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::keys::strokes::parse_binding;

    #[test]
    fn every_command_has_a_good_id_once_and_keys_that_parse_and_are_not_fixed() {
        let all = commands();
        let mut ids: Vec<&str> = all.iter().map(|c| c.id.as_str()).collect();
        ids.sort_unstable();
        let n = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), n, "an id twice");
        for c in &all {
            assert!(is_command_id(&c.id), "{}", c.id);
            assert!(!answers_a_request(&c.id), "{}", c.id);
            for k in &c.keys {
                let b = parse_binding(k).unwrap_or_else(|| panic!("{k}"));
                let text = crate::keys::strokes::binding_text(&b);
                assert!(FIXED_KEYS.iter().all(|(f, _)| *f != text), "{k} is fixed");
                assert!(KEPT_KEYS.iter().all(|(f, _)| *f != text), "{k} is kept");
            }
        }
        // The fixed and kept lists are written in their one spelling.
        let fixed = FIXED_KEYS.iter().map(|(k, _)| *k);
        for k in fixed.chain(KEPT_KEYS.iter().map(|(k, _)| *k)) {
            assert_eq!(
                crate::keys::strokes::binding_text(&parse_binding(k).unwrap()),
                k
            );
        }
    }

    #[test]
    fn ids_and_answers_are_read_as_the_web_reads_them() {
        assert!(is_command_id("layout.preset-2x2"));
        assert!(is_command_id("pane.tab-1"));
        assert!(!is_command_id("Layout.x"));
        assert!(!is_command_id("1layout"));
        assert!(!is_command_id("a..b"));
        assert!(is_command_id("a.b-c"));
        assert!(answers_a_request("allow"));
        assert!(answers_a_request("request.allow"));
        assert!(answers_a_request("approval.deny-all"));
        assert!(!answers_a_request("allowance"));
        assert!(!answers_a_request("agent.allow"));
    }
}
