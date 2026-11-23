//! ADR-0065 in the terminal client: the owner's card for a machine (or browser) that asks to be let in, and the list of cards kept in
//! memory for the session. The words are the web's (`pair-*` there, `tui-pair-*` here) so both clients say the same thing.
//!
//! The number is what ties the card to the machine in front of the person: they compare it on both screens and approve only if it is the
//! same. So the card is shown whole (name, system and address in rows of their own, the number in two groups of three) before any key
//! answers it, approving takes two deliberate keys (`a`, then `y`), and nothing here is written to a file or a log. Everything shown comes
//! from the other side, so it is made plain (no control character, no character that reorders text) and cut to the panel.

use agora_i18n::{key, Arg};
use agora_protocol::v1::{PairDecided, PairKind, PairOutcome, PairRequested};

use crate::client::PairNote;
use crate::render::{plain, wrap_row, Text};

/// A one-line value the other side sent: plain text (no control character, no character that changes the direction or hides),
/// a line break made a space.
pub fn field(s: &str) -> String {
    plain(s)
        .chars()
        .filter(|c| {
            !matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}')
        })
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_string()
}

/// `482193` as `482-193`; anything that is not six digits as given.
pub fn grouped(number: &str) -> String {
    if number.len() == 6 && number.bytes().all(|b| b.is_ascii_digit()) {
        format!("{}-{}", &number[..3], &number[3..])
    } else {
        number.to_string()
    }
}

/// One request waiting for the owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    /// The hub's pair id: what an answer names.
    pub pair_id: Vec<u8>,
    pub machine: bool,
    pub name: String,
    pub os: String,
    pub address: String,
    /// Six digits.
    pub number: String,
    /// The hub's clock, milliseconds since the Unix epoch.
    pub expires_ms: u64,
    /// The hub said the name already belongs to an enrolled machine: then, and only then, Replace is offered.
    pub taken: bool,
}

impl Card {
    /// `None` for a notice that is malformed (no id, an unknown kind, a number that is not six digits, no name): it is dropped.
    pub fn from_frame(c: &PairRequested) -> Option<Card> {
        let machine = match PairKind::try_from(c.kind).ok()? {
            PairKind::Machine => true,
            PairKind::Browser => false,
            PairKind::Unspecified => return None,
        };
        let ok = c.number.len() == 6 && c.number.bytes().all(|b| b.is_ascii_digit());
        if c.pair_id.is_empty() || c.pair_id.len() > 32 || !ok || c.name.is_empty() {
            return None;
        }
        Some(Card {
            pair_id: c.pair_id.clone(),
            machine,
            name: c.name.clone(),
            os: c.os.clone(),
            address: c.address.clone(),
            number: c.number.clone(),
            expires_ms: c.expires_ms,
            taken: false,
        })
    }
}

/// How a request ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    Approved,
    Denied,
    Expired,
    Closed,
}

/// Whether this session may see cards. Only the hub knows the role (it answers `PairStatus` to the owner and `auth.forbidden` to anyone
/// else), so cards are shown only once it said yes; a notice that arrives before is held, and dropped if the answer is no.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Owner {
    #[default]
    Unknown,
    Yes,
    No,
}

/// The cards of one hub, in memory only.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Book {
    pub owner: Owner,
    cards: Vec<Card>,
    /// How many cards ended since the owner last looked, with the last one's name and outcome (one line in the footer).
    last_end: Option<(String, Ended)>,
}

/// What the hub's answer to `PairStatus` says of the role (#1086, #1099): the error code it answered, `None` for an answer. An answer is
/// the owner's; `auth.forbidden` is anyone else's, a member's or a viewer's. `protocol.capability_missing` comes from a hub with pairing
/// switched off, which asks for the role first, so only its owner hears it (a hub that predates pairing says it to everyone; it lets no
/// member look into folders either). Any other code tells nothing: `None`, asked again at the next connection.
pub fn status_role(code: Option<&str>) -> Option<(Owner, crate::picker::Role)> {
    use crate::picker::Role;
    match code {
        None => Some((Owner::Yes, Role::Owner)),
        Some("auth.forbidden") => Some((Owner::No, Role::Limited)),
        Some("protocol.capability_missing") => Some((Owner::No, Role::Owner)),
        Some(_) => None,
    }
}

/// The hub shows at most four cards, so does the client.
pub const MAX_CARDS: usize = 4;

impl Book {
    /// The hub's answer to `PairStatus`: yes for the owner, no for anyone else (nothing held is kept).
    pub fn set_owner(&mut self, owner: Owner) {
        self.owner = owner;
        if owner != Owner::Yes {
            self.cards.clear();
            self.last_end = None;
        }
    }

    /// A new connection: what was held is stale until the hub sends the cards waiting now.
    pub fn reset(&mut self) {
        self.cards.clear();
    }

    /// The link was lost. The hub ends a person's connection when their role or grants change, so who the owner is is asked again at
    /// the next connection (#1099), and the cards held are stale, as after [`Self::reset`]. A routine refresh, which the client asks
    /// for itself, keeps the answer ([`Self::reset`]).
    pub fn lost(&mut self) {
        self.owner = Owner::Unknown;
        self.cards.clear();
    }

    /// One notice from the session (see `Session::take_pair_notes`).
    pub fn apply(&mut self, note: PairNote) {
        if self.owner == Owner::No {
            return;
        }
        match note {
            PairNote::Requested(c) => {
                if let Some(card) = Card::from_frame(&c) {
                    let taken = self
                        .cards
                        .iter()
                        .find(|k| k.pair_id == card.pair_id)
                        .is_some_and(|k| k.taken);
                    self.cards.retain(|k| k.pair_id != card.pair_id);
                    self.cards.push(Card { taken, ..card });
                    if self.cards.len() > MAX_CARDS {
                        self.cards.remove(0);
                    }
                }
            }
            PairNote::Decided(d) => self.decided(&d),
        }
    }

    fn decided(&mut self, d: &PairDecided) {
        let ended = match PairOutcome::try_from(d.outcome) {
            Ok(PairOutcome::Approved) => Ended::Approved,
            Ok(PairOutcome::Denied) => Ended::Denied,
            Ok(PairOutcome::Expired) => Ended::Expired,
            Ok(PairOutcome::Closed) => Ended::Closed,
            _ => return,
        };
        if let Some(at) = self.cards.iter().position(|k| k.pair_id == d.pair_id) {
            let card = self.cards.remove(at);
            self.last_end = Some((card.name, ended));
        }
    }

    /// The cards to show: none unless the hub said this session is the owner's.
    pub fn shown(&self) -> &[Card] {
        if self.owner == Owner::Yes {
            &self.cards
        } else {
            &[]
        }
    }

    /// The first card (the one `p` opens).
    pub fn first(&self) -> Option<&Card> {
        self.shown().first()
    }

    /// Remembers the hub's refusal: the name belongs to an enrolled machine, so the card offers Replace.
    pub fn mark_taken(&mut self, pair_id: &[u8]) {
        if let Some(c) = self.cards.iter_mut().find(|c| c.pair_id == pair_id) {
            c.taken = true;
        }
    }

    /// The line for the footer about the last request that ended, taken once.
    pub fn take_end(&mut self) -> Option<(String, Ended)> {
        if self.owner == Owner::Yes {
            self.last_end.take()
        } else {
            None
        }
    }
}

/// Where the owner is on a card: the card itself, or the question before an approval goes out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Card,
    Confirm,
}

/// A key that comes sooner than this after a screen was shown is not an answer to it: a key that was held or typed ahead must not
/// approve a machine (the same rule as `agora pair open`).
pub const MIN_THINK: std::time::Duration = std::time::Duration::from_millis(500);

/// Whether a key `elapsed` after its screen was shown came too soon to be an answer (Esc always counts: leaving is safe).
pub fn too_soon(key: u8, elapsed: std::time::Duration) -> bool {
    key != 0x1b && elapsed < MIN_THINK
}

/// Whether an answer can go out: the link is up and the hub's expiry has not passed.
pub fn can_answer(card: &Card, now_ms: u64, link_up: bool) -> bool {
    link_up && now_ms < card.expires_ms
}

/// What a key did on the card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing to send: the card is shown again (a step changed, or the key meant nothing here).
    Stay,
    /// Esc: the box closes and the request stays pending on the hub.
    Leave,
    Deny,
    Approve {
        replace: bool,
    },
}

/// The owner's steps on one card, as a pure machine over the keys. Approving takes two keys: `a`, then `y`; any other key after `a`
/// goes back, and no single key sends an approval. `r` chooses Replace, and only on a card whose name the hub said is taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dialog {
    pub step: Step,
    pub replace: bool,
}

impl Default for Dialog {
    fn default() -> Self {
        Dialog {
            step: Step::Card,
            replace: false,
        }
    }
}

impl Dialog {
    pub fn key(&mut self, k: u8, taken: bool, can_answer: bool) -> Outcome {
        if k == 0x1b {
            return Outcome::Leave;
        }
        // Offline or run out: the card can be read and left, but no answer goes out.
        if !can_answer && matches!(k.to_ascii_lowercase(), b'd' | b'y') {
            return Outcome::Stay;
        }
        match (self.step, k.to_ascii_lowercase()) {
            (Step::Card, b'a') => {
                self.step = Step::Confirm;
                Outcome::Stay
            }
            (Step::Card, b'r') if taken => {
                self.replace = !self.replace;
                Outcome::Stay
            }
            (Step::Card, b'd') => Outcome::Deny,
            (Step::Confirm, b'y') => Outcome::Approve {
                replace: taken && self.replace,
            },
            (Step::Confirm, _) => {
                self.step = Step::Card;
                Outcome::Stay
            }
            (Step::Card, _) => Outcome::Stay,
        }
    }

    /// The hub kept the request open and said the name belongs to a machine: back to the card, Replace unchosen.
    pub fn name_taken(&mut self) {
        self.step = Step::Card;
        self.replace = false;
    }
}

fn span(seconds: u64) -> String {
    if seconds < 60 {
        format!("{seconds} s")
    } else {
        format!("{} min", seconds / 60)
    }
}

/// The card's lines for a panel `width` columns wide: who asks (name, system and address in rows of their own), the number set apart in
/// two groups, what to compare it with, the time left, and the keys of this step. `link_up` false says the answer cannot be sent now.
#[allow(clippy::too_many_arguments)]
pub fn card_lines(
    card: &Card,
    step: Step,
    replace: bool,
    now_ms: u64,
    link_up: bool,
    width: usize,
    t: Text<'_>,
) -> Vec<String> {
    let width = width.max(16);
    let mut lines = wrap_row(
        &t(
            if card.machine {
                key!("tui-pair-machine-title")
            } else {
                key!("tui-pair-browser-title")
            },
            &[],
        ),
        width,
        width,
    );
    lines.push(String::new());
    let mut row = |label: String, value: &str| {
        lines.extend(wrap_row(
            &format!("  {label}:  {}", field(value)),
            width,
            width,
        ));
    };
    row(t(key!("tui-pair-name"), &[]), &card.name);
    if card.machine && !field(&card.os).is_empty() {
        row(t(key!("tui-pair-system"), &[]), &card.os);
    }
    row(t(key!("tui-pair-address"), &[]), &card.address);
    lines.push(String::new());
    lines.push(format!(
        "  {}:  {}",
        t(key!("tui-pair-number"), &[]),
        grouped(&field(&card.number)).replace('-', "   ")
    ));
    lines.push(String::new());
    lines.extend(wrap_row(
        &t(
            if card.machine {
                key!("tui-pair-machine-compare")
            } else {
                key!("tui-pair-browser-compare")
            },
            &[],
        ),
        width,
        width,
    ));
    let left = card.expires_ms.saturating_sub(now_ms) / 1000;
    lines.push(if left == 0 {
        t(key!("tui-pair-ran-out"), &[])
    } else {
        t(key!("tui-pair-expires"), &[("time", Arg::Str(span(left)))])
    });
    if !link_up {
        lines.extend(wrap_row(&t(key!("tui-pair-offline"), &[]), width, width));
    }
    if card.taken {
        lines.push(String::new());
        lines.extend(wrap_row(
            &t(
                if replace {
                    key!("tui-pair-replace-on")
                } else {
                    key!("tui-pair-replace-off")
                },
                &[("name", Arg::Str(field(&card.name)))],
            ),
            width,
            width,
        ));
    }
    lines.push(String::new());
    let keys = match (step, card.taken) {
        (Step::Confirm, _) => {
            lines.extend(wrap_row(&t(key!("tui-pair-confirm"), &[]), width, width));
            key!("tui-pair-keys-confirm")
        }
        (Step::Card, true) => key!("tui-pair-keys-taken"),
        (Step::Card, false) => key!("tui-pair-keys"),
    };
    lines.extend(wrap_row(&t(keys, &[]), width, width));
    lines
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn say(id: &str, args: &[(&str, Arg)]) -> String {
        agora_i18n::catalog::Catalog::english()
            .render(id, args)
            .unwrap_or_else(|| format!("<missing {id}>"))
    }

    fn turkish(id: &str, args: &[(&str, Arg)]) -> String {
        agora_i18n::catalog::Catalog::for_locale("tr")
            .render(id, args)
            .unwrap_or_else(|| format!("<missing {id}>"))
    }

    fn requested(id: u8, name: &str, number: &str) -> PairRequested {
        PairRequested {
            pair_id: vec![id; 16],
            kind: PairKind::Machine as i32,
            name: name.into(),
            os: "windows".into(),
            address: "192.168.1.50".into(),
            number: number.into(),
            expires_ms: 1_272_000,
        }
    }

    fn owner_book() -> Book {
        let mut b = Book::default();
        b.set_owner(Owner::Yes);
        b
    }

    fn card(name: &str) -> Card {
        Card::from_frame(&requested(1, name, "482193")).unwrap()
    }

    fn fits(lines: &[String], width: usize) {
        for l in lines {
            assert!(l.chars().count() <= width, "{l:?} is wider than {width}");
            assert!(!l.contains("<missing"), "{l}");
        }
    }

    #[test]
    fn the_card_gives_name_system_and_address_a_row_each_and_sets_the_number_apart_in_two_groups() {
        let lines = card_lines(
            &card("laptop"),
            Step::Card,
            false,
            1_000_000,
            true,
            60,
            &say,
        );
        let all = lines.join("\n");
        let flat = lines.join(" ");
        assert!(all.contains("A new machine wants to join"), "{all}");
        assert!(lines.iter().any(|l| l.trim() == "Name:  laptop"), "{all}");
        assert!(
            lines.iter().any(|l| l.trim() == "System:  windows"),
            "{all}"
        );
        assert!(
            lines.iter().any(|l| l.trim() == "Address:  192.168.1.50"),
            "{all}"
        );
        assert!(
            lines.iter().any(|l| l.trim() == "Number:  482   193"),
            "{all}"
        );
        assert!(
            flat.contains(
                "Compare with the number on the new machine. Approve only if they are the same."
            ),
            "{all}"
        );
        assert!(all.contains("Runs out in 4 min"), "{all}");
        // The first step has the keys of the card: no single key approves.
        assert!(flat.contains("a approves"), "{all}");
        assert!(!flat.contains("y approves"), "{all}");
        fits(&lines, 60);
        fits(
            &card_lines(&card("laptop"), Step::Card, false, 0, true, 24, &say),
            24,
        );
        fits(
            &card_lines(&card("laptop"), Step::Card, false, 0, true, 60, &turkish),
            60,
        );
    }

    #[test]
    fn approving_asks_once_more_and_only_y_answers_it() {
        let lines = card_lines(&card("laptop"), Step::Confirm, false, 0, true, 60, &say).join(" ");
        assert!(
            lines.contains("Do the two numbers match exactly?"),
            "{lines}"
        );
        assert!(lines.contains("y: yes, they match"), "{lines}");
        assert!(!lines.contains("a approves"), "{lines}");
    }

    #[test]
    fn replace_is_offered_only_when_the_hub_said_the_name_is_taken() {
        let plain_card =
            card_lines(&card("laptop"), Step::Card, false, 0, true, 60, &say).join("\n");
        assert!(!plain_card.contains("Replace"), "{plain_card}");
        let mut taken = card("laptop");
        taken.taken = true;
        let off = card_lines(&taken, Step::Card, false, 0, true, 60, &say).join(" ");
        assert!(off.contains("Replace the existing machine: no"), "{off}");
        assert!(off.contains("r changes Replace"), "{off}");
        let on = card_lines(&taken, Step::Card, true, 0, true, 60, &say).join(" ");
        assert!(on.contains("Replace the existing machine: yes"), "{on}");
        assert!(on.contains("loses its access"), "{on}");
    }

    #[test]
    fn it_says_when_the_link_is_down_and_when_the_request_has_run_out() {
        let down = card_lines(&card("a"), Step::Card, false, 0, false, 60, &say).join("\n");
        assert!(down.contains("Not connected right now"), "{down}");
        let gone = card_lines(&card("a"), Step::Card, false, 2_000_000, true, 60, &say).join("\n");
        assert!(gone.contains("This request has run out"), "{gone}");
    }

    #[test]
    fn what_the_other_side_sent_cannot_paint_or_move_the_screen_or_reorder_a_row() {
        let mut c = card("laptop");
        c.name = "x\x1b[2J\u{202e}evil\u{200b}\r\n".into();
        c.address = "10.0.0.2\x1b]0;t\x07".into();
        c.os = "win\tdows".into();
        let lines = card_lines(&c, Step::Card, false, 0, true, 40, &say);
        for l in &lines {
            assert!(!l.chars().any(char::is_control), "{l:?}");
            assert!(!l.contains('\u{202e}') && !l.contains('\u{200b}'), "{l:?}");
        }
    }

    #[test]
    fn no_single_key_approves_a_machine() {
        // Every key on the card itself: only `d` sends anything, and it denies.
        for k in 0u8..=255 {
            let mut d = Dialog::default();
            assert!(
                !matches!(d.key(k, false, true), Outcome::Approve { .. }),
                "key {k} approved at once"
            );
        }
        let mut d = Dialog::default();
        assert_eq!(d.key(b'y', false, true), Outcome::Stay);
        assert_eq!(d.key(b'\r', false, true), Outcome::Stay);
        assert_eq!(d.step, Step::Card);
    }

    #[test]
    fn approving_is_a_then_y_and_any_other_key_after_a_goes_back() {
        let mut d = Dialog::default();
        assert_eq!(d.key(b'a', false, true), Outcome::Stay);
        assert_eq!(d.step, Step::Confirm);
        assert_eq!(
            d.key(b'y', false, true),
            Outcome::Approve { replace: false }
        );
        for other in *b"na \rd" {
            let mut d = Dialog::default();
            d.key(b'a', false, true);
            assert_eq!(d.key(other, false, true), Outcome::Stay, "key {other}");
            assert_eq!(d.step, Step::Card);
        }
    }

    #[test]
    fn replace_is_chosen_with_r_only_on_a_taken_name_and_starts_unchosen() {
        let mut d = Dialog::default();
        assert_eq!(d.key(b'r', false, true), Outcome::Stay);
        assert!(!d.replace);
        d.key(b'r', true, true);
        assert!(d.replace);
        d.key(b'a', true, true);
        assert_eq!(d.key(b'y', true, true), Outcome::Approve { replace: true });
        // A name that is not taken never sends replace, whatever was toggled before.
        let mut d = Dialog {
            step: Step::Confirm,
            replace: true,
        };
        assert_eq!(
            d.key(b'y', false, true),
            Outcome::Approve { replace: false }
        );
        // After the hub says the name is taken, the owner starts over from the card.
        let mut d = Dialog {
            step: Step::Confirm,
            replace: true,
        };
        d.name_taken();
        assert_eq!(d, Dialog::default());
    }

    #[test]
    fn nothing_is_sent_offline_or_after_the_request_ran_out_but_leaving_still_works() {
        let mut d = Dialog {
            step: Step::Confirm,
            replace: false,
        };
        assert_eq!(d.key(b'y', false, false), Outcome::Stay);
        let mut d = Dialog::default();
        assert_eq!(d.key(b'd', false, false), Outcome::Stay);
        assert_eq!(d.key(0x1b, false, false), Outcome::Leave);
        let c = card("a");
        assert!(can_answer(&c, c.expires_ms - 1, true));
        assert!(!can_answer(&c, c.expires_ms, true));
        assert!(!can_answer(&c, 0, false));
    }

    #[test]
    fn a_key_sooner_than_half_a_second_after_the_card_is_not_an_answer() {
        use std::time::Duration;
        assert!(too_soon(b'a', Duration::from_millis(0)));
        assert!(too_soon(b'y', MIN_THINK - Duration::from_millis(1)));
        assert!(!too_soon(b'y', MIN_THINK));
        // Esc is always taken: leaving is safe.
        assert!(!too_soon(0x1b, Duration::from_millis(0)));
    }

    #[test]
    fn d_denies_and_esc_leaves_it_pending() {
        let mut d = Dialog::default();
        assert_eq!(d.key(b'd', false, true), Outcome::Deny);
        let mut d = Dialog::default();
        assert_eq!(d.key(0x1b, false, true), Outcome::Leave);
        d.key(b'a', false, true);
        assert_eq!(d.key(0x1b, false, true), Outcome::Leave);
    }

    #[test]
    fn a_malformed_notice_is_dropped() {
        assert!(Card::from_frame(&requested(1, "a", "48219")).is_none());
        assert!(Card::from_frame(&requested(1, "a", "4821930")).is_none());
        assert!(Card::from_frame(&requested(1, "a", "48219x")).is_none());
        assert!(Card::from_frame(&requested(1, "", "482193")).is_none());
        let mut r = requested(1, "a", "482193");
        r.pair_id.clear();
        assert!(Card::from_frame(&r).is_none());
        let mut r = requested(1, "a", "482193");
        r.kind = PairKind::Unspecified as i32;
        assert!(Card::from_frame(&r).is_none());
    }

    #[test]
    fn no_card_is_shown_until_the_hub_says_this_is_the_owner_and_none_is_kept_for_anyone_else() {
        let mut b = Book::default();
        b.apply(PairNote::Requested(requested(1, "a", "482193")));
        assert!(b.shown().is_empty());
        b.set_owner(Owner::Yes);
        // What arrived before the answer was held and is shown now.
        assert_eq!(b.shown().len(), 1);
        let mut other = Book::default();
        other.apply(PairNote::Requested(requested(1, "a", "482193")));
        other.set_owner(Owner::No);
        assert!(other.shown().is_empty());
        other.apply(PairNote::Requested(requested(2, "b", "111222")));
        assert!(other.shown().is_empty());
        assert!(other.first().is_none());
    }

    #[test]
    fn cards_are_kept_four_at_most_one_per_request_and_ended_by_the_decision() {
        let mut b = owner_book();
        for i in 1..=5u8 {
            b.apply(PairNote::Requested(requested(
                i,
                &format!("m{i}"),
                "482193",
            )));
        }
        assert_eq!(b.shown().len(), MAX_CARDS);
        assert_eq!(b.first().unwrap().name, "m2");
        b.apply(PairNote::Requested(requested(3, "again", "482193")));
        assert_eq!(
            b.shown()
                .iter()
                .filter(|c| c.pair_id == vec![3; 16])
                .count(),
            1
        );
        b.apply(PairNote::Decided(PairDecided {
            pair_id: vec![2; 16],
            outcome: PairOutcome::Approved as i32,
        }));
        assert_eq!(b.shown().len(), MAX_CARDS - 1);
        assert_eq!(b.take_end(), Some(("m2".to_string(), Ended::Approved)));
        assert_eq!(b.take_end(), None);
        // A decision about a request this client never held says nothing.
        b.apply(PairNote::Decided(PairDecided {
            pair_id: vec![9; 16],
            outcome: PairOutcome::Denied as i32,
        }));
        assert_eq!(b.take_end(), None);
    }

    #[test]
    fn a_new_connection_removes_the_stale_cards_and_keeps_who_the_owner_is() {
        let mut b = owner_book();
        b.apply(PairNote::Requested(requested(1, "a", "482193")));
        b.reset();
        assert!(b.shown().is_empty());
        assert_eq!(b.owner, Owner::Yes);
        b.apply(PairNote::Requested(requested(1, "a", "482193")));
        assert_eq!(b.shown().len(), 1);
    }

    #[test]
    fn a_lost_link_asks_again_who_the_owner_is_and_a_routine_refresh_does_not() {
        // #1099: the hub ends a connection when the person's role changes; a member made the owner is asked again after that.
        let mut b = Book::default();
        b.set_owner(Owner::No);
        b.reset();
        assert_eq!(b.owner, Owner::No, "a refresh keeps the answer");
        b.lost();
        assert_eq!(b.owner, Owner::Unknown, "a lost link asks again");
        let mut b = owner_book();
        b.apply(PairNote::Requested(requested(1, "a", "482193")));
        b.lost();
        assert!(b.shown().is_empty());
        assert_eq!(b.owner, Owner::Unknown);
    }

    #[test]
    fn the_answer_to_pair_status_says_the_role_for_the_folder_picker() {
        use crate::picker::Role;
        assert_eq!(status_role(None), Some((Owner::Yes, Role::Owner)));
        assert_eq!(
            status_role(Some("auth.forbidden")),
            Some((Owner::No, Role::Limited))
        );
        // Pairing switched off: only the owner gets this far; the owner's picker is unchanged.
        assert_eq!(
            status_role(Some("protocol.capability_missing")),
            Some((Owner::No, Role::Owner))
        );
        assert_eq!(status_role(Some("hub.unreachable")), None);
    }

    #[test]
    fn a_name_that_is_taken_stays_marked_when_the_hub_sends_the_card_again() {
        let mut b = owner_book();
        b.apply(PairNote::Requested(requested(1, "a", "482193")));
        b.mark_taken(&[1; 16]);
        assert!(b.first().unwrap().taken);
        b.apply(PairNote::Requested(requested(1, "a", "482193")));
        assert!(b.first().unwrap().taken);
    }

    #[test]
    fn the_number_is_grouped_and_anything_else_is_left_alone() {
        assert_eq!(grouped("482193"), "482-193");
        assert_eq!(grouped("004017"), "004-017");
        assert_eq!(grouped("12"), "12");
    }
}
