//! Launch options chosen when an agent starts (ADR-0059): which a start may name, what they add to the command, and whether the
//! node's owner allows the ones that make an agent act without asking. Plain data and pure functions; the supervisor applies them.

use agora_adapters::manifest::{valid_option_id, Manifest, MAX_LAUNCH_OPTIONS};

/// The node's `skip_permissions` policy line (`<data>/policy`), set by the machine's owner and never over the hub.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SkipPermissions {
    /// No agent may start with an option that skips permissions.
    Forbid,
    /// The default the maintainer chose (ADR-0059): allowed, with the confirmation, the signature, the audit and the badge.
    #[default]
    Allow,
    /// Allowed only while the agent's confinement (ADR-0047/0050) is active.
    ConfinedOnly,
}

impl SkipPermissions {
    /// A value that is not one of the three counts as `forbid`: a typo must never widen what the node allows.
    pub fn parse(v: &str) -> Self {
        match v.trim() {
            "allow" => Self::Allow,
            "confined_only" => Self::ConfinedOnly,
            _ => Self::Forbid,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Forbid => "forbid",
            Self::Allow => "allow",
            Self::ConfinedOnly => "confined_only",
        }
    }
}

/// What a start chose, resolved against the kind's manifest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchChoice {
    /// The chosen ids, in the manifest's order.
    pub ids: Vec<String>,
    /// Their arguments, in the same order.
    pub args: Vec<String>,
    /// One of them makes the agent act without asking.
    pub skips_permissions: bool,
}

/// Why a choice was refused before anything started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Too many ids, a repeated one, or one the kind does not declare: `protocol.invalid_message`.
    Invalid,
    /// `agent.option_forbidden`: the first refused id and the policy that refused it (`forbid`, `confined_only`).
    Forbidden {
        option: String,
        policy: &'static str,
    },
    /// `agent.option_unconfirmed`: this node's manifest says the option skips permissions, and the start did not carry the confirmation
    /// marker (`<id>!`): the window did not show the confirmation for it (a hub that relabelled it as harmless).
    Unconfirmed { option: String },
}

/// The confirmation marker: a start names an option that skips permissions as `<id>!` when the user confirmed "Let the agent act
/// without asking?" for it. The signature covers the marker; the node, not the window, knows which options skip permissions.
pub const CONFIRMED: char = '!';

/// An id as sent, split into the id and whether it carries the confirmation marker.
pub fn split_marker(sent: &str) -> (&str, bool) {
    match sent.strip_suffix(CONFIRMED) {
        Some(id) => (id, true),
        None => (sent, false),
    }
}

/// Resolves `ids` against `manifest` and checks them against the node's policy. `confined` says whether this agent's confinement
/// would be active (only `confined_only` asks).
pub fn choose(
    manifest: &Manifest,
    ids: &[String],
    policy: SkipPermissions,
    confined: bool,
) -> Result<LaunchChoice, Refusal> {
    if ids.len() > MAX_LAUNCH_OPTIONS {
        return Err(Refusal::Invalid);
    }
    let sent: Vec<(&str, bool)> = ids.iter().map(|s| split_marker(s)).collect();
    for (i, (id, _)) in sent.iter().enumerate() {
        if !valid_option_id(id)
            || sent[..i].iter().any(|(x, _)| x == id)
            || manifest.launch_option(id).is_none()
        {
            return Err(Refusal::Invalid);
        }
    }
    let mut choice = LaunchChoice::default();
    for o in manifest.launch.option.iter() {
        let Some((_, confirmed)) = sent.iter().find(|(id, _)| *id == o.id) else {
            continue;
        };
        if o.skips_permissions && !confirmed {
            return Err(Refusal::Unconfirmed {
                option: o.id.clone(),
            });
        }
        if o.skips_permissions {
            let refused = match policy {
                SkipPermissions::Allow => None,
                SkipPermissions::Forbid => Some("forbid"),
                SkipPermissions::ConfinedOnly if confined => None,
                SkipPermissions::ConfinedOnly => Some("confined_only"),
            };
            if let Some(policy) = refused {
                return Err(Refusal::Forbidden {
                    option: o.id.clone(),
                    policy,
                });
            }
            choice.skips_permissions = true;
        }
        choice.ids.push(o.id.clone());
        choice.args.extend(o.args.iter().cloned());
    }
    Ok(choice)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        Manifest::parse(concat!(
            "kind = \"k\"\ndisplay_name = \"K\"\nversion = \"1\"\nmin_engine = 2\n[process]\nlaunch = [\"k\"]\n",
            "[[launch.option]]\nid = \"skip-permissions\"\nlabel = \"option-skip-permissions\"\nargs = [\"--yes\", \"--really\"]\n",
            "skips_permissions = true\n",
            "[[launch.option]]\nid = \"resume-last\"\nlabel = \"option-resume-last\"\nargs = [\"--continue\"]\n",
        ))
        .unwrap()
    }

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn the_policy_reads_three_values_and_anything_else_forbids() {
        assert_eq!(SkipPermissions::default(), SkipPermissions::Allow);
        assert_eq!(SkipPermissions::parse(" allow "), SkipPermissions::Allow);
        assert_eq!(
            SkipPermissions::parse("confined_only"),
            SkipPermissions::ConfinedOnly
        );
        for v in ["forbid", "", "yes", "Allow", "true"] {
            assert_eq!(SkipPermissions::parse(v), SkipPermissions::Forbid, "{v}");
        }
    }

    #[test]
    fn an_option_that_skips_permissions_needs_the_confirmation_marker() {
        // A hub that relabelled it as harmless: the window showed no confirmation and signed the bare id.
        assert_eq!(
            choose(
                &manifest(),
                &ids(&["skip-permissions"]),
                SkipPermissions::Allow,
                true
            ),
            Err(Refusal::Unconfirmed {
                option: "skip-permissions".into()
            })
        );
        // A marker on a harmless option is harmless; a repeated id with and without it is a bad request.
        let c = choose(
            &manifest(),
            &ids(&["resume-last!"]),
            SkipPermissions::Forbid,
            false,
        )
        .unwrap();
        assert_eq!(c.ids, ids(&["resume-last"]));
        assert_eq!(
            choose(
                &manifest(),
                &ids(&["skip-permissions!", "skip-permissions"]),
                SkipPermissions::Allow,
                true
            ),
            Err(Refusal::Invalid)
        );
    }

    #[test]
    fn options_add_their_arguments_in_the_manifests_order() {
        let c = choose(
            &manifest(),
            &ids(&["resume-last", "skip-permissions!"]),
            SkipPermissions::Allow,
            false,
        )
        .unwrap();
        assert_eq!(c.ids, ids(&["skip-permissions", "resume-last"]));
        assert_eq!(c.args, ids(&["--yes", "--really", "--continue"]));
        assert!(c.skips_permissions);
        let none = choose(&manifest(), &[], SkipPermissions::Forbid, false).unwrap();
        assert_eq!(none, LaunchChoice::default());
    }

    #[test]
    fn forbid_allow_and_confined_only_decide_only_options_that_skip_permissions() {
        let skip = ids(&["skip-permissions!"]);
        let forbidden = |policy| Refusal::Forbidden {
            option: "skip-permissions".into(),
            policy,
        };
        assert_eq!(
            choose(&manifest(), &skip, SkipPermissions::Forbid, true),
            Err(forbidden("forbid"))
        );
        assert_eq!(
            choose(&manifest(), &skip, SkipPermissions::ConfinedOnly, false),
            Err(forbidden("confined_only"))
        );
        assert!(choose(&manifest(), &skip, SkipPermissions::ConfinedOnly, true).is_ok());
        assert!(choose(&manifest(), &skip, SkipPermissions::Allow, false).is_ok());
        // A harmless option is never refused by the policy.
        let harmless = choose(
            &manifest(),
            &ids(&["resume-last"]),
            SkipPermissions::Forbid,
            false,
        )
        .unwrap();
        assert!(!harmless.skips_permissions);
    }

    #[test]
    fn unknown_repeated_or_too_many_ids_are_invalid() {
        for bad in [
            ids(&["other"]),
            ids(&["resume-last", "resume-last"]),
            ids(&["Skip-Permissions"]),
            ids(&["a,b"]),
            vec!["resume-last".to_string(); MAX_LAUNCH_OPTIONS + 1],
        ] {
            assert_eq!(
                choose(&manifest(), &bad, SkipPermissions::Allow, true),
                Err(Refusal::Invalid),
                "{bad:?}"
            );
        }
    }
}
