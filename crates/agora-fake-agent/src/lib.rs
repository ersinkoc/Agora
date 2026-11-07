//! Scripted fake coding agent for tests: plays a TOML script of screen frames and delays to a terminal.
//!
//! This is the L0/L1 mode only (screen output). ACP (L2) and adapter (L3) modes come later.

use std::io::{self, Write};
use std::time::Duration;

use serde::Deserialize;

/// ANSI: clear screen and move the cursor home.
const CLEAR_HOME: &str = "\x1b[2J\x1b[H";

/// A script: an ordered list of frames.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Script {
    /// Optional label, only for humans reading the file.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(rename = "frame", default)]
    pub frames: Vec<Frame>,
}

/// One screen frame.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Frame {
    /// Screen contents. `\n` becomes CR LF on output.
    pub text: String,
    /// How long to hold this frame before the next one.
    #[serde(default)]
    pub delay_ms: u64,
    /// Clear the screen before drawing (default true). Set false to append, like scrolling output.
    #[serde(default = "default_clear")]
    pub clear: bool,
}

fn default_clear() -> bool {
    true
}

/// Script loading errors.
#[derive(Debug)]
pub enum ScriptError {
    Parse(toml::de::Error),
    Empty,
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "invalid script: {e}"),
            Self::Empty => write!(f, "invalid script: no frames"),
        }
    }
}

impl std::error::Error for ScriptError {}

impl Script {
    /// Parse a script from TOML text.
    pub fn parse(src: &str) -> Result<Self, ScriptError> {
        let script: Self = toml::from_str(src).map_err(ScriptError::Parse)?;
        if script.frames.is_empty() {
            return Err(ScriptError::Empty);
        }
        Ok(script)
    }
}

/// Play `script` to `out`, calling `sleep` for each frame delay (injectable so tests do not wait).
pub fn play<W: Write>(
    script: &Script,
    out: &mut W,
    mut sleep: impl FnMut(Duration),
) -> io::Result<()> {
    for frame in &script.frames {
        if frame.clear {
            out.write_all(CLEAR_HOME.as_bytes())?;
        }
        out.write_all(frame.text.replace('\n', "\r\n").as_bytes())?;
        out.flush()?;
        if frame.delay_ms > 0 {
            sleep(Duration::from_millis(frame.delay_ms));
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used)] // expect is allowed in tests
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_script() {
        assert!(matches!(Script::parse(""), Err(ScriptError::Empty)));
    }

    #[test]
    fn rejects_garbage() {
        assert!(matches!(
            Script::parse("frame = 3"),
            Err(ScriptError::Parse(_))
        ));
    }

    #[test]
    fn plays_frames_in_order_with_delays() {
        let script = Script::parse(
            "[[frame]]\ntext = \"a\\nb\"\ndelay_ms = 5\n[[frame]]\ntext = \"c\"\nclear = false\n",
        )
        .expect("valid script");
        let mut out = Vec::new();
        let mut sleeps = Vec::new();
        play(&script, &mut out, |d| sleeps.push(d)).expect("write to vec");
        assert_eq!(out, b"\x1b[2J\x1b[Ha\r\nbc");
        assert_eq!(sleeps, vec![Duration::from_millis(5)]);
    }
}
