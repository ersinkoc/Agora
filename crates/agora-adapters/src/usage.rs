//! Token usage read from the file an agent writes itself (ADR-0042). Pure: a [`FileReader`] is given one line at a time and says
//! what usage the line adds; finding and tailing files is the node's job (`agora-node::usage_files`).

use std::collections::{HashSet, VecDeque};

use serde_json::Value;

use crate::manifest::{QuotaSpec, UsageFile, UsageMode};

/// Lines longer than this are skipped (ADR-0042).
pub const MAX_LINE: usize = 256 * 1024;
/// Message ids remembered for de-duplication.
const SEEN: usize = 4096;

/// What one line adds to an agent's usage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sample {
    pub model: String,
    pub input: u64,
    pub output: u64,
}

/// A plan window may reset at most this long ago (a day) and at most this far ahead (31 days): anything else is dropped (ADR-0091).
pub const QUOTA_MAX_PAST_MS: u64 = 24 * 60 * 60 * 1000;
pub const QUOTA_MAX_AHEAD_MS: u64 = 31 * 24 * 60 * 60 * 1000;
/// Window lengths above this (a year in minutes) are not a plan window.
const QUOTA_MAX_MINUTES: u64 = 525_600;
/// Raw windows kept between two reads of the reader's output.
const QUOTA_HELD: usize = 16;
/// A reset time of this size or more is milliseconds, below it seconds (a second count reaches it in the year 5138).
const MILLIS_FROM: f64 = 100_000_000_000.0;

/// ADR-0091: one plan window as a selected line says it, before the node bounds it. Only these three numbers are ever taken from a
/// line for a limit (a strict allow-list by construction: there is no field for anything else, and the values are numbers, never
/// text), so a credit balance, a plan name or an account id in the same line cannot reach an event.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuotaRaw {
    pub percent: f64,
    pub minutes: u64,
    /// Milliseconds since the Unix epoch.
    pub resets_ms: u64,
    /// The line's own time (`[quota] at`), milliseconds since the Unix epoch, when the line has a readable one. The node uses it to
    /// stamp what it replays from a file it reads from the start; it is never taken as later than the node's clock.
    pub line_ms: Option<u64>,
}

/// A bounded observation: what the node may send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotaSample {
    /// `five_hour`, `seven_day` or `minutes:<n>`.
    pub window: String,
    /// 0 to 100.
    pub used_percent: u32,
    pub resets_at_ms: u64,
}

/// The label of a window of this length, from the length alone (never from a field name).
pub fn window_label(minutes: u64) -> String {
    match minutes {
        300 => "five_hour".into(),
        10_080 => "seven_day".into(),
        n => format!("minutes:{n}"),
    }
}

impl QuotaRaw {
    /// Clamps the percentage to 0..=100 and bounds the reset time against `now_ms`; `None` when the time is more than a day in the past
    /// or more than 31 days ahead (the observation is dropped: unknown, not guessed at).
    pub fn bound(&self, now_ms: u64) -> Option<QuotaSample> {
        if !self.percent.is_finite()
            || self.resets_ms < now_ms.saturating_sub(QUOTA_MAX_PAST_MS)
            || self.resets_ms > now_ms.saturating_add(QUOTA_MAX_AHEAD_MS)
        {
            return None;
        }
        Some(QuotaSample {
            window: window_label(self.minutes),
            // Clamped first, so the cast cannot overflow or wrap.
            used_percent: self.percent.clamp(0.0, 100.0).round() as u32,
            resets_at_ms: self.resets_ms,
        })
    }
}

/// An RFC 3339 time (`2026-11-20T09:00:05.123Z`, or with a `+hh:mm` offset) as milliseconds since the Unix epoch. Anything else, or a
/// year outside 1970..=2200, is `None`.
pub fn parse_rfc3339_ms(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    if b.len() < 20 || b.len() > 40 || !s.is_ascii() {
        return None;
    }
    let num = |from: usize, to: usize| -> Option<i64> {
        let t = s.get(from..to)?;
        t.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| t.parse().ok())?
    };
    if b[4] != b'-'
        || b[7] != b'-'
        || !matches!(b[10], b'T' | b't' | b' ')
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let (y, mo, d, h, mi, sec) = (
        num(0, 4)?,
        num(5, 7)?,
        num(8, 10)?,
        num(11, 13)?,
        num(14, 16)?,
        num(17, 19)?,
    );
    if !(1970..=2200).contains(&y)
        || !(1..=12).contains(&mo)
        || !(1..=31).contains(&d)
        || h > 23
        || mi > 59
        || sec > 60
    {
        return None;
    }
    let days_in_month = match mo {
        2 if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if d > days_in_month {
        return None;
    }
    let mut i = 19;
    let mut frac_ms = 0i64;
    if b[i] == b'.' {
        let start = i + 1;
        i = start;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return None;
        }
        let digits: String = s[start..i].chars().take(3).collect();
        frac_ms = format!("{digits:0<3}").parse().ok()?;
    }
    let offset_s = match s.get(i..)? {
        "Z" | "z" => 0,
        tz if tz.len() == 6
            && (tz.starts_with('+') || tz.starts_with('-'))
            && tz.as_bytes()[3] == b':' =>
        {
            let (oh, om) = (num(i + 1, i + 3)?, num(i + 4, i + 6)?);
            if oh > 23 || om > 59 {
                return None;
            }
            let secs = oh * 3600 + om * 60;
            if tz.starts_with('+') {
                secs
            } else {
                -secs
            }
        }
        _ => return None,
    };
    // Days since 1970-01-01 (proleptic Gregorian, after Howard Hinnant's `days_from_civil`).
    let (yy, mm) = if mo <= 2 {
        (y - 1, mo + 9)
    } else {
        (y, mo - 3)
    };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let doy = (153 * mm + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + h * 3600 + mi * 60 + sec - offset_s;
    u64::try_from(secs)
        .ok()?
        .checked_mul(1000)?
        .checked_add(frac_ms as u64)
}

fn float(v: &Value, path: &str) -> Option<f64> {
    at(v, path)?.as_f64().filter(|f| f.is_finite())
}

/// The plan windows one selected line carries, by the manifest's paths. A window with a missing, non-numeric or impossible value (a
/// length of 0 or over a year, a reset time that is not positive) is left out.
fn quota_windows(v: &Value, spec: &QuotaSpec) -> Vec<QuotaRaw> {
    let line_ms = spec
        .at
        .as_deref()
        .and_then(|p| at(v, p)?.as_str())
        .and_then(parse_rfc3339_ms);
    let mut out = Vec::new();
    for w in &spec.window {
        let (Some(percent), Some(minutes), Some(resets)) = (
            float(v, &w.percent),
            float(v, &w.minutes),
            float(v, &w.resets),
        ) else {
            continue;
        };
        if minutes < 1.0
            || minutes > QUOTA_MAX_MINUTES as f64
            || minutes.fract() != 0.0
            || resets <= 0.0
        {
            continue;
        }
        let resets_ms = if resets >= MILLIS_FROM {
            resets
        } else {
            resets * 1000.0
        };
        // `as` saturates, and the value is positive and finite.
        out.push(QuotaRaw {
            percent,
            minutes: minutes as u64,
            resets_ms: resets_ms as u64,
            line_ms,
        });
    }
    out
}

fn at<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(v, |v, key| v.get(key))
}

fn number(v: &Value, path: &str) -> Option<u64> {
    at(v, path)?.as_u64()
}

fn text(v: &Value, path: &str) -> Option<String> {
    match at(v, path)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Reads the lines of one usage file.
pub struct FileReader {
    spec: UsageFile,
    model: String,
    cwd: Option<String>,
    last: (u64, u64),
    quota: Option<QuotaSpec>,
    held: Vec<QuotaRaw>,
    seen: HashSet<String>,
    order: VecDeque<String>,
}

impl FileReader {
    pub fn new(spec: UsageFile) -> Self {
        Self {
            spec,
            model: String::new(),
            cwd: None,
            last: (0, 0),
            quota: None,
            held: Vec::new(),
            seen: HashSet::new(),
            order: VecDeque::new(),
        }
    }

    /// ADR-0091: also reads the plan windows of selected lines, by the manifest's `[quota]`.
    #[must_use]
    pub fn with_quota(mut self, quota: Option<QuotaSpec>) -> Self {
        self.quota = quota;
        self
    }

    /// The plan windows of the lines read since the last call, oldest first (at most 16 are kept: the newest).
    pub fn take_quota(&mut self) -> Vec<QuotaRaw> {
        std::mem::take(&mut self.held)
    }

    /// The working directory the file records, once a line has said it.
    pub fn cwd(&self) -> Option<&str> {
        self.cwd.as_deref()
    }

    /// What this line adds, if it is a usage line. A line that is too long, does not parse, is not selected or lacks the numbers
    /// adds nothing: never guessed at.
    pub fn line(&mut self, line: &str) -> Option<Sample> {
        if line.len() > MAX_LINE {
            return None;
        }
        let v: Value = serde_json::from_str(line).ok()?;
        if self.cwd.is_none() {
            self.cwd = text(&v, &self.spec.cwd);
        }
        if let Some(m) = text(&v, &self.spec.model) {
            self.model = m;
        }
        if let Some(sel) = &self.spec.select {
            if text(&v, &sel.path).as_deref() != Some(sel.equals.as_str()) {
                return None;
            }
        }
        // ADR-0091: a line is a limit report whether or not it adds tokens.
        if let Some(q) = &self.quota {
            self.held.extend(quota_windows(&v, q));
            if self.held.len() > QUOTA_HELD {
                self.held.drain(..self.held.len() - QUOTA_HELD);
            }
        }
        let mut input = number(&v, &self.spec.input)?;
        let output = number(&v, &self.spec.output)?;
        match self.spec.mode {
            UsageMode::Delta => {
                if let Some(id) = self.spec.dedupe.as_deref().and_then(|p| text(&v, p)) {
                    if !self.seen.insert(id.clone()) {
                        return None;
                    }
                    self.order.push_back(id);
                    while self.order.len() > SEEN {
                        if let Some(old) = self.order.pop_front() {
                            self.seen.remove(&old);
                        }
                    }
                }
                for extra in [&self.spec.cache_read, &self.spec.cache_write]
                    .into_iter()
                    .flatten()
                {
                    input = input.saturating_add(number(&v, extra).unwrap_or(0));
                }
                Some(self.sample(input, output))
            }
            UsageMode::Total => {
                let (din, dout) = (
                    input.saturating_sub(self.last.0),
                    output.saturating_sub(self.last.1),
                );
                self.last = (input, output);
                (din != 0 || dout != 0).then(|| self.sample(din, dout))
            }
        }
    }

    fn sample(&self, input: u64, output: u64) -> Sample {
        Sample {
            model: self.model.clone(),
            input,
            output,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::manifest::UsageSelect;

    fn claude() -> UsageFile {
        UsageFile {
            glob: "{home}/.claude/projects/*/*.jsonl".into(),
            format: Some("jsonl".into()),
            select: Some(UsageSelect {
                path: "type".into(),
                equals: "assistant".into(),
            }),
            cwd: "cwd".into(),
            model: "message.model".into(),
            input: "message.usage.input_tokens".into(),
            output: "message.usage.output_tokens".into(),
            cache_read: Some("message.usage.cache_read_input_tokens".into()),
            cache_write: Some("message.usage.cache_creation_input_tokens".into()),
            dedupe: Some("message.id".into()),
            mode: UsageMode::Delta,
        }
    }

    fn codex() -> UsageFile {
        UsageFile {
            glob: "{home}/.codex/sessions/*/*/*/*.jsonl".into(),
            format: Some("jsonl".into()),
            select: Some(UsageSelect {
                path: "payload.type".into(),
                equals: "token_count".into(),
            }),
            cwd: "payload.cwd".into(),
            model: "payload.model".into(),
            input: "payload.info.total_token_usage.input_tokens".into(),
            output: "payload.info.total_token_usage.output_tokens".into(),
            cache_read: None,
            cache_write: None,
            dedupe: None,
            mode: UsageMode::Total,
        }
    }

    const A: &str = r#"{"type":"assistant","cwd":"/w","message":{"id":"m1","model":"m-a","usage":{"input_tokens":2,"output_tokens":10,"cache_read_input_tokens":100,"cache_creation_input_tokens":30}}}"#;

    #[test]
    fn a_delta_line_adds_its_prompt_side_tokens_and_a_repeated_message_counts_once() {
        let mut r = FileReader::new(claude());
        let s = r.line(A).unwrap();
        assert_eq!(
            s,
            Sample {
                model: "m-a".into(),
                input: 132,
                output: 10
            }
        );
        assert_eq!(r.cwd(), Some("/w"));
        assert_eq!(r.line(A), None, "the same message id again");
        let b = A.replace("\"m1\"", "\"m2\"");
        assert_eq!(r.line(&b).unwrap().output, 10);
    }

    #[test]
    fn lines_that_are_not_usage_are_skipped_never_guessed_at() {
        let mut r = FileReader::new(claude());
        assert_eq!(r.line("not json"), None);
        assert_eq!(r.line(r#"{"type":"user","cwd":"/w"}"#), None);
        assert_eq!(
            r.line(r#"{"type":"assistant","message":{"id":"x","usage":{"input_tokens":"two"}}}"#),
            None
        );
        assert_eq!(r.line(&"x".repeat(MAX_LINE + 1)), None);
        assert_eq!(
            r.cwd(),
            Some("/w"),
            "a skipped line can still say where the file belongs"
        );
    }

    #[test]
    fn a_total_line_reports_the_growth_and_takes_the_model_from_an_earlier_line() {
        let mut r = FileReader::new(codex());
        assert_eq!(
            r.line(r#"{"type":"turn_context","payload":{"cwd":"/w","model":"gpt-x"}}"#),
            None
        );
        let t = |i, o| {
            format!(
                r#"{{"type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":{i},"output_tokens":{o}}}}}}}}}"#
            )
        };
        let s = r.line(&t(100, 5)).unwrap();
        assert_eq!((s.model.as_str(), s.input, s.output), ("gpt-x", 100, 5));
        let s = r.line(&t(160, 9)).unwrap();
        assert_eq!((s.input, s.output), (60, 4));
        assert_eq!(r.line(&t(160, 9)), None, "no growth, nothing to report");
        // A total that went down (a reset) is a new baseline, not a negative use.
        assert_eq!(r.line(&t(10, 1)), None);
        let s = r.line(&t(15, 2)).unwrap();
        assert_eq!((s.input, s.output), (5, 1));
        assert_eq!(r.cwd(), Some("/w"));
    }

    // ---- ADR-0091: plan limits ----

    fn codex_quota() -> QuotaSpec {
        let window = |k: &str| crate::manifest::QuotaWindow {
            percent: format!("payload.rate_limits.{k}.used_percent"),
            minutes: format!("payload.rate_limits.{k}.window_minutes"),
            resets: format!("payload.rate_limits.{k}.resets_at"),
        };
        QuotaSpec {
            at: Some("timestamp".into()),
            window: vec![window("primary"), window("secondary")],
        }
    }

    fn quota_reader() -> FileReader {
        FileReader::new(codex()).with_quota(Some(codex_quota()))
    }

    fn limits(rate_limits: &str) -> String {
        format!(
            r#"{{"type":"event_msg","payload":{{"type":"token_count","info":null,"rate_limits":{rate_limits}}}}}"#
        )
    }

    const BOTH: &str = r#"{"primary":{"used_percent":17.0,"window_minutes":300,"resets_at":1795000000},"secondary":{"used_percent":4,"window_minutes":10080,"resets_at":1795400000}}"#;

    #[test]
    fn a_codex_line_with_and_without_the_secondary_window_gives_its_windows() {
        let mut r = quota_reader();
        // A line without token info still reports the limits.
        assert_eq!(r.line(&limits(BOTH)), None);
        let both = r.take_quota();
        assert_eq!(
            both,
            [
                QuotaRaw {
                    percent: 17.0,
                    minutes: 300,
                    resets_ms: 1_795_000_000_000,
                    line_ms: None,
                },
                QuotaRaw {
                    percent: 4.0,
                    minutes: 10_080,
                    resets_ms: 1_795_400_000_000,
                    line_ms: None,
                },
            ]
        );
        assert!(r.take_quota().is_empty(), "taken once");
        for only_primary in [
            r#"{"primary":{"used_percent":18.5,"window_minutes":300,"resets_at":1795000000}}"#,
            r#"{"primary":{"used_percent":18.5,"window_minutes":300,"resets_at":1795000000},"secondary":null}"#,
        ] {
            r.line(&limits(only_primary));
            assert_eq!(r.take_quota().len(), 1, "{only_primary}");
        }
        // A reader without a `[quota]` mapping reads none.
        let mut plain = FileReader::new(codex());
        plain.line(&limits(BOTH));
        assert!(plain.take_quota().is_empty());
        // Only selected lines count.
        r.line(&limits(BOTH).replace("token_count", "agent_message"));
        assert!(r.take_quota().is_empty());
    }

    #[test]
    fn the_window_is_named_from_its_length_never_from_its_field_name() {
        assert_eq!(window_label(300), "five_hour");
        assert_eq!(window_label(10_080), "seven_day");
        assert_eq!(window_label(60), "minutes:60");
        assert_eq!(window_label(301), "minutes:301");
        // The "primary" window may be the weekly one (a plan without the short window).
        let mut r = quota_reader();
        r.line(&limits(
            r#"{"primary":{"used_percent":1,"window_minutes":10080,"resets_at":1795400000},"secondary":{"used_percent":2,"window_minutes":300,"resets_at":1795000000}}"#,
        ));
        let q = r.take_quota();
        assert_eq!(window_label(q[0].minutes), "seven_day");
        assert_eq!(window_label(q[1].minutes), "five_hour");
    }

    #[test]
    fn malformed_missing_or_odd_fields_mean_no_window() {
        let mut r = quota_reader();
        for bad in [
            "null",
            "[]",
            r#""x""#,
            "{}",
            r#"{"primary":{}}"#,
            r#"{"primary":{"used_percent":"12","window_minutes":300,"resets_at":1795000000}}"#,
            r#"{"primary":{"used_percent":12,"window_minutes":"300","resets_at":1795000000}}"#,
            r#"{"primary":{"used_percent":12,"window_minutes":300,"resets_at":"1795000000"}}"#,
            r#"{"primary":{"used_percent":12,"window_minutes":300}}"#,
            r#"{"primary":{"used_percent":12,"window_minutes":0,"resets_at":1795000000}}"#,
            r#"{"primary":{"used_percent":12,"window_minutes":-5,"resets_at":1795000000}}"#,
            r#"{"primary":{"used_percent":12,"window_minutes":2.5,"resets_at":1795000000}}"#,
            r#"{"primary":{"used_percent":12,"window_minutes":999999999,"resets_at":1795000000}}"#,
            r#"{"primary":{"used_percent":12,"window_minutes":300,"resets_at":0}}"#,
            r#"{"primary":{"used_percent":12,"window_minutes":300,"resets_at":-1}}"#,
            r#"{"primary":{"used_percent":null,"window_minutes":300,"resets_at":1795000000}}"#,
        ] {
            r.line(&limits(bad));
            assert!(r.take_quota().is_empty(), "{bad}");
        }
        r.line("not json");
        r.line(&"x".repeat(MAX_LINE + 1));
        assert!(r.take_quota().is_empty());
    }

    #[test]
    fn a_reset_time_in_seconds_or_milliseconds_is_taken_as_milliseconds() {
        let mut r = quota_reader();
        for (given, ms) in [
            ("1795000000", 1_795_000_000_000u64),
            ("1795000000000", 1_795_000_000_000),
            ("1795000000.5", 1_795_000_000_500),
            ("99999999999", 99_999_999_999_000),
            ("100000000000", 100_000_000_000),
        ] {
            r.line(&limits(&format!(
                r#"{{"primary":{{"used_percent":1,"window_minutes":300,"resets_at":{given}}}}}"#
            )));
            assert_eq!(r.take_quota()[0].resets_ms, ms, "{given}");
        }
    }

    #[test]
    fn hostile_values_are_clamped_or_dropped_when_bounded() {
        let now = 1_795_000_000_000u64;
        let day = 24 * 60 * 60 * 1000;
        let raw = |percent: f64, resets_ms: u64| QuotaRaw {
            percent,
            minutes: 300,
            resets_ms,
            line_ms: None,
        };
        // The percentage is clamped, not trusted.
        assert_eq!(raw(1e30, now).bound(now).unwrap().used_percent, 100);
        assert_eq!(raw(150.0, now).bound(now).unwrap().used_percent, 100);
        assert_eq!(raw(-3.0, now).bound(now).unwrap().used_percent, 0);
        assert_eq!(raw(f64::NEG_INFINITY, now).bound(now), None);
        assert_eq!(raw(f64::NAN, now).bound(now), None);
        assert_eq!(raw(12.4, now).bound(now).unwrap().used_percent, 12);
        assert_eq!(raw(12.5, now).bound(now).unwrap().used_percent, 13);
        // The reset time: not more than a day ago, not more than 31 days ahead.
        assert!(raw(1.0, now - day).bound(now).is_some());
        assert_eq!(raw(1.0, now - day - 1).bound(now), None);
        assert!(raw(1.0, now + 31 * day).bound(now).is_some());
        assert_eq!(raw(1.0, now + 31 * day + 1).bound(now), None);
        assert_eq!(raw(1.0, u64::MAX).bound(now), None);
        assert_eq!(raw(1.0, 1).bound(now), None);
        // A huge number in the file saturates and is then out of bounds, never a wrapped small value.
        let mut r = quota_reader();
        r.line(&limits(
            r#"{"primary":{"used_percent":1,"window_minutes":300,"resets_at":1e300}}"#,
        ));
        let q = r.take_quota();
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].bound(now), None);
    }

    /// The record has no field for anything but the window, the percentage and the reset time: a line full of account data gives
    /// numbers only, whatever the keys are called.
    #[test]
    fn nothing_but_the_allow_listed_numbers_is_taken_from_a_line() {
        let line = limits(
            r#"{"primary":{"used_percent":17.0,"window_minutes":300,"resets_at":1795000000,"label":"PLAN-LABEL","account_id":"ACCT-1"},"secondary":null,"credits":{"has_credits":true,"balance":"BAL-99"},"plan_type":"PLUS-X","account":{"email":"someone@example.com"},"token":"TOK-SECRET"}"#,
        );
        let mut r = quota_reader();
        r.line(&line);
        let q = r.take_quota();
        assert_eq!(q.len(), 1);
        let shown = format!("{:?}", q[0].bound(1_795_000_000_000).unwrap());
        for never in [
            "PLAN-LABEL",
            "ACCT-1",
            "BAL-99",
            "PLUS-X",
            "someone",
            "TOK-SECRET",
        ] {
            assert!(!shown.contains(never), "{never}");
        }
        assert_eq!(
            shown,
            r#"QuotaSample { window: "five_hour", used_percent: 17, resets_at_ms: 1795000000000 }"#
        );
    }

    #[test]
    fn a_long_history_keeps_the_newest_windows() {
        let mut r = quota_reader();
        for i in 0..40 {
            r.line(&limits(&format!(
                r#"{{"primary":{{"used_percent":{i},"window_minutes":300,"resets_at":1795000000}}}}"#
            )));
        }
        let q = r.take_quota();
        assert_eq!(q.len(), QUOTA_HELD);
        assert_eq!(q.last().unwrap().percent, 39.0);
    }

    #[test]
    fn a_lines_own_time_is_read_defensively() {
        assert_eq!(
            parse_rfc3339_ms("2026-11-20T09:00:05.000Z"),
            Some(1_795_165_205_000)
        );
        assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_rfc3339_ms("2026-11-20T11:00:05.5+02:00"),
            Some(1_795_165_205_500)
        );
        assert_eq!(
            parse_rfc3339_ms("2026-11-20T04:00:05-05:00"),
            Some(1_795_165_205_000)
        );
        for bad in [
            "",
            "yesterday",
            "2026-11-20",
            "2026-13-20T09:00:05Z",
            "2026-11-20T25:00:05Z",
            "2026-11-20T09:00:05",
            "2026-11-20T09:00:05+2:00",
            "2026-11-20T09:00:05.Z",
            "1969-12-31T23:59:59Z",
            "9999-01-01T00:00:00Z",
            "2026-11-20T09:00:05Zjunk",
            "２０２６-11-20T09:00:05Z",
        ] {
            assert_eq!(parse_rfc3339_ms(bad), None, "{bad}");
        }
        let mut r = quota_reader();
        let line = format!(
            r#"{{"timestamp":"2026-11-20T09:00:05.000Z","type":"event_msg","payload":{{"type":"token_count","info":null,"rate_limits":{BOTH}}}}}"#
        );
        r.line(&line);
        assert_eq!(r.take_quota()[0].line_ms, Some(1_795_165_205_000));
        // A missing or unreadable time is no time, and the window is still read.
        r.line(&line.replace("2026-11-20T09:00:05.000Z", "soon"));
        assert_eq!(r.take_quota()[0].line_ms, None);
        r.line(&limits(BOTH));
        assert_eq!(r.take_quota()[0].line_ms, None);
    }

    /// A JSONL line that still ends in `\r` (a CRLF file, split at `\n`) is read like any other, tokens and plan windows alike.
    #[test]
    fn a_line_ending_in_a_carriage_return_is_read() {
        let mut r = quota_reader();
        r.line(&format!("{}\r", limits(BOTH)));
        assert_eq!(r.take_quota().len(), 2);
        let tokens = r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":7,"output_tokens":1}}}}"#;
        assert_eq!(r.line(&format!("{tokens}\r")).unwrap().input, 7);
    }
}
