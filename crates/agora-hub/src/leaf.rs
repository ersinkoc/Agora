//! The hub's own TLS certificate and the names it carries (docs/specs/pairing.md section 5b).
//!
//! [`Leaf`] issues the hub's server leaf from the hub's CA with the names [`crate::names::select`] picks, at start and again at every
//! `agora pair open`. When the names the hub is reachable at are not all in the leaf (a new address from DHCP, a second network card) it
//! issues a new leaf from the **same CA** (so the CA fingerprint and every enrolled node stay as they are), puts it in the listener's
//! [`TlsSlot`] and writes it to `server-cert.pem`; connections accepted from then on use it, open connections keep theirs. When that
//! cannot be done it says which names are missing, and the hub refuses to open for pairing rather than open with a leaf a machine could
//! not verify.

use std::net::IpAddr;
use std::sync::{Mutex, PoisonError};

use crate::ca::Ca;
use crate::names::{covers, select, Iface, NamesError};
use crate::tls::{acceptor_with, leaf_hash, TlsSlot};

/// Why the names could not be provisioned: a stable word for the sentence, and the names concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamesFailure {
    /// `interfaces` (the system would not list the network addresses), `invalid` (a `--name` a certificate cannot carry; `names` is
    /// that name), `too_many` (more `--name` values than a leaf carries) or `issue` (the certificate could not be made; `names` is
    /// what it should have carried).
    pub reason: &'static str,
    pub names: Vec<String>,
}

/// Something that can make sure the hub's certificate names the addresses it is reachable at (the hub core calls it from `PairOpen`).
pub trait Provision: Send + Sync {
    /// Returns the hash of the certificate that is in use afterwards.
    fn provision(&self) -> Result<[u8; 32], NamesFailure>;
}

/// Keeps the certificate PEM where `agora doctor` reads it (`server-cert.pem`): the caller decides how (the hub binary writes it
/// atomically and owner-only through `agora-platform`).
pub type Persist = Box<dyn Fn(&str) -> std::io::Result<()> + Send + Sync>;

type Interfaces = Box<dyn Fn() -> Result<Vec<Iface>, String> + Send + Sync>;
/// More owner names, asked for again at every `provision` (a file the owner's tools write while the hub runs). Every name in it must
/// already be checked ([`crate::names::checked_name`]).
pub type Extra = Box<dyn Fn() -> Vec<String> + Send + Sync>;

type HostName = Box<dyn Fn() -> Option<String> + Send + Sync>;

struct Current {
    names: Vec<String>,
    hash: [u8; 32],
}

pub struct Leaf {
    ca: Ca,
    configured: Vec<String>,
    listen: IpAddr,
    host: HostName,
    interfaces: Interfaces,
    /// Keeps `server-cert.pem` up to date (`agora doctor` reads it), when there is such a place.
    persist: Option<Persist>,
    /// Names added while the hub runs (`agora remote tailscale`); read at every `provision`.
    extra: Option<Extra>,
    slot: TlsSlot,
    current: Mutex<Current>,
}

impl Leaf {
    /// Issues the first leaf. A system that will not list its interfaces does not stop the hub: the leaf then carries what is known
    /// (the owner's names, `localhost`, the host name) and the failure is returned for the caller to say; `provision` tries again at the
    /// next `pair open`. An error here means no certificate could be made at all.
    pub fn start(
        ca: Ca,
        configured: Vec<String>,
        listen: IpAddr,
        host: HostName,
        interfaces: Interfaces,
        persist: Option<Persist>,
    ) -> Result<(Self, Option<NamesFailure>), String> {
        let (names, failure) = match wanted(&configured, &host, listen, &interfaces) {
            Ok(n) => (n, None),
            Err(f) if f.reason == "interfaces" => {
                let n = wanted(
                    &configured,
                    &host,
                    listen,
                    &(Box::new(|| Ok(vec![])) as Interfaces),
                )
                .map_err(|f| format!("{f:?}"))?;
                (n, Some(f))
            }
            Err(f) => return Err(format!("{}: {}", f.reason, f.names.join(", "))),
        };
        let (cert, key) = ca.issue_server(&names).map_err(|e| e.to_string())?;
        let hash = leaf_hash(&cert).map_err(|e| e.to_string())?;
        let acceptor =
            acceptor_with(&cert, &key, ca.cert_pem(), true).map_err(|e| e.to_string())?;
        if let Some(save) = &persist {
            save(&cert).map_err(|e| format!("server-cert.pem: {e}"))?;
        }
        let leaf = Self {
            ca,
            configured,
            listen,
            host,
            interfaces,
            persist,
            extra: None,
            slot: TlsSlot::new(acceptor, hash),
            current: Mutex::new(Current { names, hash }),
        };
        Ok((leaf, failure))
    }

    /// Makes every later `provision` also name what `extra` returns (after the `--name` values).
    #[must_use]
    pub fn with_extra(mut self, extra: Extra) -> Self {
        self.extra = Some(extra);
        self
    }

    /// The slot the listener takes its acceptor from.
    pub fn slot(&self) -> TlsSlot {
        self.slot.clone()
    }

    /// The names in the certificate that is in use.
    pub fn names(&self) -> Vec<String> {
        self.lock().names.clone()
    }

    /// The addresses (not host names) in the certificate that is in use: what discovery may announce.
    pub fn addresses(&self) -> Vec<IpAddr> {
        self.lock()
            .names
            .iter()
            .filter_map(|n| n.parse().ok())
            .collect()
    }

    pub fn hash(&self) -> [u8; 32] {
        self.lock().hash
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Current> {
        self.current.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The names the hub should carry now, or why they cannot be worked out. The interfaces are only listed for a wildcard listen address.
fn wanted(
    configured: &[String],
    host: &HostName,
    listen: IpAddr,
    interfaces: &Interfaces,
) -> Result<Vec<String>, NamesFailure> {
    let ifs = if listen.is_unspecified() {
        interfaces().map_err(|_| NamesFailure {
            reason: "interfaces",
            names: vec![],
        })?
    } else {
        vec![]
    };
    select(configured, host().as_deref(), listen, &ifs)
        .map(|n| n.all)
        .map_err(|e| match e {
            NamesError::Invalid(n) => NamesFailure {
                reason: "invalid",
                names: vec![n],
            },
            NamesError::TooMany => NamesFailure {
                reason: "too_many",
                names: configured.to_vec(),
            },
        })
}

impl Provision for Leaf {
    fn provision(&self) -> Result<[u8; 32], NamesFailure> {
        let mut configured = self.configured.clone();
        // Names added by the owner's tools never take the leaf past its limit (the owner's own `--name` values must always fit, and a
        // refusal here would stop every `pair open`): the ones that do not fit are left out, and `agora doctor` says so.
        for n in self.extra.iter().flat_map(|f| f()) {
            if !configured.contains(&n) && configured.len() < crate::names::MAX_NAMES {
                configured.push(n);
            }
        }
        let names = wanted(&configured, &self.host, self.listen, &self.interfaces)?;
        let mut cur = self.lock();
        if covers(&cur.names, &names) {
            return Ok(cur.hash);
        }
        let failed = || NamesFailure {
            reason: "issue",
            names: names.clone(),
        };
        let (cert, key) = self.ca.issue_server(&names).map_err(|_| failed())?;
        let hash = leaf_hash(&cert).map_err(|_| failed())?;
        let acceptor =
            acceptor_with(&cert, &key, self.ca.cert_pem(), true).map_err(|_| failed())?;
        // Saved before it is switched on: when the file cannot be written nothing changes and the owner is told.
        if let Some(save) = &self.persist {
            save(&cert).map_err(|_| NamesFailure {
                reason: "write",
                names: names.clone(),
            })?;
        }
        self.slot.set(acceptor, hash);
        *cur = Current { names, hash };
        Ok(hash)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Names written while the hub runs reach the certificate at the next `provision`, from the same CA, and a name that is not
    /// there any more does not force a new one.
    #[test]
    fn extra_names_reach_the_leaf_at_the_next_provision() {
        let ca = Ca::generate("test ca").unwrap();
        let ca_pem = ca.cert_pem().to_string();
        let extra: Arc<Mutex<Vec<String>>> = Arc::default();
        let seen = Arc::clone(&extra);
        let (leaf, failure) = Leaf::start(
            ca,
            vec![],
            "127.0.0.1".parse().unwrap(),
            Box::new(|| Some("box".to_string())),
            Box::new(|| Ok(vec![])),
            None,
        )
        .unwrap();
        assert!(failure.is_none());
        let leaf = leaf.with_extra(Box::new(move || seen.lock().unwrap().clone()));
        let first = leaf.provision().unwrap();
        assert_eq!(leaf.names(), ["localhost", "box", "127.0.0.1"]);
        extra
            .lock()
            .unwrap()
            .extend(["box.tail1234.ts.net".to_string(), "100.64.0.7".to_string()]);
        let second = leaf.provision().unwrap();
        assert_ne!(first, second, "a new certificate");
        assert_eq!(
            leaf.names(),
            [
                "box.tail1234.ts.net",
                "100.64.0.7",
                "localhost",
                "box",
                "127.0.0.1"
            ]
        );
        // Same CA: a node that trusts it still verifies the new leaf.
        assert!(ca_pem.contains("CERTIFICATE"));
        extra.lock().unwrap().clear();
        assert_eq!(
            leaf.provision().unwrap(),
            second,
            "a removed name changes nothing"
        );
    }

    /// A leaf with as many owner names as it can carry still issues: names added later are left out, never an error.
    #[test]
    fn added_names_that_do_not_fit_are_left_out_and_never_stop_provision() {
        let ca = Ca::generate("test ca").unwrap();
        let owner: Vec<String> = (0..crate::names::MAX_NAMES)
            .map(|i| format!("h{i}.example"))
            .collect();
        let (leaf, _) = Leaf::start(
            ca,
            owner.clone(),
            "127.0.0.1".parse().unwrap(),
            Box::new(|| None),
            Box::new(|| Ok(vec![])),
            None,
        )
        .unwrap();
        let leaf = leaf.with_extra(Box::new(|| vec!["box.tail1.ts.net".to_string()]));
        assert!(leaf.provision().is_ok());
        assert_eq!(leaf.names(), owner);
    }
}
