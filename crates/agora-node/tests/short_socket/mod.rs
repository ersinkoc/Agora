//! #926, #1031: the folder for a test node's control socket (or its whole home). Include it next to `private_home` and call
//! `private_home::init` first.
//!
//! - **Short.** A unix socket path must be shorter than SUN_LEN (108 bytes on Linux, 104 on macOS), and the node puts its agents'
//!   mesh sockets beside the control socket (`<folder>/m/agent-N.sock`). Under a deep checkout a folder below the target folder
//!   does not fit.
//! - **Out of a confined agent's reach.** The control token sits beside the socket (`<socket>.token`, ADR-0082), and a node refuses
//!   to start a confined agent whose profile reaches it (`control_token_reachable`). On Linux an agent may write all of `/tmp`, so
//!   the system temp folder is no place for it when `TMPDIR` is unset or below `/tmp`.
//!
//! The first place that is both: the test's own folder, a folder in the system temp folder, or one in the real home folder.
#![allow(dead_code)]

use agora_node::sandbox::{Level, SandboxPolicy};
use std::path::{Path, PathBuf};

/// The longest folder used: `<folder>/m/agent-NNNN.sock` adds 20 bytes and stays below macOS's 104.
const MAX: usize = 80;

/// A folder for sockets; one made here is removed on drop.
pub struct SocketDir {
    path: PathBuf,
    own: bool,
}

impl SocketDir {
    /// `preferred` (the test's own folder, which the test removes) if it fits; otherwise a new `agora-<tag>-<pid>` folder in the
    /// system temp folder or a `.tmp-test-agora-<tag>-<pid>` folder in the real home folder. Panics, naming the folders tried, when
    /// no place fits.
    pub fn new(preferred: &Path, tag: &str) -> Self {
        let name = format!("agora-{tag}-{}", std::process::id());
        let mut tried = Vec::new();
        let candidates = std::iter::once((preferred.to_path_buf(), false))
            .chain(std::iter::once((std::env::temp_dir().join(&name), true)))
            .chain(
                // Named so that nothing matching `~/.agora*` (the node's own folders) can be taken for it.
                super::private_home::real_home()
                    .map(|h| (h.join(format!(".tmp-test-{name}")), true)),
            );
        for (path, own) in candidates {
            if path.as_os_str().len() > MAX {
                tried.push(path);
                continue;
            }
            if own {
                let _ = std::fs::remove_dir_all(&path);
            }
            if std::fs::create_dir_all(&path).is_ok() && !reachable(&path) {
                return Self { path, own };
            }
            if own {
                let _ = std::fs::remove_dir_all(&path);
            }
            tried.push(path);
        }
        panic!("no short folder out of a confined agent's reach for the node's socket; tried {tried:?}");
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The socket `name` in this folder.
    pub fn socket(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for SocketDir {
    fn drop(&mut self) {
        if self.own {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

/// Whether a confined agent could read or write a control token in `dir`, by the node's own rule. Only where the node keeps a
/// control token (Linux); elsewhere any short folder does. The agent here is a program named without a folder; the folders these
/// tests start agents from (`target/debug`, `/bin`) hold none of the places tried.
fn reachable(dir: &Path) -> bool {
    if !agora_platform::CONTROL_TOKEN_REQUIRED {
        return false;
    }
    let policy = SandboxPolicy {
        level: Level::Worktree,
        ..SandboxPolicy::default()
    };
    let worktree = Path::new("/agora-test-worktree-that-does-not-exist");
    policy
        .profile("probe", None, worktree, "agent")
        .is_some_and(|p| policy.path_reachable(&dir.join("n.sock.token"), &p))
}
