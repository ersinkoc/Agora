//! ADR-0065 in the terminal client: the card that asks the owner to approve a new machine or a browser, and the screen a joining
//! machine shows while it waits. Layout only, from a plain description of the request: the protocol that carries it comes with the
//! pairing schema, and the words are the web's (`pair-*` there, `tui-pair-*` here) so both clients say the same thing.
//!
//! The number is the short string both sides derive from the handshake; the person compares it on both screens before approving.
//! Everything shown comes from the other side, so it is made plain (no control characters) and cut to the panel.

use agora_i18n::{key, Arg};

use crate::render::{plain, wrap_row, Text};

/// A one-line value the other side sent: plain text (no control character), a line break made a space.
fn field(s: &str) -> String {
    plain(s)
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_string()
}

/// Who asks to be let in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Asker {
    /// A machine that wants to join this hub: its name, its system when it says it, and the address it asked from.
    Machine {
        name: String,
        os: Option<String>,
        address: String,
    },
    /// A browser that wants to sign in without a code: its name ("Chrome · macOS") and address.
    Browser { name: String, address: String },
}

/// A request waiting for the owner's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairCard {
    pub asker: Asker,
    /// The comparison number, as both sides show it (`482-193`).
    pub number: String,
}

/// The card's lines for a panel `width` columns wide: who asks, the number set apart, what to compare, and the keys.
pub fn card_lines(card: &PairCard, width: usize, t: Text<'_>) -> Vec<String> {
    let width = width.max(16);
    let title = match &card.asker {
        Asker::Machine {
            name,
            os: Some(os),
            address,
        } => t(
            key!("tui-pair-machine-title"),
            &[
                ("name", Arg::Str(field(name))),
                ("os", Arg::Str(field(os))),
                ("address", Arg::Str(field(address))),
            ],
        ),
        Asker::Machine {
            name,
            os: None,
            address,
        } => t(
            key!("tui-pair-machine-title-plain"),
            &[
                ("name", Arg::Str(field(name))),
                ("address", Arg::Str(field(address))),
            ],
        ),
        Asker::Browser { name, address } => t(
            key!("tui-pair-browser-title"),
            &[
                ("name", Arg::Str(field(name))),
                ("address", Arg::Str(field(address))),
            ],
        ),
    };
    let compare = match card.asker {
        Asker::Machine { .. } => t(key!("tui-pair-machine-compare"), &[]),
        Asker::Browser { .. } => t(key!("tui-pair-browser-compare"), &[]),
    };
    let mut lines = wrap_row(&title, width, width);
    lines.push(String::new());
    lines.push(format!(
        "  {}:  {}",
        t(key!("tui-pair-number"), &[]),
        field(&card.number)
    ));
    lines.push(String::new());
    lines.extend(wrap_row(&compare, width, width));
    lines.push(String::new());
    lines.extend(wrap_row(&t(key!("tui-pair-keys"), &[]), width, width));
    lines
}

/// Where a joining machine is (`agora join`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Joining {
    /// Looking for hubs on the network; the ones found so far (name, address).
    Looking(Vec<(String, String)>),
    /// Asked `hub` to let it in; the number to compare with the one the hub's owner sees.
    Waiting {
        hub: String,
        number: String,
    },
    Approved {
        hub: String,
    },
    Denied {
        hub: String,
    },
    /// The owner did not answer in time.
    Expired {
        hub: String,
    },
}

/// The join screen's lines for `width` columns.
pub fn join_lines(state: &Joining, width: usize, t: Text<'_>) -> Vec<String> {
    let width = width.max(16);
    let hub = |h: &str| Arg::Str(field(h));
    let mut lines = Vec::new();
    match state {
        Joining::Looking(found) if found.is_empty() => {
            lines.extend(wrap_row(&t(key!("tui-join-looking"), &[]), width, width));
        }
        Joining::Looking(found) => {
            lines.extend(wrap_row(&t(key!("tui-join-found"), &[]), width, width));
            lines.push(String::new());
            for (i, (name, address)) in found.iter().take(9).enumerate() {
                lines.extend(wrap_row(
                    &format!(" {}  {}  ({})", i + 1, field(name), field(address)),
                    width,
                    width,
                ));
            }
            lines.push(String::new());
            lines.extend(wrap_row(&t(key!("tui-join-pick"), &[]), width, width));
        }
        Joining::Waiting { hub: h, number } => {
            lines.extend(wrap_row(
                &t(key!("tui-join-waiting"), &[("hub", hub(h))]),
                width,
                width,
            ));
            lines.push(String::new());
            lines.push(format!(
                "  {}:  {}",
                t(key!("tui-pair-number"), &[]),
                field(number)
            ));
            lines.push(String::new());
            lines.extend(wrap_row(&t(key!("tui-join-compare"), &[]), width, width));
        }
        Joining::Approved { hub: h } => lines.extend(wrap_row(
            &t(key!("tui-join-approved"), &[("hub", hub(h))]),
            width,
            width,
        )),
        Joining::Denied { hub: h } => lines.extend(wrap_row(
            &t(key!("tui-join-denied"), &[("hub", hub(h))]),
            width,
            width,
        )),
        Joining::Expired { hub: h } => lines.extend(wrap_row(
            &t(key!("tui-join-expired"), &[("hub", hub(h))]),
            width,
            width,
        )),
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn english(id: &str, args: &[(&str, Arg)]) -> String {
        agora_i18n::catalog::Catalog::english()
            .render(id, args)
            .unwrap_or_else(|| format!("<missing {id}>"))
    }

    fn turkish(id: &str, args: &[(&str, Arg)]) -> String {
        agora_i18n::catalog::Catalog::for_locale("tr")
            .render(id, args)
            .unwrap_or_else(|| format!("<missing {id}>"))
    }

    fn width_ok(lines: &[String], width: usize) {
        for l in lines {
            assert!(l.chars().count() <= width, "{l:?} is wider than {width}");
            assert!(!l.contains("<missing"), "{l}");
        }
    }

    #[test]
    fn a_machine_card_names_it_sets_the_number_apart_and_says_what_to_compare() {
        let card = PairCard {
            asker: Asker::Machine {
                name: "laptop".into(),
                os: Some("Windows".into()),
                address: "192.168.1.50".into(),
            },
            number: "482-193".into(),
        };
        let lines = card_lines(&card, 60, &english);
        let all = lines.join("\n");
        assert!(
            all.contains("New machine \"laptop\" (Windows, 192.168.1.50) wants to join"),
            "{all}"
        );
        assert!(
            lines.iter().any(|l| l.trim() == "Number:  482-193"),
            "{all}"
        );
        assert!(all.contains("same number"), "{all}");
        assert!(all.contains("y approves"), "{all}");
        width_ok(&lines, 60);
        width_ok(&card_lines(&card, 24, &english), 24);
        width_ok(&card_lines(&card, 60, &turkish), 60);
    }

    #[test]
    fn what_the_other_side_sent_cannot_paint_or_move_the_screen() {
        let card = PairCard {
            asker: Asker::Browser {
                name: "Chrome\x1b[2J · macOS".into(),
                address: "10.0.0.2\r\n".into(),
            },
            number: "1\x1b]0;x\x07-2".into(),
        };
        for l in card_lines(&card, 40, &english) {
            assert!(!l.chars().any(char::is_control), "{l:?}");
        }
    }

    #[test]
    fn the_join_screen_follows_the_request_from_looking_to_an_answer() {
        let w = 50;
        let looking = join_lines(&Joining::Looking(Vec::new()), w, &english);
        assert!(looking.join(" ").contains("Looking for"), "{looking:?}");
        let found = join_lines(
            &Joining::Looking(vec![("blackview".into(), "192.168.1.246".into())]),
            w,
            &english,
        );
        assert!(
            found
                .iter()
                .any(|l| l.contains("1  blackview  (192.168.1.246)")),
            "{found:?}"
        );
        let waiting = join_lines(
            &Joining::Waiting {
                hub: "blackview".into(),
                number: "482-193".into(),
            },
            w,
            &english,
        );
        assert!(
            waiting.iter().any(|l| l.trim() == "Number:  482-193"),
            "{waiting:?}"
        );
        for state in [
            Joining::Approved { hub: "h".into() },
            Joining::Denied { hub: "h".into() },
            Joining::Expired { hub: "h".into() },
        ] {
            let l = join_lines(&state, w, &english);
            width_ok(&l, w);
            width_ok(&join_lines(&state, w, &turkish), w);
        }
        width_ok(&found, w);
        width_ok(&waiting, w);
    }
}
