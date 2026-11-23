//! ADR-0106 in the terminal client: what the person's role, as the hub's `Welcome` named it, changes in what the client asks and shows.
//! The role is a hint for the window, never a permission: the hub still checks every command. With the role unspecified (an older
//! hub) the client learns it as before (#1086, #1099): from the hub's answer to `PairStatus`, or a folder refused for the role.

use agora_protocol::v1::PersonRole;

use crate::client::Session;
use crate::pair::Owner;
use crate::picker::Role;

/// What a new connection does about the pairing cards and the folder picker's role.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Learn {
    /// What is held stands; nothing is asked.
    Keep,
    /// Ask the hub (`PairStatus`): answered to the owner only, and every refusal is a row in the owner's Activity, so only the owner, or a
    /// connection that does not know the role, asks.
    Ask,
    /// Known from the `Welcome`: nothing is asked.
    Set(Owner, Role),
}

/// What a connection whose `Welcome` named `person` does, holding `held` (the pairing answer) and `role` (the picker's) from before.
pub fn learn(person: PersonRole, held: Owner, role: Role) -> Learn {
    match person {
        // An older hub: asked once per connection that followed a lost link, as before (#1099).
        PersonRole::Unspecified if held == Owner::Unknown => Learn::Ask,
        PersonRole::Unspecified => Learn::Keep,
        // A member or viewer is never asked: the hub would refuse `PairStatus`, and write the refusal down.
        PersonRole::Member | PersonRole::Viewer => Learn::Set(Owner::No, Role::Limited),
        // The owner whose pairing answer is held: the hub's own, or one with pairing switched off (`protocol.capability_missing`, the
        // owner's picker). Anything else (nothing held, or a member made the owner) asks whether pairing is on.
        PersonRole::Owner if held == Owner::Yes || (held == Owner::No && role == Role::Owner) => {
            Learn::Keep
        }
        PersonRole::Owner => Learn::Ask,
    }
}

/// What the console holds after a connection opened: the pairing answer and the picker's role, asking the hub only when [`learn`]
/// says so. `None` when the hub could not tell (the link failed): what was held stays, and it is asked again at the next connection.
pub fn learn_on(session: &mut Session, held: Owner, role: Role) -> Option<(Owner, Role)> {
    let person = session.person_role();
    match learn(person, held, role) {
        Learn::Keep => Some((held, role)),
        Learn::Set(owner, role) => Some((owner, role)),
        Learn::Ask => {
            let answer = session.command_info(
                "",
                agora_protocol::v1::command_request::Command::PairStatus(
                    agora_protocol::v1::PairStatus {},
                ),
                std::time::Duration::from_secs(3),
            );
            let got = crate::pair::status_role(answer.as_ref().err().map(|e| e.code.as_str()));
            match (got, person) {
                // The `Welcome` said owner: the picker is the owner's, whatever the pairing answer was.
                (Some((owner, _)), PersonRole::Owner) => Some((owner, Role::Owner)),
                (None, PersonRole::Owner) => None,
                (got, _) => got,
            }
        }
    }
}

/// A key of the list that changes something (answers a request, starts, stops, resumes, removes): hidden from a viewer, whose window
/// only watches (ADR-0106). The hub would refuse each, and write the refusal down.
pub fn changes(key: char) -> bool {
    matches!(
        key,
        'y' | 'n' | crate::app::ALLOW_ALWAYS | 's' | 'f' | 'x' | 'u' | 'U' | 'd'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_member_or_viewer_is_never_asked_and_gets_the_projects_picker() {
        for person in [PersonRole::Member, PersonRole::Viewer] {
            for held in [Owner::Unknown, Owner::Yes, Owner::No] {
                for role in [Role::Unknown, Role::Owner, Role::Limited] {
                    assert_eq!(
                        learn(person, held, role),
                        Learn::Set(Owner::No, Role::Limited),
                        "{person:?} {held:?} {role:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_owner_asks_only_when_the_pairing_answer_is_not_the_owners() {
        assert_eq!(
            learn(PersonRole::Owner, Owner::Unknown, Role::Unknown),
            Learn::Ask
        );
        assert_eq!(
            learn(PersonRole::Owner, Owner::Yes, Role::Owner),
            Learn::Keep
        );
        // Pairing switched off: the owner's answer, kept.
        assert_eq!(
            learn(PersonRole::Owner, Owner::No, Role::Owner),
            Learn::Keep
        );
        // A member made the owner over a routine reconnect: the "not the owner" answer held is asked again at once.
        assert_eq!(
            learn(PersonRole::Owner, Owner::No, Role::Limited),
            Learn::Ask
        );
    }

    #[test]
    fn an_older_hub_keeps_the_way_before() {
        assert_eq!(
            learn(PersonRole::Unspecified, Owner::Unknown, Role::Unknown),
            Learn::Ask
        );
        for held in [Owner::Yes, Owner::No] {
            assert_eq!(
                learn(PersonRole::Unspecified, held, Role::Limited),
                Learn::Keep
            );
        }
    }

    #[test]
    fn a_viewer_loses_the_keys_that_change_something_and_keeps_the_ones_that_read() {
        for k in [
            'y',
            'n',
            crate::app::ALLOW_ALWAYS,
            's',
            'f',
            'x',
            'u',
            'U',
            'd',
        ] {
            assert!(changes(k), "{k}");
        }
        for k in ['r', 'o', 'b', 'i', 'a', 'v', 'p', 'q'] {
            assert!(!changes(k), "{k}");
        }
    }
}
