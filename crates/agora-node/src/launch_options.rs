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

    /// Why this policy refuses something that makes an agent act without asking (`forbid`, `confined_only`), or `None` when it allows
    /// it. `confined` says whether the agent's confinement is active.
    pub fn refusal(self, confined: bool) -> Option<&'static str> {
        match self {
            Self::Allow => None,
            Self::Forbid => Some("forbid"),
            Self::ConfinedOnly if confined => None,
            Self::ConfinedOnly => Some("confined_only"),
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
    /// ADR-0066/ADR-0123: the settings applied, `(id, value)` in the manifest's order (their arguments are in `args`).
    pub settings: Vec<(String, String)>,
}

/// What a start asked for: the chosen ids, and the ids whose "act without asking" confirmation the window showed and the user
/// accepted. Both are signed (`directory-action-v3`); the confirmation is never read out of an id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Requested {
    pub ids: Vec<String>,
    pub confirmed: Vec<String>,
    /// ADR-0066: `(setting id, value)` pairs, at most one per id.
    pub settings: Vec<(String, String)>,
}

impl Requested {
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty() && self.confirmed.is_empty() && self.settings.is_empty()
    }
}

/// Why a choice was refused before anything started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// `agent.option_invalid`: too many ids, one that is not `[a-z0-9-]{1,32}` (a `!` or `,` in it included), a repeated one, one the
    /// kind does not declare, or a confirmation for an id that was not chosen. `option` is the first such id (empty for too many).
    Invalid { option: String },
    /// `agent.option_forbidden`: the first refused id and the policy that refused it (`forbid`, `confined_only`).
    Forbidden {
        option: String,
        policy: &'static str,
    },
    /// `agent.option_unconfirmed`: this node's manifest says the option skips permissions, and the start did not name it among the
    /// confirmed ids (the window did not show the confirmation for it, for example because a hub relabelled it as harmless).
    Unconfirmed { option: String },
    /// ADR-0066 `agent.setting_unknown`: a setting the kind does not declare, or one named twice, or too many.
    SettingUnknown { setting: String },
    /// ADR-0066 `agent.setting_invalid`: a value the setting does not take.
    SettingInvalid { setting: String },
}

/// Resolves a start's ids against `manifest` and checks them against the node's policy. Which options skip permissions is this node's
/// manifest's word, never the window's. `confined` says whether this agent's confinement would be active (only `confined_only` asks).
pub fn choose(
    manifest: &Manifest,
    req: &Requested,
    policy: SkipPermissions,
    confined: bool,
) -> Result<LaunchChoice, Refusal> {
    let invalid = |option: &str| Refusal::Invalid {
        option: option.to_string(),
    };
    if req.ids.len() > MAX_LAUNCH_OPTIONS || req.confirmed.len() > MAX_LAUNCH_OPTIONS {
        return Err(invalid(""));
    }
    for (i, id) in req.ids.iter().enumerate() {
        if !valid_option_id(id) || req.ids[..i].contains(id) || manifest.launch_option(id).is_none()
        {
            return Err(invalid(id));
        }
    }
    for (i, id) in req.confirmed.iter().enumerate() {
        if !req.ids.contains(id) || req.confirmed[..i].contains(id) {
            return Err(invalid(id));
        }
    }
    let mut choice = LaunchChoice::default();
    for o in manifest
        .launch
        .option
        .iter()
        .filter(|o| req.ids.contains(&o.id))
    {
        if o.skips_permissions {
            if !req.confirmed.contains(&o.id) {
                return Err(Refusal::Unconfirmed {
                    option: o.id.clone(),
                });
            }
            if let Some(policy) = policy.refusal(confined) {
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
    // ADR-0066: each chosen setting's arguments, in the manifest's order, after the options. The value is checked here, on the node,
    // against this node's manifest, and only ever becomes part of one argument.
    if req.settings.len() > agora_adapters::manifest::MAX_LAUNCH_SETTINGS {
        return Err(Refusal::SettingUnknown {
            setting: String::new(),
        });
    }
    for (i, (id, _)) in req.settings.iter().enumerate() {
        if manifest.launch_setting(id).is_none() || req.settings[..i].iter().any(|(o, _)| o == id) {
            return Err(Refusal::SettingUnknown {
                setting: id.clone(),
            });
        }
    }
    for st in &manifest.launch.setting {
        if let Some((_, value)) = req.settings.iter().find(|(id, _)| *id == st.id) {
            let args = st.args_for(value).ok_or_else(|| Refusal::SettingInvalid {
                setting: st.id.clone(),
            })?;
            choice.args.extend(args);
            choice.settings.push((st.id.clone(), value.clone()));
        }
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

    fn req(chosen: &[&str], confirmed: &[&str]) -> Requested {
        Requested {
            ids: ids(chosen),
            confirmed: ids(confirmed),
            settings: Vec::new(),
        }
    }

    const SKIP: &[&str] = &["skip-permissions"];

    fn with_settings() -> Manifest {
        Manifest::parse(concat!(
            "kind = \"k\"\ndisplay_name = \"K\"\nversion = \"1\"\nmin_engine = 3\n[process]\nlaunch = [\"k\"]\n",
            "[[launch.option]]\nid = \"resume-last\"\nlabel = \"option-resume-last\"\nargs = [\"--continue\"]\n",
            "[[launch.setting]]\nid = \"model\"\nlabel = \"setting-model\"\nargs = [\"--model\", \"{value}\"]\n",
            "[[launch.setting]]\nid = \"effort\"\nlabel = \"setting-effort\"\nargs = [\"--effort\", \"{value}\"]\n",
            "values = [\"low\", \"high\"]\n",
        ))
        .unwrap()
    }

    fn settings(v: &[(&str, &str)]) -> Requested {
        Requested {
            ids: ids(&["resume-last"]),
            confirmed: Vec::new(),
            settings: v
                .iter()
                .map(|(i, x)| ((*i).to_string(), (*x).to_string()))
                .collect(),
        }
    }

    #[test]
    fn settings_add_their_arguments_after_the_options_in_the_manifests_order() {
        // ADR-0066: whatever order the start names them in.
        let c = choose(
            &with_settings(),
            &settings(&[("effort", "high"), ("model", "gpt-6.1-sol")]),
            SkipPermissions::Allow,
            false,
        )
        .unwrap();
        assert_eq!(c.ids, ["resume-last"]);
        assert_eq!(
            c.args,
            ["--continue", "--model", "gpt-6.1-sol", "--effort", "high"]
        );
        // ADR-0123: and are kept as chosen, in the same order, for `AgentStarted.launch_settings`.
        assert_eq!(
            c.settings,
            [
                ("model".to_string(), "gpt-6.1-sol".to_string()),
                ("effort".to_string(), "high".to_string())
            ]
        );
    }

    #[test]
    fn an_unknown_repeated_or_bad_setting_starts_nothing() {
        let refused = |v: &[(&str, &str)]| {
            choose(
                &with_settings(),
                &settings(v),
                SkipPermissions::Allow,
                false,
            )
            .unwrap_err()
        };
        let unknown = |s: &str| Refusal::SettingUnknown { setting: s.into() };
        let invalid = |s: &str| Refusal::SettingInvalid { setting: s.into() };
        assert_eq!(refused(&[("mode", "plan")]), unknown("mode"));
        assert_eq!(refused(&[("model", "a"), ("model", "b")]), unknown("model"));
        assert_eq!(refused(&[("effort", "max")]), invalid("effort"));
        // A value that would be a flag, or two arguments, is never put on the command line.
        assert_eq!(
            refused(&[("model", "--dangerously-skip-permissions")]),
            invalid("model")
        );
        assert_eq!(refused(&[("model", "x --yolo")]), invalid("model"));
        let many: Vec<(&str, &str)> =
            vec![("model", "a"); agora_adapters::manifest::MAX_LAUNCH_SETTINGS + 1];
        assert_eq!(refused(&many), unknown(""));
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
    fn an_option_that_skips_permissions_needs_its_signed_confirmation() {
        // A hub that relabelled it as harmless: the window asked nothing, so nothing is confirmed.
        assert_eq!(
            choose(&manifest(), &req(SKIP, &[]), SkipPermissions::Allow, true),
            Err(Refusal::Unconfirmed {
                option: "skip-permissions".into()
            })
        );
        // A hub that advertised `skip-permissions!`: that is not an id, and a `!` never confirms anything.
        assert_eq!(
            choose(
                &manifest(),
                &req(&["skip-permissions!"], &[]),
                SkipPermissions::Allow,
                true
            ),
            Err(Refusal::Invalid {
                option: "skip-permissions!".into()
            })
        );
        // A confirmation for an id that was not chosen, or repeated, is a bad request; one on a harmless option is harmless.
        assert!(matches!(
            choose(
                &manifest(),
                &req(&["resume-last"], SKIP),
                SkipPermissions::Allow,
                true
            ),
            Err(Refusal::Invalid { .. })
        ));
        assert!(matches!(
            choose(
                &manifest(),
                &req(SKIP, &["skip-permissions", "skip-permissions"]),
                SkipPermissions::Allow,
                true
            ),
            Err(Refusal::Invalid { .. })
        ));
        let c = choose(
            &manifest(),
            &req(&["resume-last"], &["resume-last"]),
            SkipPermissions::Forbid,
            false,
        )
        .unwrap();
        assert_eq!(c.ids, ids(&["resume-last"]));
    }

    #[test]
    fn options_add_their_arguments_in_the_manifests_order() {
        let c = choose(
            &manifest(),
            &req(&["resume-last", "skip-permissions"], SKIP),
            SkipPermissions::Allow,
            false,
        )
        .unwrap();
        assert_eq!(c.ids, ids(&["skip-permissions", "resume-last"]));
        assert_eq!(c.args, ids(&["--yes", "--really", "--continue"]));
        assert!(c.skips_permissions);
        let none = choose(
            &manifest(),
            &Requested::default(),
            SkipPermissions::Forbid,
            false,
        )
        .unwrap();
        assert_eq!(none, LaunchChoice::default());
    }

    #[test]
    fn forbid_allow_and_confined_only_decide_only_options_that_skip_permissions() {
        let skip = req(SKIP, SKIP);
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
        let harmless = choose(
            &manifest(),
            &req(&["resume-last"], &[]),
            SkipPermissions::Forbid,
            false,
        )
        .unwrap();
        assert!(!harmless.skips_permissions);
    }

    #[test]
    fn unknown_repeated_malformed_or_too_many_ids_are_invalid() {
        for bad in [
            ids(&["other"]),
            ids(&["resume-last", "resume-last"]),
            ids(&["Skip-Permissions"]),
            ids(&["a,b"]),
            ids(&["skip-permissions!"]),
            vec!["resume-last".to_string(); MAX_LAUNCH_OPTIONS + 1],
        ] {
            let r = Requested {
                ids: bad.clone(),
                confirmed: Vec::new(),
                settings: Vec::new(),
            };
            assert!(
                matches!(
                    choose(&manifest(), &r, SkipPermissions::Allow, true),
                    Err(Refusal::Invalid { .. })
                ),
                "{bad:?}"
            );
        }
    }
}
