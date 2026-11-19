//! Whether choosing an option of an agent's session makes the agent act without asking (ADR-0058, ADR-0059, ADR-0090).
//!
//! ACP has no field for it, so the bridge (and the node, for an adapter that says nothing) decides from the words an agent used. A
//! list of bad names alone is evaded by any name it does not list, so the rule has two parts:
//!
//! 1. A *denylist*: a value whose id, name or description says it acts without asking (`bypass`, `yolo`, `accept edits`, `auto`,
//!    `autonomous`, `never ask`, a description saying `skips all permission prompts` or `automatically approves`, ...) skips,
//!    whatever else is true. It runs first, so no allowlisted id can hide what the name or the description says.
//! 2. An *allowlist for permission options*: an option that is about permissions (ACP category `mode`, or an id or name with a word
//!    such as `mode`, `permission`, `approval`, `sandbox`, `allow`, `execution`, `policy`) takes a value only as a safe one when its id
//!    or name is one of the clearly asking or read-only words (`default`, `ask`, `plan`, `read-only`, ...). A value the rule cannot
//!    place is treated as skipping: the user is asked to confirm it and a node whose policy forbids skipping refuses it. The unknown
//!    case is therefore never read as safe, and no rule here ever lowers protection: it only adds a flag.
//!
//! Text is folded first (full-width ASCII to ASCII, lower case; everything that is not a letter or digit is dropped when words are
//! squashed), so `ＢＹＰＡＳＳ` and `by-pass` read as `bypass`.
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

/// Words in a value's id or name (folded, letters and digits only) that mark a value in which the agent acts without asking.
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
    "autonomous",
    "autopilot",
    "godmode",
    "neverask",
    "dontask",
    "donotask",
    "noask",
    "noprompt",
    "noconfirm",
    "skipconfirm",
    "skipprompt",
    "allowall",
    "alwaysallow",
    "approveall",
    "trustall",
    "everything",
];

/// Phrases (folded to lower case, single spaces) in a description that say the agent acts without asking.
const SKIP_PHRASES: &[&str] = &[
    "without asking",
    "without confirm",
    "without prompt",
    "without permission",
    "without approval",
    "skips all",
    "skip all",
    "skips permission",
    "skip permission",
    "skips approval",
    "skip approval",
    "skips confirm",
    "bypass",
    "auto-approve",
    "auto approve",
    "autoapprove",
    "auto-accept",
    "auto accept",
    "never ask",
    "never prompt",
    "don't ask",
    "dont ask",
    "do not ask",
    "doesn't ask",
    "does not ask",
    "no prompt",
    "no permission",
    "no confirmation",
    "no approval",
    "yolo",
    "dangerous",
    "all permissions",
    "full access",
];

/// Words (folded, whole) in an option's own id or name that say the option is about permissions.
const PERMISSION_TOKENS: &[&str] = &[
    "mode",
    "allow",
    "allowed",
    "policy",
    "execution",
    "exec",
    "perms",
    "autonomy",
    "yolo",
    "auto",
    "access",
    "trust",
    "ask",
];

/// Words (folded, letters and digits only) found anywhere in an option's id or name that say the option is about permissions.
const PERMISSION_SUBSTRINGS: &[&str] = &[
    "permission",
    "approval",
    "approve",
    "sandbox",
    "autonomy",
    "trust",
    "safety",
    "confirm",
    "bypass",
    "accept",
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

/// Folds `s`: full-width ASCII (U+FF01 to U+FF5E) and the ideographic space become their ASCII forms, then lower case.
fn fold(s: &str) -> String {
    s.chars()
        .flat_map(|c| {
            let c = match c {
                '\u{3000}' => ' ',
                '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
                c => c,
            };
            c.to_lowercase()
        })
        .collect()
}

/// Letters and digits of the folded `s` only (`by-pass` and `BY PASS` are `bypass`).
fn squash(s: &str) -> String {
    fold(s)
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect()
}

/// The words of the folded `s` (runs of ASCII letters and digits).
fn words(s: &str) -> Vec<String> {
    fold(s)
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// ADR-0058: whether a value's id, name or description says the agent acts without asking in it (the denylist). `auto` as a whole
/// word counts only when `auto_word` is set.
pub fn says_it_skips(id: &str, name: &str, description: &str, auto_word: bool) -> bool {
    // `auto` only as a whole word (`auto`, `Auto mode`, `auto-edit`), so `autosave` or `automatic-plan` are not marked by it.
    let has_auto = |s: &str| words(s).iter().any(|w| w == "auto");
    let d = fold(description);
    let d = d.split_whitespace().collect::<Vec<_>>().join(" ");
    // Both words, in either order: "approves ... automatically", "Automatically approves tool calls".
    let approves_automatically =
        (d.contains("approv") || d.contains("accept")) && d.contains("automatic");
    let (sid, sname) = (squash(id), squash(name));
    SKIPS_PERMISSIONS
        .iter()
        .any(|w| sid.contains(w) || sname.contains(w))
        || (auto_word && (has_auto(id) || has_auto(name)))
        || SKIP_PHRASES.iter().any(|p| d.contains(p))
        || approves_automatically
}

/// Whether the option itself (its id or name) is about permissions.
fn about_permissions(option_id: &str, option_name: &str) -> bool {
    let (i, n) = (squash(option_id), squash(option_name));
    let w: Vec<String> = words(option_id)
        .into_iter()
        .chain(words(option_name))
        .collect();
    PERMISSION_SUBSTRINGS
        .iter()
        .any(|s| i.contains(s) || n.contains(s))
        || PERMISSION_TOKENS.iter().any(|t| w.iter().any(|x| x == t))
}

/// Whether choosing `value` of the option makes the agent act without asking, or the node cannot tell and so must ask first.
/// `option_*` is the option (for the older `modes` list, the list itself); `value_*` is the choice. For a boolean option use
/// [`boolean_skips`] and [`boolean_value_skips`].
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
    // Clearly safe when its id or its name is one of the asking or read-only words; anything else is unknown, so it skips. The
    // denylist ran first: `ask` named "Never ask" has been flagged above.
    !(SAFE_VALUES.contains(&i.as_str()) || SAFE_VALUES.contains(&n.as_str()))
}

/// Whether a boolean option is about permissions (so the window asks before it is changed, and a node whose policy forbids skipping
/// refuses the risky direction, see [`boolean_value_skips`]).
pub fn boolean_skips(category: Category, option: (&str, &str), description: &str) -> bool {
    if matches!(category, Category::Model | Category::Tuning) {
        return says_it_skips(option.0, option.1, description, false);
    }
    says_it_skips(option.0, option.1, description, true)
        || category == Category::Mode
        || about_permissions(option.0, option.1)
}

/// Which value of a boolean permission option is the risky one, from the option's words: `Some(true)` when turning it on grants
/// something (`auto_approve`, `always_allow`, `disable_confirmations`), `Some(false)` when turning it off removes a protection
/// (`ask_for_approval`, `sandbox`), `None` when the words do not say.
pub fn boolean_risky_value(option: (&str, &str)) -> Option<bool> {
    const GRANTS_TOKENS: &[&str] = &[
        "auto",
        "allow",
        "allowed",
        "skip",
        "full",
        "disable",
        "disabled",
        "off",
        "no",
        "never",
        "yolo",
        "bypass",
        "trust",
        "unsafe",
        "accept",
        "autonomous",
    ];
    const GRANTS_SUBSTRINGS: &[&str] = &[
        "autoapprove",
        "autoaccept",
        "approveall",
        "alwaysallow",
        "allowall",
        "skippermission",
        "bypass",
        "disable",
        "noask",
        "dontask",
        "neverask",
        "unsafe",
        "yolo",
        "trustall",
    ];
    const PROTECTS_TOKENS: &[&str] = &[
        "ask", "confirm", "prompt", "require", "review", "sandbox", "safe", "protect", "verify",
    ];
    let w: Vec<String> = words(option.0).into_iter().chain(words(option.1)).collect();
    let sq = format!("{}{}", squash(option.0), squash(option.1));
    let has = |l: &[&str]| w.iter().any(|x| l.contains(&x.as_str()));
    if has(GRANTS_TOKENS) || GRANTS_SUBSTRINGS.iter().any(|s| sq.contains(s)) {
        return Some(true);
    }
    if has(PROTECTS_TOKENS) || sq.contains("confirm") {
        return Some(false);
    }
    None
}

/// Whether setting a boolean option to `value` makes the agent act without asking, or cannot be told not to. `flagged` is the
/// adapter's own flag for the option. A change TO the protective value is never a risk: turning `auto_approve` off, or
/// `ask_for_approval` on, is allowed even where the node's policy forbids skipping permissions. When the words do not say which
/// value is protective, both are treated as risky.
pub fn boolean_value_skips(
    category: Category,
    option: (&str, &str),
    description: &str,
    flagged: bool,
    value: bool,
) -> bool {
    if !(flagged || boolean_skips(category, option, description)) {
        return false;
    }
    match boolean_risky_value(option) {
        Some(risky) => value == risky,
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(id: &str, name: &str, d: &str) -> bool {
        value_skips(Category::Mode, ("mode", "Mode"), (id, name, d))
    }

    #[test]
    fn a_listed_name_skips_and_an_unlisted_one_in_a_mode_is_not_taken_for_safe() {
        assert!(mode("bypassPermissions", "Bypass Permissions", ""));
        // Not on the denylist: an unlisted name must not read as safe.
        assert!(mode("turbo", "Turbo", ""));
        assert!(mode("godmode", "God mode", ""));
        assert!(mode("agent", "Agent", ""));
        // The clearly asking and read-only ones are safe.
        for (id, name) in [
            ("default", "Default"),
            ("plan", "Plan"),
            ("build", "Build"),
            ("x", "Read only"),
            ("ask", "Ask each time"),
        ] {
            assert!(!mode(id, name, ""), "{id}");
        }
        // The denylist wins over a safe word.
        assert!(mode("default", "Default", "runs tools without asking"));
    }

    /// Every probe of the review of #779 that the first rule missed, one case each.
    #[test]
    fn the_probed_false_negatives_are_flagged() {
        // No category, an option whose id is `mode`, a value no list knows.
        assert!(value_skips(
            Category::Other,
            ("mode", "mode"),
            ("agent", "Agent", "")
        ));
        // An option called `execution` with the value `everything`.
        assert!(value_skips(
            Category::Other,
            ("execution", "Execution"),
            ("everything", "Everything", "")
        ));
        // `autonomous` in an option that is not about permissions at all.
        assert!(value_skips(
            Category::Other,
            ("style", "Style"),
            ("autonomous", "Autonomous", "")
        ));
        // The allowlist does not beat what the description says.
        assert!(mode("default", "Default", "Skips all permission prompts"));
        assert!(mode("plan", "Plan", "Bypasses the approval step"));
        // The name says the opposite of the id.
        assert!(mode("ask", "Never ask", ""));
        assert!(mode("default", "Don't ask me", ""));
        // Word order.
        assert!(mode("x", "X", "Automatically approves tool calls"));
        assert!(mode("x", "X", "Approve every edit automatically"));
        // Full-width and spaced-out spellings.
        assert!(mode("ｂｙｐａｓｓ", "ＢＹＰＡＳＳ", ""));
        assert!(mode("x", "Ｙ　Ｏ　Ｌ　Ｏ", ""));
        assert!(mode("x", "by-pass", ""));
        assert!(mode("default", "Default", "ｗｉｔｈｏｕｔ ａｓｋｉｎｇ"));
        // Booleans: `allow` and `confirm` are permission words.
        assert!(boolean_skips(
            Category::Other,
            ("always_allow", "Always allow"),
            ""
        ));
        assert!(boolean_skips(
            Category::Other,
            ("disable_confirmations", "Disable confirmations"),
            ""
        ));
        assert!(boolean_skips(Category::Other, ("x", "Ｙｏｌｏ"), ""));
    }

    #[test]
    fn what_is_safe_stays_safe() {
        assert!(!mode("default", "Always Ask", "Ask before every edit"));
        assert!(!mode("plan", "Plan", "Read and plan only; no edits"));
        assert!(!mode("ask", "Ask", "Ask before every edit"));
        assert!(!says_it_skips("autosave", "Autosave", "", true));
        assert!(!boolean_skips(Category::Other, ("verbose", "Verbose"), ""));
        assert!(!boolean_skips(
            Category::Other,
            ("extended", "Extended thinking"),
            ""
        ));
        assert!(!boolean_skips(
            Category::Tuning,
            ("extended", "Extended thinking"),
            "Think harder"
        ));
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

    /// With the policy `forbid` a user must still be able to turn protection ON: the value that grants is the risky one, not the
    /// option. Where the words do not say which, both are risky (never lowers protection).
    #[test]
    fn turning_protection_on_is_never_a_risk_and_turning_it_off_is() {
        let v = |id: &str, name: &str, value: bool| {
            boolean_value_skips(Category::Other, (id, name), "", false, value)
        };
        // Grants when on.
        assert!(v("auto_approve", "Auto approve", true));
        assert!(!v("auto_approve", "Auto approve", false));
        assert!(
            v("always_allow", "Always allow", true) && !v("always_allow", "Always allow", false)
        );
        assert!(v("disable_confirmations", "Disable confirmations", true));
        assert!(!v("disable_confirmations", "Disable confirmations", false));
        // Protects when on.
        assert!(!v("ask_for_approval", "Ask for approval", true));
        assert!(v("ask_for_approval", "Ask for approval", false));
        assert!(!v("sandbox", "Sandbox", true) && v("sandbox", "Sandbox", false));
        // The words do not say: both.
        assert!(v("approvals", "Approvals", true) && v("approvals", "Approvals", false));
        // Not about permissions: neither, unless the adapter flagged it (then the words, else both).
        assert!(!v("verbose", "Verbose", true));
        assert!(boolean_value_skips(
            Category::Other,
            ("verbose", "Verbose"),
            "",
            true,
            true
        ));
        assert!(boolean_value_skips(
            Category::Other,
            ("verbose", "Verbose"),
            "",
            true,
            false
        ));
        assert!(!boolean_value_skips(
            Category::Other,
            ("auto_approve", "Auto approve"),
            "",
            true,
            false
        ));
    }
}
