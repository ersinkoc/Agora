//! Token usage read from the file an agent writes itself (ADR-0042). Pure: a [`FileReader`] is given one line at a time and says
//! what usage the line adds; finding and tailing files is the node's job (`agora-node::usage_files`).

use std::collections::{HashSet, VecDeque};

use serde_json::Value;

use crate::manifest::{UsageFile, UsageMode};

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
            seen: HashSet::new(),
            order: VecDeque::new(),
        }
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
}
