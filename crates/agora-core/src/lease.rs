//! File leases: time-limited exclusive claims on a path (a directory lease covers everything below it).
//! The node enforces them; this is the pure bookkeeping.

use crate::ids::AgentId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseError {
    /// Another agent holds a lease on this path or a parent/child of it.
    Conflict { holder: AgentId },
    /// Path is empty, absolute-escaping, or contains `..`.
    InvalidPath,
    /// A lease must last at least one millisecond.
    ZeroTtl,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lease {
    pub path: String,
    pub holder: AgentId,
    pub expires_at_ms: u64,
}

impl Lease {
    pub fn is_expired(&self, now_ms: u64) -> bool {
        now_ms >= self.expires_at_ms
    }
}

/// All leases of one project.
#[derive(Debug, Default)]
pub struct LeaseTable {
    leases: Vec<Lease>,
}

fn normalize(path: &str) -> Result<String, LeaseError> {
    let unified = path.replace('\\', "/");
    let mut parts = Vec::new();
    for part in unified.split('/') {
        match part {
            "" | "." => {}
            ".." => return Err(LeaseError::InvalidPath),
            p => parts.push(p),
        }
    }
    if parts.is_empty() {
        return Err(LeaseError::InvalidPath);
    }
    Ok(parts.join("/"))
}

/// True when one path equals or is an ancestor of the other, on component boundaries.
fn overlaps(a: &str, b: &str) -> bool {
    let is_prefix =
        |p: &str, q: &str| q == p || q.strip_prefix(p).is_some_and(|r| r.starts_with('/'));
    is_prefix(a, b) || is_prefix(b, a)
}

impl LeaseTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Acquire or renew. The same holder may re-acquire (renew) its own lease.
    pub fn acquire(
        &mut self,
        path: &str,
        holder: &AgentId,
        now_ms: u64,
        ttl_ms: u64,
    ) -> Result<Lease, LeaseError> {
        if ttl_ms == 0 {
            return Err(LeaseError::ZeroTtl);
        }
        let path = normalize(path)?;
        self.expire(now_ms);
        if let Some(other) = self
            .leases
            .iter()
            .find(|l| &l.holder != holder && overlaps(&l.path, &path))
        {
            return Err(LeaseError::Conflict {
                holder: other.holder.clone(),
            });
        }
        self.leases
            .retain(|l| !(l.path == path && &l.holder == holder));
        let lease = Lease {
            path,
            holder: holder.clone(),
            expires_at_ms: now_ms.saturating_add(ttl_ms),
        };
        self.leases.push(lease.clone());
        Ok(lease)
    }

    /// Release a lease; returns whether one was held by `holder`.
    pub fn release(&mut self, path: &str, holder: &AgentId) -> bool {
        let Ok(path) = normalize(path) else {
            return false;
        };
        let before = self.leases.len();
        self.leases
            .retain(|l| !(l.path == path && &l.holder == holder));
        self.leases.len() != before
    }

    /// Drop expired leases and return them.
    pub fn expire(&mut self, now_ms: u64) -> Vec<Lease> {
        let (expired, live): (Vec<_>, Vec<_>) = std::mem::take(&mut self.leases)
            .into_iter()
            .partition(|l| l.is_expired(now_ms));
        self.leases = live;
        expired
    }

    /// Who may not write `path` right now (a live lease held by someone else).
    pub fn blocked_for(&self, path: &str, agent: &AgentId, now_ms: u64) -> Option<&AgentId> {
        let path = normalize(path).ok()?;
        self.leases
            .iter()
            .find(|l| !l.is_expired(now_ms) && &l.holder != agent && overlaps(&l.path, &path))
            .map(|l| &l.holder)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ag(n: &str) -> AgentId {
        AgentId::new(n)
    }

    #[test]
    fn directory_lease_covers_children_on_component_boundaries() {
        let mut t = LeaseTable::new();
        assert!(t.acquire("src/net", &ag("a"), 0, 100).is_ok());
        assert_eq!(
            t.acquire("src/net/tcp.rs", &ag("b"), 1, 100),
            Err(LeaseError::Conflict { holder: ag("a") })
        );
        assert_eq!(
            t.acquire("src", &ag("b"), 1, 100),
            Err(LeaseError::Conflict { holder: ag("a") })
        );
        // "src/network" is a different component than "src/net"
        assert!(t.acquire("src/network", &ag("b"), 1, 100).is_ok());
    }

    #[test]
    fn holder_can_renew_and_others_get_it_after_expiry() {
        let mut t = LeaseTable::new();
        assert!(t.acquire("a.rs", &ag("a"), 0, 10).is_ok());
        let renewed = t.acquire("./a.rs", &ag("a"), 5, 10);
        assert_eq!(renewed.map(|l| l.expires_at_ms), Ok(15));
        assert!(t.acquire("a.rs", &ag("b"), 14, 10).is_err());
        assert!(t.acquire("a.rs", &ag("b"), 15, 10).is_ok());
    }

    #[test]
    fn release_and_blocked_for() {
        let mut t = LeaseTable::new();
        assert!(t.acquire("a.rs", &ag("a"), 0, 10).is_ok());
        assert_eq!(t.blocked_for("a.rs", &ag("b"), 1), Some(&ag("a")));
        assert_eq!(t.blocked_for("a.rs", &ag("a"), 1), None);
        assert_eq!(t.blocked_for("a.rs", &ag("b"), 10), None);
        assert!(!t.release("a.rs", &ag("b")));
        assert!(t.release("a.rs", &ag("a")));
        assert_eq!(t.blocked_for("a.rs", &ag("b"), 1), None);
    }

    #[test]
    fn rejects_bad_input() {
        let mut t = LeaseTable::new();
        assert_eq!(
            t.acquire("../x", &ag("a"), 0, 1),
            Err(LeaseError::InvalidPath)
        );
        assert_eq!(t.acquire("", &ag("a"), 0, 1), Err(LeaseError::InvalidPath));
        assert_eq!(t.acquire("x", &ag("a"), 0, 0), Err(LeaseError::ZeroTtl));
    }
}
