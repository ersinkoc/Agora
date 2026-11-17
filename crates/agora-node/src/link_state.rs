//! What the node knows about its own link to the hub, written where `agora status` and `agora doctor` can read it.
//!
//! `agora status` used to print the hub the machine was *configured* for as "connected", also while every dial failed (macOS answers
//! `No route to host` until the program is allowed onto the local network). The node is the only one that knows, so it writes one
//! small file in its data directory on every change. This is node-local state, not a protocol message: nothing about it goes to the hub.
//! No user-facing text here: the reader renders a stable code and one parameter from `locales/`.

use std::path::Path;

/// File name inside the node's data directory.
pub const FILE: &str = "hub-link";

/// Longest `param` kept (the reason text is only a parameter, never matched by a reader).
const PARAM_MAX: usize = 200;

/// Link codes (stable; the suffix of the reader's locale keys). Unknown errors are [`OTHER`] with the raw text as the parameter.
pub const UNREACHABLE: &str = "link.unreachable";
pub const NOT_PERMITTED: &str = "link.not_permitted";
pub const REFUSED: &str = "link.refused";
pub const CERTIFICATE: &str = "link.certificate";
pub const HUB_REFUSED: &str = "link.hub_refused";
pub const CLOSED: &str = "link.closed";
pub const OTHER: &str = "link.other";

const KNOWN: [&str; 7] = [
    UNREACHABLE,
    NOT_PERMITTED,
    REFUSED,
    CERTIFICATE,
    HUB_REFUSED,
    CLOSED,
    OTHER,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkState {
    /// The node started and has not yet reached the hub.
    Connecting,
    /// The hub accepted the connection and it is open now.
    Up,
    /// The last attempt failed (or the link was lost): a stable code and the detail as a parameter.
    Down { code: &'static str, param: String },
}

/// Maps a transport error text to a code. Matches the OS error number and the text, for the three systems Agora runs on.
pub fn code_for_transport(text: &str) -> &'static str {
    let t = text.to_lowercase();
    let has = |s: &str| t.contains(s);
    if has("certificate") || has("unknownissuer") || has("invalid peer") || has("badsignature") {
        CERTIFICATE
    } else if has("no route to host")
        || has("network is unreachable")
        || has("host is unreachable")
        || has("timed out")
        || has("os error 65)")
        || has("os error 51)")
        || has("os error 101)")
        || has("os error 113)")
        || has("os error 10065)")
        || has("os error 10051)")
    {
        UNREACHABLE
    } else if has("operation not permitted")
        || has("permission denied")
        || has("os error 1)")
        || has("os error 13)")
        || has("os error 10013)")
    {
        NOT_PERMITTED
    } else if has("connection refused")
        || has("os error 61)")
        || has("os error 111)")
        || has("os error 10061)")
    {
        REFUSED
    } else {
        OTHER
    }
}

impl LinkState {
    /// The state for a failed attempt: the transport's own text.
    pub fn down_transport(text: &str) -> Self {
        Self::Down {
            code: code_for_transport(text),
            param: clip(text),
        }
    }

    /// The hub closed the connection saying why: its stable error code is the parameter.
    pub fn down_hub_refused(code: &str) -> Self {
        Self::Down {
            code: HUB_REFUSED,
            param: clip(code),
        }
    }

    pub fn down(code: &'static str, param: &str) -> Self {
        Self::Down {
            code,
            param: clip(param),
        }
    }

    fn encode(&self) -> String {
        match self {
            Self::Connecting => "connecting\n".into(),
            Self::Up => "up\n".into(),
            Self::Down { code, param } => format!("down\n{code}\n{param}\n"),
        }
    }

    /// Reads what [`Self::write`] wrote; `None` for a missing or unreadable file (an older node, or none running).
    pub fn read(dir: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(dir.join(FILE)).ok()?;
        let mut lines = text.lines();
        match lines.next()? {
            "connecting" => Some(Self::Connecting),
            "up" => Some(Self::Up),
            "down" => {
                let code = lines.next()?;
                let code = KNOWN.iter().find(|k| **k == code).copied().unwrap_or(OTHER);
                Some(Self::Down {
                    code,
                    param: lines.next().unwrap_or("").to_string(),
                })
            }
            _ => None,
        }
    }

    /// Writes the state for `agora status`; a failure is ignored (the link itself never depends on it).
    pub fn write(&self, dir: &Path) {
        let tmp = dir.join(format!("{FILE}.tmp"));
        if std::fs::write(&tmp, self.encode()).is_ok() {
            let _ = std::fs::rename(&tmp, dir.join(FILE));
        }
    }

    /// Removes the file (a node that starts, so an earlier run's state is never read as this one's).
    pub fn clear(dir: &Path) {
        let _ = std::fs::remove_file(dir.join(FILE));
    }
}

fn clip(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(PARAM_MAX)
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn os_errors_map_to_a_small_set_of_codes() {
        for (text, code) in [
            ("No route to host (os error 65)", UNREACHABLE),
            ("No route to host (os error 113)", UNREACHABLE),
            ("Network is unreachable (os error 101)", UNREACHABLE),
            ("Operation not permitted (os error 1)", NOT_PERMITTED),
            ("Permission denied (os error 13)", NOT_PERMITTED),
            ("timed out", UNREACHABLE),
            ("Connection refused (os error 111)", REFUSED),
            ("Connection refused (os error 61)", REFUSED),
            ("invalid peer certificate: UnknownIssuer", CERTIFICATE),
            ("something nobody has seen (os error 999)", OTHER),
            ("a different error (os error 1111)", OTHER),
            ("", OTHER),
        ] {
            assert_eq!(code_for_transport(text), code, "{text}");
        }
    }

    #[test]
    fn a_state_survives_the_file_and_an_unknown_code_reads_as_other() {
        let d = std::env::temp_dir().join(format!("agora-linkstate-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(LinkState::read(&d), None);
        for s in [
            LinkState::Connecting,
            LinkState::Up,
            LinkState::down_transport("No route to host (os error 65)"),
            LinkState::down_hub_refused("protocol.producer_mismatch"),
        ] {
            s.write(&d);
            assert_eq!(LinkState::read(&d), Some(s));
        }
        std::fs::write(d.join(FILE), "down\nlink.from_the_future\nx\n").unwrap();
        assert_eq!(
            LinkState::read(&d),
            Some(LinkState::Down {
                code: OTHER,
                param: "x".into()
            })
        );
        std::fs::write(d.join(FILE), "garbage\n").unwrap();
        assert_eq!(LinkState::read(&d), None);
        LinkState::clear(&d);
        assert_eq!(LinkState::read(&d), None);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_parameter_is_one_bounded_line() {
        let s = LinkState::down_transport(&format!("a\nb{}", "x".repeat(500)));
        let LinkState::Down { param, .. } = s else {
            panic!("down expected")
        };
        assert!(!param.contains('\n') && param.chars().count() == PARAM_MAX);
    }
}
