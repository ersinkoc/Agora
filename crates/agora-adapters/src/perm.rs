//! Whether choosing an option of an agent's session makes the agent act without asking (ADR-0058, ADR-0059, ADR-0090).
//!
//! ACP has no field for it, so the bridge (and the node, for an adapter that says nothing) decides from the words an agent used. A
//! list of bad names alone is evaded by any name it does not list, so the rule has two parts:
//!
//! 1. A *denylist*: a value whose id, name or description says it acts without asking (`bypass`, `yolo`, `accept edits`, `auto`, a
//!    description saying `without asking`, ...) skips, whatever else is true.
//! 2. An *allowlist for permission options*: an option that is about permissions (ACP category `mode`, or an id or name that says
//!    `permission`, `approval`, `sandbox`, `trust`, ...) takes a value only as a safe one when its id or name is one of the clearly
//!    asking or read-only words (`default`, `ask`, `plan`, `read-only`, ...). A value the rule cannot place is treated as
//!    skipping: the user is asked to confirm it and a node whose policy forbids skipping refuses it. The unknown case is therefore
//!    never read as safe, and no rule here ever lowers protection: it only adds a flag.
//!
//! A model or a reasoning level (ACP categories `model`, `thought_level`) is never a permission option on its category alone; only
//! the strong words of the denylist apply to it (`auto` as a whole word does not: "Auto" is a common model router).

/// What an option is about, as far as the words of the rule are concerned (ACP `category`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    /// ACP `mode` (and the older `modes` list of `session/new`): the permission and working mode.
    Mode,
    /// ACP `model`.
    Model,
    /// ACP `model_config`, `thought_level`: a model's tuning, not a permission.
    Tuning,
    /// Anything else (`_custom`, no or an unknown category): permission-related only when its own words say so.
    Other,
}

impl Category {
    /// The ACP category word as a category of this rule; any other word is [`Category::Other`].
    pub fn from_acp(word: Option<&str>) -> Self {
        match word {
            Some("mode") => Self::Mode,
            Some("model") => Self::Model,
            Some("model_config" | "thought_level") => Self::Tuning,
            _ => Self::Other,
        }
    }
}

/// Words in a value's id or name (lowercase, letters and digits only) that mark a value in which the agent acts without asking.
const SKIPS_PERMISSIONS: &[&str] = &[
    "bypass",
    "yolo",
    "danger",
    "skippermission",
    "nopermission",
    "noapproval",
    "fullaccess",
    "autoapprove",
    "autoaccept",
    "acceptedits",
    "acceptall",
    "unsafe",
];

/// Words (squashed like the above) in an option's own id or name that say the option is about permissions.
const PERMISSION_WORDS: &[&str] = &[
    "permission",
    "approval",
    "approve",
    "sandbox",
    "autonomy",
    "trust",
    "safety",
    "access",
];

/// Values (squashed) that are clearly safe in a permission option: the agent asks, or may only read.
const SAFE_VALUES: &[&str] = &[
    "default",
    "ask",
    "askeachtime",
    "askbeforeedits",
    "askbeforeacting",
    "askalways",
    "alwaysask",
    "plan",
    "planmode",
    "planning",
    "readonly",
    "read",
    "suggest",
    "manual",
    "normal",
    "standard",
    "confirm",
    "build",
    "code",
    "architect",
    "debug",
    "review",
    "chat",
    "explain",
    "orchestrator",
];

fn squash(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// ADR-0058: whether a value's id, name or description says the agent acts without asking in it (the denylist). `auto` as a whole
/// word counts only when `auto_word` is set.
pub fn says_it_skips(id: &str, name: &str, description: &str, auto_word: bool) -> bool {
    // `auto` only as a whole word (`auto`, `Auto mode`, `auto-edit`), so `autosave` or `automatic-plan` are not marked by it.
    let has_word = |s: &str, w: &str| {
        s.split(|c: char| !c.is_ascii_alphanumeric())
            .any(|part| part.eq_ignore_ascii_case(w))
    };
    let d = description.to_ascii_lowercase();
    let approves_automatically = d
        .find("approve")
        .is_some_and(|at| d[at..].contains("automatically"));
    let (sid, sname) = (squash(id), squash(name));
    SKIPS_PERMISSIONS
        .iter()
        .any(|w| sid.contains(w) || sname.contains(w))
        || (auto_word && (has_word(id, "auto") || has_word(name, "auto")))
        || d.contains("without asking")
        || approves_automatically
}

/// Whether the option itself (its id or name) is about permissions.
fn about_permissions(option_id: &str, option_name: &str) -> bool {
    let (i, n) = (squash(option_id), squash(option_name));
    PERMISSION_WORDS
        .iter()
        .any(|w| i.contains(w) || n.contains(w))
}

/// Whether choosing `value` of the option makes the agent act without asking, or the node cannot tell and so must ask first.
/// `option_*` is the option (for the older `modes` list, the list itself); `value_*` is the choice. For a boolean option pass the
/// option's own words as the value (a boolean about permissions is flagged whichever way it is set: what `true` means is the
/// agent's word, and neither direction is known to keep protection).
pub fn value_skips(category: Category, option: (&str, &str), value: (&str, &str, &str)) -> bool {
    let (vid, vname, vdesc) = value;
    if says_it_skips(vid, vname, vdesc, category != Category::Model) {
        return true;
    }
    if matches!(category, Category::Model | Category::Tuning) {
        return false;
    }
    let related = category == Category::Mode || about_permissions(option.0, option.1);
    if !related {
        return false;
    }
    let (i, n) = (squash(vid), squash(vname));
    // Clearly safe when its id or its name is one of the asking or read-only words; anything else is unknown, so it skips.
    !(SAFE_VALUES.contains(&i.as_str()) || SAFE_VALUES.contains(&n.as_str()))
}

/// Whether a boolean option is about permissions (so changing it either way is treated as possibly skipping them).
pub fn boolean_skips(category: Category, option: (&str, &str), description: &str) -> bool {
    if matches!(category, Category::Model | Category::Tuning) {
        return says_it_skips(option.0, option.1, description, false);
    }
    says_it_skips(option.0, option.1, description, true)
        || category == Category::Mode
        || about_permissions(option.0, option.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_listed_name_skips_and_an_unlisted_one_in_a_mode_is_not_taken_for_safe() {
        let skips = |id: &str, name: &str, d: &str| {
            value_skips(Category::Mode, ("mode", "Mode"), (id, name, d))
        };
        assert!(skips("bypassPermissions", "Bypass Permissions", ""));
        // Not on the denylist: an unlisted name must not read as safe.
        assert!(skips("turbo", "Turbo", ""));
        assert!(skips("godmode", "God mode", ""));
        assert!(skips("agent", "Agent", ""));
        // The clearly asking and read-only ones are safe.
        for (id, name) in [
            ("default", "Default"),
            ("plan", "Plan"),
            ("build", "Build"),
            ("x", "Read only"),
            ("ask", "Ask each time"),
        ] {
            assert!(!skips(id, name, ""), "{id}");
        }
        // The denylist wins over a safe word.
        assert!(skips("default", "Default", "runs tools without asking"));
    }

    #[test]
    fn an_option_that_is_not_about_permissions_is_not_flagged_by_its_values() {
        let effort = |id: &str| value_skips(Category::Tuning, ("effort", "Effort"), (id, id, ""));
        assert!(!effort("high") && !effort("turbo"));
        let model = |id: &str| value_skips(Category::Model, ("model", "Model"), (id, id, ""));
        assert!(!model("auto") && !model("claude-sonnet"));
        assert!(model("yolo-model"), "strong words still apply to a model");
        // A custom option is permission-related only by its own words.
        let custom = |opt: &str, v: &str| value_skips(Category::Other, (opt, opt), (v, v, ""));
        assert!(!custom("verbosity", "turbo"));
        assert!(custom("approval_policy", "never"));
        assert!(!custom("approval_policy", "ask"));
        assert!(custom("sandbox", "danger-full-access"));
        assert!(
            custom("sandbox", "workspace-write"),
            "an unknown sandbox value is not safe"
        );
        assert!(!custom("sandbox", "read-only"));
    }

    #[test]
    fn a_boolean_about_permissions_is_flagged_and_an_unrelated_one_is_not() {
        let b = |cat, id: &str, name: &str| boolean_skips(cat, (id, name), "");
        assert!(b(Category::Other, "auto_approve", "Auto approve"));
        assert!(b(Category::Other, "approvals", "Ask for approval"));
        assert!(b(Category::Mode, "x", "X"));
        assert!(!b(Category::Other, "verbose", "Verbose"));
        assert!(!b(Category::Tuning, "extended", "Extended thinking"));
    }
}
