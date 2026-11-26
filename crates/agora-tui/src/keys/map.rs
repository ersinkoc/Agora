//! The keys in force (`docs/ux/keybindings.md`, "Where a key acts"; the web's `keymap` and `registry.ts`), and what one key press does
//! with them. Pure: the time a chord waits is passed in.

use std::time::{Duration, Instant};

use super::commands::{commands, Act, CommandDef, Context, KeptReason, KEPT_KEYS};
use super::file::KeyFile;
use super::strokes::{binding_text, parse_binding, Binding, Stroke};

/// How long a chord waits for its second key.
pub const CHORD_WAIT: Duration = Duration::from_millis(1500);

/// One key in force.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bound {
    pub command: String,
    pub binding: Binding,
    /// From the person's file, not a default.
    pub own: bool,
}

/// A key of the file the terminal client keeps to itself ([`KEPT_KEYS`]): left out here, and said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    pub command: String,
    pub key: String,
    pub reason: KeptReason,
}

/// The keys in force: the person's own first (in the file's order), then the defaults of the commands the file does not name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keymap {
    pub bound: Vec<Bound>,
    /// Every default key, by command: a default the file took away does nothing (it is not passed on as a letter or to the agent).
    defaults: Vec<Bound>,
    /// The commands of the file this client does not have (a web window's), kept and binding nothing.
    pub unknown: Vec<String>,
    pub skipped: Vec<Skipped>,
    defs: Vec<CommandDef>,
}

impl Default for Keymap {
    fn default() -> Self {
        Self::new(&KeyFile::default())
    }
}

/// What one key press does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Press {
    /// Run this command.
    Run(Act),
    /// The first key of a chord: wait for the second (nothing else happens).
    Wait,
    /// A default key the file took from its command: nothing happens.
    Taken,
    /// Not a command's key: the shell does what it did before (moving in the list, typing into the agent, the caller's letters).
    Pass,
}

fn kept(binding: &[Stroke]) -> Option<KeptReason> {
    binding.iter().find_map(|s| {
        let text = super::strokes::stroke_text(s);
        KEPT_KEYS.iter().find(|(k, _)| *k == text).map(|(_, r)| *r)
    })
}

impl Keymap {
    /// The keys in force for a file that was read ([`super::file::read`]); [`KeyFile::default`] gives the defaults.
    pub fn new(file: &KeyFile) -> Self {
        let defs = commands();
        let mut bound = Vec::new();
        let mut named: Vec<&str> = Vec::new();
        let mut unknown = Vec::new();
        let mut skipped = Vec::new();
        for e in &file.entries {
            if !defs.iter().any(|d| d.id == e.command) {
                unknown.push(e.command.clone());
                continue;
            }
            named.push(&e.command);
            for text in &e.keys {
                let Some(binding) = parse_binding(text) else {
                    continue;
                };
                if let Some(reason) = kept(&binding) {
                    skipped.push(Skipped {
                        command: e.command.clone(),
                        key: binding_text(&binding),
                        reason,
                    });
                    continue;
                }
                bound.push(Bound {
                    command: e.command.clone(),
                    binding,
                    own: true,
                });
            }
        }
        let mut defaults = Vec::new();
        for d in &defs {
            for text in &d.keys {
                if let Some(binding) = parse_binding(text) {
                    defaults.push(Bound {
                        command: d.id.clone(),
                        binding,
                        own: false,
                    });
                }
            }
        }
        bound.extend(
            defaults
                .iter()
                .filter(|b| !named.contains(&b.command.as_str()))
                .cloned(),
        );
        Self {
            bound,
            defaults,
            unknown,
            skipped,
            defs,
        }
    }

    fn def(&self, id: &str) -> Option<&CommandDef> {
        self.defs.iter().find(|d| d.id == id)
    }

    /// Whether `b` may act where the keyboard is: in the list (`list`), the details, or inside an agent's tab (`tab`), where only a
    /// one-stroke Alt key of a command that acts anywhere does (every other key there is the agent's).
    fn acts(&self, b: &Bound, list: bool, tab: bool) -> bool {
        let Some(d) = self.def(&b.command) else {
            return false;
        };
        if tab {
            let s = &b.binding[0];
            return d.context == Context::Anywhere
                && b.binding.len() == 1
                && s.alt
                && !s.ctrl
                && !s.meta;
        }
        d.context != Context::List || list
    }

    /// The keys a command has now, in order (the palette and the footer show the first).
    pub fn keys_of(&self, command: &str) -> Vec<&Binding> {
        self.bound
            .iter()
            .filter(|b| b.command == command)
            .map(|b| &b.binding)
            .collect()
    }

    /// One key pressed. `list`: the agent list has the keyboard (else the details, or `tab`: an agent's tab). `chord` is the first key
    /// of a chord waiting since its time; it is taken here, and set again when this key starts one.
    pub fn press(
        &self,
        stroke: Option<&Stroke>,
        list: bool,
        tab: bool,
        now: Instant,
        chord: &mut Option<(Stroke, Instant)>,
    ) -> Press {
        let waiting = chord
            .take()
            .filter(|(_, at)| now.saturating_duration_since(*at) <= CHORD_WAIT);
        let Some(stroke) = stroke else {
            return Press::Pass;
        };
        if let Some((first, _)) = waiting {
            if stroke.key == "escape" && !stroke.ctrl && !stroke.alt && !stroke.meta {
                return Press::Taken;
            }
            let found = self.bound.iter().find(|b| {
                b.binding.len() == 2
                    && b.binding[0] == first
                    && b.binding[1] == *stroke
                    && self.acts(b, list, tab)
            });
            if let Some(b) = found {
                return self.run(&b.command);
            }
            // Not the chord's second key: dropped, and this key is taken on its own.
        }
        if !tab
            && self
                .bound
                .iter()
                .any(|b| b.binding.len() == 2 && b.binding[0] == *stroke && self.acts(b, list, tab))
        {
            *chord = Some((stroke.clone(), now));
            return Press::Wait;
        }
        if let Some(b) = self
            .bound
            .iter()
            .find(|b| b.binding.len() == 1 && b.binding[0] == *stroke && self.acts(b, list, tab))
        {
            return self.run(&b.command);
        }
        // A default key whose command the file moved elsewhere: it no longer does what it did. Inside a tab it is the agent's again.
        if !tab
            && self.defaults.iter().any(|b| {
                b.binding.len() == 1
                    && b.binding[0] == *stroke
                    && self.acts(b, list, tab)
                    && !self.bound.contains(b)
            })
        {
            return Press::Taken;
        }
        Press::Pass
    }

    fn run(&self, id: &str) -> Press {
        self.def(id).map_or(Press::Pass, |d| Press::Run(d.act))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::app::Key;
    use crate::keys::file::read;
    use crate::keys::strokes::stroke_of;
    use crate::layout::engine::Preset;

    fn press(
        map: &Keymap,
        k: Key,
        list: bool,
        tab: bool,
        now: Instant,
        chord: &mut Option<(Stroke, Instant)>,
    ) -> Press {
        map.press(stroke_of(&k).as_ref(), list, tab, now, chord)
    }

    #[test]
    fn the_defaults_are_the_keys_of_before() {
        let map = Keymap::default();
        let now = Instant::now();
        let mut chord = None;
        let mut p = |k: Key, list: bool, tab: bool| press(&map, k, list, tab, now, &mut chord);
        assert_eq!(p(Key::Ctrl('k'), true, false), Press::Run(Act::Palette));
        assert_eq!(p(Key::Alt('k'), false, true), Press::Run(Act::Palette));
        // Inside a tab Ctrl+K and plain letters are the agent's.
        assert_eq!(p(Key::Ctrl('k'), false, true), Press::Pass);
        assert_eq!(p(Key::Char('m'), false, true), Press::Pass);
        assert_eq!(p(Key::Alt('m'), false, true), Press::Run(Act::Mouse));
        // With Shift (Caps Lock) too, as before.
        assert_eq!(p(Key::Alt('K'), false, true), Press::Run(Act::Palette));
        assert_eq!(p(Key::Alt('M'), false, true), Press::Run(Act::Mouse));
        assert_eq!(p(Key::Alt('3'), false, true), Press::Run(Act::Tab(2)));
        assert_eq!(
            p(Key::Char('4'), true, false),
            Press::Run(Act::Preset(Preset::TwoByTwo))
        );
        assert_eq!(
            p(Key::Char('N'), true, false),
            Press::Run(Act::NextNeedsYou)
        );
        // The next agent that needs you and closing a tab act from the list only, as before.
        assert_eq!(p(Key::Char('N'), false, false), Press::Pass);
        assert_eq!(p(Key::Char('w'), false, false), Press::Pass);
        assert_eq!(
            p(Key::Char('s'), false, false),
            Press::Run(Act::Letter('s'))
        );
        assert_eq!(p(Key::Char('U'), true, false), Press::Run(Act::Letter('U')));
        // Answers and moving are not commands.
        for k in [
            Key::Char('y'),
            Key::Char('n'),
            Key::Char('A'),
            Key::Char('j'),
            Key::Enter,
            Key::Tab,
        ] {
            assert_eq!(p(k, true, false), Press::Pass);
        }
        assert!(map.unknown.is_empty() && map.skipped.is_empty());
    }

    #[test]
    fn the_file_moves_keys_and_a_default_it_took_does_nothing() {
        let file = read(
            r#"{"version":1,"bindings":[
                {"command":"agent.start","keys":["ctrl+s"]},
                {"command":"layout.maximize","keys":[]},
                {"command":"go.settings","keys":["ctrl+,"]},
                {"command":"attention.next","keys":["n","ctrl+n"]}
            ]}"#,
        )
        .unwrap();
        let map = Keymap::new(&file);
        let now = Instant::now();
        let mut chord = None;
        let mut p = |k: Key| press(&map, k, true, false, now, &mut chord);
        assert_eq!(p(Key::Ctrl('s')), Press::Run(Act::Letter('s')));
        assert_eq!(p(Key::Char('s')), Press::Taken);
        assert_eq!(p(Key::Char('z')), Press::Taken);
        assert_eq!(p(Key::Char('N')), Press::Taken);
        assert_eq!(p(Key::Ctrl('n')), Press::Run(Act::NextNeedsYou));
        // `n` answers a request here: left out, and said.
        assert_eq!(p(Key::Char('n')), Press::Pass);
        assert_eq!(map.unknown, vec!["go.settings".to_string()]);
        assert_eq!(
            map.skipped,
            vec![Skipped {
                command: "attention.next".into(),
                key: "n".into(),
                reason: KeptReason::Answers
            }]
        );
        assert_eq!(map.keys_of("agent.start").len(), 1);
    }

    #[test]
    fn a_chord_waits_for_its_second_key_within_its_time() {
        let file = read(
            r#"{"version":1,"bindings":[{"command":"layout.preset-2x2","keys":["ctrl+g 2"]}]}"#,
        )
        .unwrap();
        let map = Keymap::new(&file);
        let t0 = Instant::now();
        let mut chord = None;
        assert_eq!(
            press(&map, Key::Ctrl('g'), true, false, t0, &mut chord),
            Press::Wait
        );
        assert_eq!(
            press(
                &map,
                Key::Char('2'),
                true,
                false,
                t0 + Duration::from_millis(500),
                &mut chord
            ),
            Press::Run(Act::Preset(Preset::TwoByTwo))
        );
        // Too late: the second key is taken on its own (2 is the 2x1 preset).
        press(&map, Key::Ctrl('g'), true, false, t0, &mut chord);
        assert_eq!(
            press(
                &map,
                Key::Char('2'),
                true,
                false,
                t0 + Duration::from_secs(2),
                &mut chord
            ),
            Press::Run(Act::Preset(Preset::TwoByOne))
        );
        // Another key drops it and acts itself; Esc drops it and does nothing.
        press(&map, Key::Ctrl('g'), true, false, t0, &mut chord);
        assert_eq!(
            press(&map, Key::Char('z'), true, false, t0, &mut chord),
            Press::Run(Act::Maximize)
        );
        assert!(chord.is_none());
        press(&map, Key::Ctrl('g'), true, false, t0, &mut chord);
        assert_eq!(
            press(&map, Key::Esc, true, false, t0, &mut chord),
            Press::Taken
        );
        // No chord starts inside a tab.
        assert_eq!(
            press(&map, Key::Ctrl('g'), false, true, t0, &mut chord),
            Press::Pass
        );
        assert!(chord.is_none());
    }

    #[test]
    fn inside_a_tab_only_alt_keys_of_commands_that_act_anywhere() {
        let file = read(
            r#"{"version":1,"bindings":[
                {"command":"layout.maximize","keys":["alt+z"]},
                {"command":"palette.open","keys":["ctrl+p","alt+p"]}
            ]}"#,
        )
        .unwrap();
        let map = Keymap::new(&file);
        let now = Instant::now();
        let mut chord = None;
        assert_eq!(
            press(&map, Key::Alt('z'), false, true, now, &mut chord),
            Press::Pass
        );
        assert_eq!(
            press(&map, Key::Alt('z'), true, false, now, &mut chord),
            Press::Run(Act::Maximize)
        );
        assert_eq!(
            press(&map, Key::Ctrl('p'), false, true, now, &mut chord),
            Press::Pass
        );
        assert_eq!(
            press(&map, Key::Alt('p'), false, true, now, &mut chord),
            Press::Run(Act::Palette)
        );
        // Alt+K was the palette's default, which the file replaced: inside a tab it is the agent's again.
        assert_eq!(
            press(&map, Key::Alt('k'), false, true, now, &mut chord),
            Press::Pass
        );
    }
}
