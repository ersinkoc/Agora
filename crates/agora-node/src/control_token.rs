//! The control token (ADR-0082): a secret in the node's home that a confined agent cannot read, required on the control socket
//! where the system lets a confined agent connect to it (Linux). Pure file and string handling; the one platform call is
//! `agora_platform::write_private`. The token is never logged, never in an event, never in an environment variable or a command line.

use std::io;
use std::path::{Path, PathBuf};

/// The token file's name inside the node's home.
pub const TOKEN_FILE: &str = "control.token";
/// The socket name `agora node run` uses by default; the token lives beside it.
const DEFAULT_SOCKET: &str = "node.sock";
const TOKEN_BYTES: usize = 32;

/// Where the token for the control socket at `socket` is kept: `<home>/control.token` beside the default `node.sock`, and
/// `<socket>.token` for a socket with any other name, so a client derives it from the socket path alone.
pub fn token_path(socket: &Path) -> PathBuf {
    if socket.file_name().is_some_and(|n| n == DEFAULT_SOCKET) {
        if let Some(dir) = socket.parent() {
            return dir.join(TOKEN_FILE);
        }
    }
    let mut p = socket.as_os_str().to_owned();
    p.push(".token");
    PathBuf::from(p)
}

/// A token the node holds in memory and checks presented ones against.
pub struct ControlToken(String);

impl std::fmt::Debug for ControlToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ControlToken(..)")
    }
}

impl ControlToken {
    /// Makes a new token, in memory only.
    pub fn generate() -> io::Result<Self> {
        let mut raw = [0u8; TOKEN_BYTES];
        getrandom::getrandom(&mut raw).map_err(|e| io::Error::other(e.to_string()))?;
        let mut text = String::with_capacity(TOKEN_BYTES * 2);
        for b in raw {
            text.push(char::from(b"0123456789abcdef"[usize::from(b >> 4)]));
            text.push(char::from(b"0123456789abcdef"[usize::from(b & 15)]));
        }
        Ok(Self(text))
    }

    /// Writes the token to `path` (0600 from creation; an owner-only ACL on Windows), replacing any file there, so a token left by
    /// an earlier run never stays valid.
    pub fn write(&self, path: &Path) -> io::Result<()> {
        agora_platform::write_private(path, self.0.as_bytes())
    }

    /// Makes a new token and writes it to `path`.
    pub fn create(path: &Path) -> io::Result<Self> {
        let t = Self::generate()?;
        t.write(path)?;
        Ok(t)
    }

    /// Whether `presented` is this token. The comparison reads every byte whatever the first difference is.
    pub fn verify(&self, presented: &str) -> bool {
        let (a, b) = (self.0.as_bytes(), presented.as_bytes());
        // The length is not secret (always 64), so an early exit on it leaks nothing.
        if a.len() != b.len() {
            return false;
        }
        a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
    }
}

/// The token a client presents: the file's content when it is a well-formed token, `None` when there is no file (a node that
/// does not require one) or it cannot be read (a confined process, or another user).
pub fn read_token(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let t = text.trim();
    (t.len() == TOKEN_BYTES * 2 && t.bytes().all(|b| b.is_ascii_hexdigit())).then(|| t.to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("agora-ctok-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_token_is_found_beside_the_default_socket_or_named_after_another() {
        assert_eq!(
            token_path(Path::new("/h/node.sock")),
            Path::new("/h/control.token")
        );
        assert_eq!(
            token_path(Path::new("/h/other.sock")),
            Path::new("/h/other.sock.token")
        );
    }

    #[test]
    fn a_created_token_is_64_hex_private_and_verifies_only_itself() {
        let d = dir("create");
        let path = d.join(TOKEN_FILE);
        let t = ControlToken::create(&path).unwrap();
        let read = read_token(&path).unwrap();
        assert_eq!(read.len(), 64);
        assert!(read.bytes().all(|b| b.is_ascii_hexdigit()));
        assert!(t.verify(&read));
        assert!(!t.verify(""));
        assert!(!t.verify(&read[..63]));
        let mut off = read.clone().into_bytes();
        off[63] = if off[63] == b'0' { b'1' } else { b'0' };
        assert!(!t.verify(&String::from_utf8(off).unwrap()));
        assert!(!t.verify(&format!("{read}0")));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert!(!format!("{t:?}").contains(&read));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn every_start_replaces_the_token_so_a_stale_file_stops_working() {
        let d = dir("replace");
        let path = d.join(TOKEN_FILE);
        let first = ControlToken::create(&path).unwrap();
        let old = read_token(&path).unwrap();
        let second = ControlToken::create(&path).unwrap();
        let new = read_token(&path).unwrap();
        assert_ne!(old, new);
        assert!(first.verify(&old) && !second.verify(&old));
        assert!(second.verify(&new));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_missing_or_malformed_file_reads_as_no_token() {
        let d = dir("bad");
        assert_eq!(read_token(&d.join("none")), None);
        std::fs::write(d.join("short"), "abc").unwrap();
        assert_eq!(read_token(&d.join("short")), None);
        std::fs::write(d.join("nothex"), "z".repeat(64)).unwrap();
        assert_eq!(read_token(&d.join("nothex")), None);
        let _ = std::fs::remove_dir_all(&d);
    }
}
