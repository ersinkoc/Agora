//! Usage reports merged before they are logged (architecture review R4). An agent reports its usage after every turn, and a terminal
//! agent's usage file grows a line per model call: one busy agent wrote 11 751 `AgentUsageReported` events in 26 hours, 96% of the
//! hub's whole log, and the hub replays its whole log when it starts. Reports of one agent, model and currency that arrive within
//! [`WINDOW_MS`] are added up and logged as one: the token and cost counts are growths, so their sum is exact; the context level is a
//! level, so the latest one is kept. Pure state, no clock of its own (the caller passes `now_ms`).

use agora_protocol::v1::AgentUsageReported;

/// How long reports are gathered before one is logged. Short enough that the window still shows usage as it happens.
pub const WINDOW_MS: u64 = 5_000;
/// At most this many gatherings at once (agents x models x currencies); a report past it is logged as it is.
pub const MAX_PENDING: usize = 256;

struct Pending {
    since_ms: u64,
    usage: AgentUsageReported,
}

#[derive(Default)]
pub struct UsageMerge {
    pending: Vec<Pending>,
}

fn same(a: &AgentUsageReported, b: &AgentUsageReported) -> bool {
    a.agent_id == b.agent_id && a.model == b.model && a.currency == b.currency
}

impl UsageMerge {
    /// Gathers a report. Returns it back when it cannot be held (too many gatherings): the caller logs it at once.
    pub fn add(&mut self, now_ms: u64, u: AgentUsageReported) -> Option<AgentUsageReported> {
        if let Some(p) = self.pending.iter_mut().find(|p| same(&p.usage, &u)) {
            let s = &mut p.usage;
            s.input_tokens = s.input_tokens.saturating_add(u.input_tokens);
            s.output_tokens = s.output_tokens.saturating_add(u.output_tokens);
            s.cost_micros = match (s.cost_micros, u.cost_micros) {
                (Some(a), Some(b)) => Some(a.saturating_add(b)),
                (a, b) => a.or(b),
            };
            // A level, not a growth: the latest report that says it wins; 0 means "not reported".
            if u.context_tokens > 0 || u.context_window > 0 {
                s.context_tokens = u.context_tokens;
                s.context_window = u.context_window;
            }
            return None;
        }
        if self.pending.len() >= MAX_PENDING {
            return Some(u);
        }
        self.pending.push(Pending {
            since_ms: now_ms,
            usage: u,
        });
        None
    }

    /// The gatherings whose window has passed, oldest first.
    pub fn due(&mut self, now_ms: u64) -> Vec<AgentUsageReported> {
        self.take(|p| now_ms >= p.since_ms.saturating_add(WINDOW_MS))
    }

    /// Everything gathered for `agent` (it ended: its usage is logged before its end).
    pub fn take_agent(&mut self, agent: &str) -> Vec<AgentUsageReported> {
        self.take(|p| p.usage.agent_id == agent)
    }

    /// Everything gathered (the node stops).
    pub fn take_all(&mut self) -> Vec<AgentUsageReported> {
        self.take(|_| true)
    }

    fn take(&mut self, pick: impl Fn(&Pending) -> bool) -> Vec<AgentUsageReported> {
        let (out, keep): (Vec<Pending>, Vec<Pending>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|p| pick(p));
        self.pending = keep;
        out.into_iter().map(|p| p.usage).collect()
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn u(agent: &str, model: &str, input: u64, output: u64) -> AgentUsageReported {
        AgentUsageReported {
            agent_id: agent.into(),
            model: model.into(),
            input_tokens: input,
            output_tokens: output,
            ..Default::default()
        }
    }

    #[test]
    fn reports_in_one_window_become_one_with_the_exact_sums() {
        let mut m = UsageMerge::default();
        assert!(m.add(0, u("a1", "m", 10, 1)).is_none());
        assert!(m.add(1_000, u("a1", "m", 20, 2)).is_none());
        let mut with_cost = u("a1", "m", 5, 5);
        with_cost.cost_micros = Some(700);
        with_cost.currency = String::new();
        with_cost.context_tokens = 900;
        with_cost.context_window = 200_000;
        assert!(m.add(2_000, with_cost).is_none());
        assert!(m.add(3_000, u("a1", "m", 1, 1)).is_none()); // says no context level: the last one stays
        assert!(m.due(4_999).is_empty());
        let out = m.due(5_000);
        assert_eq!(out.len(), 1);
        let r = &out[0];
        assert_eq!((r.input_tokens, r.output_tokens), (36, 9));
        assert_eq!(r.cost_micros, Some(700));
        assert_eq!((r.context_tokens, r.context_window), (900, 200_000));
        assert!(m.due(100_000).is_empty());
    }

    #[test]
    fn agents_models_and_currencies_are_kept_apart() {
        let mut m = UsageMerge::default();
        m.add(0, u("a1", "m", 1, 0));
        m.add(0, u("a1", "other", 2, 0));
        m.add(0, u("a2", "m", 3, 0));
        let mut eur = u("a1", "m", 4, 0);
        eur.currency = "EUR".into();
        eur.cost_micros = Some(1);
        m.add(0, eur);
        let out = m.due(WINDOW_MS);
        assert_eq!(out.len(), 4);
        assert_eq!(out.iter().map(|r| r.input_tokens).sum::<u64>(), 10);
    }

    #[test]
    fn an_agent_that_ends_and_a_node_that_stops_lose_nothing() {
        let mut m = UsageMerge::default();
        m.add(0, u("a1", "m", 1, 0));
        m.add(0, u("a2", "m", 2, 0));
        let a1 = m.take_agent("a1");
        assert_eq!((a1.len(), a1[0].input_tokens), (1, 1));
        let rest = m.take_all();
        assert_eq!((rest.len(), rest[0].agent_id.as_str()), (1, "a2"));
        assert!(m.take_all().is_empty());
    }

    #[test]
    fn past_the_limit_a_report_is_given_back_to_log_at_once() {
        let mut m = UsageMerge::default();
        for i in 0..MAX_PENDING {
            assert!(m.add(0, u(&format!("a{i}"), "m", 1, 0)).is_none());
        }
        let back = m.add(0, u("one-more", "m", 7, 0)).expect("given back");
        assert_eq!(back.input_tokens, 7);
        // An agent already gathering still merges.
        assert!(m.add(0, u("a0", "m", 1, 0)).is_none());
    }
}
