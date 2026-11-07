//! Notification routing (docs/design/notifications.md): event → rule → channel → delivery. This module is the
//! rule step: given a notice and one person's preferences, which channels get it now, and is it dropped as a
//! repeat or held for a digest. Pure and deterministic (the caller supplies the clock and who is looking);
//! delivery itself (toast, OS notification, push, webhook) happens elsewhere. Text is never built here: a
//! [`Notice`] names a locale key, so it is rendered in the recipient's language.
//!
//! Rules:
//! - In-app is never silenced by quiet hours; the other channels are.
//! - "Only when I am not looking at it": a notice about the thing the person is viewing right now is dropped
//!   for every channel except the in-app badge.
//! - Repeats of the same trigger about the same subject inside the dedupe window are collapsed.
//! - Low-priority triggers do not interrupt: they go to the digest queue.
//! - Approvals still require a signed approval (ADR-0020); a notification only points at them.

use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Trigger {
    AgentBlocked,
    AgentDone,
    ApprovalRequested,
    TaskCompleted,
    NodeOffline,
    BudgetThreshold,
    UpdateAvailable,
}

impl Trigger {
    /// Locale key for the message of this trigger.
    pub fn key(self) -> &'static str {
        match self {
            Self::AgentBlocked => "notify-agent-blocked",
            Self::AgentDone => "notify-agent-done",
            Self::ApprovalRequested => "notify-approval-requested",
            Self::TaskCompleted => "notify-task-completed",
            Self::NodeOffline => "notify-node-offline",
            Self::BudgetThreshold => "notify-budget-threshold",
            Self::UpdateAvailable => "notify-update-available",
        }
    }

    pub fn priority(self) -> Priority {
        match self {
            Self::AgentBlocked | Self::ApprovalRequested | Self::NodeOffline => Priority::High,
            Self::AgentDone | Self::BudgetThreshold => Priority::Normal,
            Self::TaskCompleted | Self::UpdateAvailable => Priority::Low,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Low,
    Normal,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Channel {
    InApp,
    Desktop,
    WebPush,
    Email,
    Webhook,
}

/// What happened, about what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub trigger: Trigger,
    /// The agent, node or task the notice is about; used for dedupe and "am I looking at it".
    pub subject: String,
    pub at_ms: u64,
}

/// A person's preferences. Defaults: every trigger on in-app and desktop; nothing else until enabled.
#[derive(Debug, Clone)]
pub struct Prefs {
    channels: BTreeMap<Trigger, BTreeSet<Channel>>,
    /// Quiet hours as minutes since local midnight, `(start, end)`; may wrap midnight (22:00 → 07:00).
    pub quiet_hours: Option<(u16, u16)>,
    pub only_when_not_looking: bool,
}

const ALL_TRIGGERS: [Trigger; 7] = [
    Trigger::AgentBlocked,
    Trigger::AgentDone,
    Trigger::ApprovalRequested,
    Trigger::TaskCompleted,
    Trigger::NodeOffline,
    Trigger::BudgetThreshold,
    Trigger::UpdateAvailable,
];

impl Default for Prefs {
    fn default() -> Self {
        let on: BTreeSet<Channel> = [Channel::InApp, Channel::Desktop].into();
        Self {
            channels: ALL_TRIGGERS.iter().map(|t| (*t, on.clone())).collect(),
            quiet_hours: None,
            only_when_not_looking: true,
        }
    }
}

impl Prefs {
    pub fn set(&mut self, trigger: Trigger, channel: Channel, enabled: bool) {
        let set = self.channels.entry(trigger).or_default();
        if enabled {
            set.insert(channel);
        } else {
            set.remove(&channel);
        }
    }

    fn quiet(&self, minute_of_day: u16) -> bool {
        match self.quiet_hours {
            None => false,
            Some((s, e)) if s == e => false,
            Some((s, e)) if s < e => (s..e).contains(&minute_of_day),
            Some((s, e)) => minute_of_day >= s || minute_of_day < e, // wraps midnight
        }
    }
}

/// What the router needs to know about the recipient right now.
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// Minutes since local midnight for this recipient.
    pub minute_of_day: u16,
    /// Subjects the person has open on a connected client.
    pub viewing: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Deliver now on these channels (never empty).
    Deliver(Vec<Channel>),
    /// Low priority: held for the digest.
    Digest,
    /// Nothing to do, with the reason (useful in tests and the "why did I not get this" explanation).
    Dropped(Dropped),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dropped {
    Repeat,
    NoChannel,
    Looking,
}

/// Same trigger and subject within this window counts as a repeat.
pub const DEDUPE_MS: u64 = 60_000;

#[derive(Debug, Default)]
pub struct Router {
    last: BTreeMap<(Trigger, String), u64>,
    digest: u32,
}

impl Router {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn route(&mut self, prefs: &Prefs, ctx: &Context, n: &Notice) -> Outcome {
        let key = (n.trigger, n.subject.clone());
        if let Some(&prev) = self.last.get(&key) {
            if n.at_ms.saturating_sub(prev) < DEDUPE_MS {
                return Outcome::Dropped(Dropped::Repeat);
            }
        }
        // Remember the notice even when it is dropped below, so a flapping state does not re-notify.
        self.last.insert(key, n.at_ms);

        let mut channels: Vec<Channel> = prefs
            .channels
            .get(&n.trigger)
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default();
        if channels.is_empty() {
            return Outcome::Dropped(Dropped::NoChannel);
        }
        let looking = prefs.only_when_not_looking && ctx.viewing.contains(&n.subject);
        if looking {
            // The window already shows it; the badge may still count it, everything else would be noise.
            channels.retain(|c| *c == Channel::InApp);
            return if channels.is_empty() {
                Outcome::Dropped(Dropped::Looking)
            } else {
                Outcome::Deliver(channels)
            };
        }
        if prefs.quiet(ctx.minute_of_day) {
            channels.retain(|c| *c == Channel::InApp);
            if channels.is_empty() {
                return Outcome::Digest.counted(self);
            }
        }
        if n.trigger.priority() == Priority::Low {
            return Outcome::Digest.counted(self);
        }
        Outcome::Deliver(channels)
    }

    /// Takes the digest count (for `notify-digest`), resetting it. `None` when there is nothing to report.
    pub fn flush_digest(&mut self) -> Option<u32> {
        (self.digest > 0).then(|| std::mem::take(&mut self.digest))
    }

    /// Forget dedupe entries older than the window so the table does not grow without bound.
    pub fn prune(&mut self, now_ms: u64) {
        self.last
            .retain(|_, at| now_ms.saturating_sub(*at) < DEDUPE_MS);
    }
}

impl Outcome {
    fn counted(self, r: &mut Router) -> Self {
        if self == Outcome::Digest {
            r.digest += 1;
        }
        self
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn notice(t: Trigger, subject: &str, at: u64) -> Notice {
        Notice {
            trigger: t,
            subject: subject.into(),
            at_ms: at,
        }
    }

    fn ctx() -> Context {
        Context {
            minute_of_day: 12 * 60,
            viewing: BTreeSet::new(),
        }
    }

    #[test]
    fn default_prefs_deliver_in_app_and_desktop() {
        let mut r = Router::new();
        assert_eq!(
            r.route(
                &Prefs::default(),
                &ctx(),
                &notice(Trigger::AgentBlocked, "a", 0)
            ),
            Outcome::Deliver(vec![Channel::InApp, Channel::Desktop])
        );
    }

    #[test]
    fn repeats_are_collapsed_until_the_window_passes() {
        let (mut r, p) = (Router::new(), Prefs::default());
        assert!(matches!(
            r.route(&p, &ctx(), &notice(Trigger::AgentBlocked, "a", 0)),
            Outcome::Deliver(_)
        ));
        assert_eq!(
            r.route(&p, &ctx(), &notice(Trigger::AgentBlocked, "a", 30_000)),
            Outcome::Dropped(Dropped::Repeat)
        );
        // A different agent, or a different trigger, is not a repeat.
        assert!(matches!(
            r.route(&p, &ctx(), &notice(Trigger::AgentBlocked, "b", 30_000)),
            Outcome::Deliver(_)
        ));
        assert!(matches!(
            r.route(&p, &ctx(), &notice(Trigger::AgentDone, "a", 30_000)),
            Outcome::Deliver(_)
        ));
        assert!(matches!(
            r.route(&p, &ctx(), &notice(Trigger::AgentBlocked, "a", DEDUPE_MS)),
            Outcome::Deliver(_)
        ));
    }

    #[test]
    fn disabled_channels_and_empty_sets_drop() {
        let mut p = Prefs::default();
        p.set(Trigger::AgentDone, Channel::Desktop, false);
        let mut r = Router::new();
        assert_eq!(
            r.route(&p, &ctx(), &notice(Trigger::AgentDone, "a", 0)),
            Outcome::Deliver(vec![Channel::InApp])
        );
        p.set(Trigger::NodeOffline, Channel::InApp, false);
        p.set(Trigger::NodeOffline, Channel::Desktop, false);
        assert_eq!(
            r.route(&p, &ctx(), &notice(Trigger::NodeOffline, "n", 0)),
            Outcome::Dropped(Dropped::NoChannel)
        );
        p.set(Trigger::NodeOffline, Channel::Webhook, true);
        assert_eq!(
            r.route(&p, &ctx(), &notice(Trigger::NodeOffline, "n", DEDUPE_MS)),
            Outcome::Deliver(vec![Channel::Webhook])
        );
    }

    #[test]
    fn quiet_hours_keep_only_the_badge_and_wrap_midnight() {
        let mut p = Prefs {
            quiet_hours: Some((22 * 60, 7 * 60)),
            ..Prefs::default()
        };
        let mut r = Router::new();
        let mut c = ctx();
        c.minute_of_day = 23 * 60;
        assert_eq!(
            r.route(&p, &c, &notice(Trigger::ApprovalRequested, "a", 0)),
            Outcome::Deliver(vec![Channel::InApp])
        );
        c.minute_of_day = 6 * 60 + 59;
        assert_eq!(
            r.route(&p, &c, &notice(Trigger::ApprovalRequested, "b", 0)),
            Outcome::Deliver(vec![Channel::InApp])
        );
        c.minute_of_day = 7 * 60;
        assert_eq!(
            r.route(&p, &c, &notice(Trigger::ApprovalRequested, "c", 0)),
            Outcome::Deliver(vec![Channel::InApp, Channel::Desktop])
        );
        // Quiet hours with nothing in-app go to the digest rather than being lost.
        p.set(Trigger::AgentDone, Channel::InApp, false);
        c.minute_of_day = 23 * 60;
        assert_eq!(
            r.route(&p, &c, &notice(Trigger::AgentDone, "d", 0)),
            Outcome::Digest
        );
    }

    #[test]
    fn looking_at_the_subject_silences_everything_but_the_badge() {
        let (mut r, p) = (Router::new(), Prefs::default());
        let mut c = ctx();
        c.viewing.insert("a".into());
        assert_eq!(
            r.route(&p, &c, &notice(Trigger::AgentBlocked, "a", 0)),
            Outcome::Deliver(vec![Channel::InApp])
        );
        let mut p2 = Prefs::default();
        p2.set(Trigger::AgentDone, Channel::InApp, false);
        assert_eq!(
            r.route(&p2, &c, &notice(Trigger::AgentDone, "a", 0)),
            Outcome::Dropped(Dropped::Looking)
        );
        // The setting can be turned off.
        let p3 = Prefs {
            only_when_not_looking: false,
            ..Prefs::default()
        };
        assert_eq!(
            r.route(&p3, &c, &notice(Trigger::NodeOffline, "a", 0)),
            Outcome::Deliver(vec![Channel::InApp, Channel::Desktop])
        );
    }

    #[test]
    fn low_priority_goes_to_a_digest_that_flushes_once() {
        let (mut r, p) = (Router::new(), Prefs::default());
        assert_eq!(
            r.route(&p, &ctx(), &notice(Trigger::TaskCompleted, "t1", 0)),
            Outcome::Digest
        );
        assert_eq!(
            r.route(&p, &ctx(), &notice(Trigger::UpdateAvailable, "v2", 0)),
            Outcome::Digest
        );
        assert_eq!(r.flush_digest(), Some(2));
        assert_eq!(r.flush_digest(), None);
    }

    #[test]
    fn prune_forgets_old_entries_only() {
        let (mut r, p) = (Router::new(), Prefs::default());
        r.route(&p, &ctx(), &notice(Trigger::AgentBlocked, "old", 0));
        r.route(&p, &ctx(), &notice(Trigger::AgentBlocked, "new", DEDUPE_MS));
        r.prune(DEDUPE_MS + 1);
        assert_eq!(r.last.len(), 1);
    }

    #[test]
    fn every_trigger_has_a_locale_key_that_exists() {
        let cat = agora_i18n::Catalog::english();
        for t in ALL_TRIGGERS {
            assert!(cat.contains(t.key()), "{}", t.key());
        }
        assert!(cat.contains("notify-digest"));
    }
}
