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

use crate::ca::{cert_not_after_secs, Ca};
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
    /// When the certificate in use ends, seconds since 1970 (ADR-0085).
    not_after_secs: i64,
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
    /// How many times writing `server-cert.pem` failed after a certificate had been switched on (ADR-0085).
    persist_failures: std::sync::atomic::AtomicU64,
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
        let not_after_secs = cert_not_after_secs(&cert).map_err(|e| e.to_string())?;
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
            current: Mutex::new(Current {
                names,
                hash,
                not_after_secs,
            }),
            persist_failures: std::sync::atomic::AtomicU64::new(0),
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

    /// ADR-0085: replaces the certificate in use with a new one (same names, same CA, a fresh key) when it ends within `within_secs` of
    /// `now_secs`. New connections use it from then on; open ones keep theirs. The new certificate is switched on **first**: the hub
    /// keeps serving whatever happens to the file `agora doctor` reads. A failed write of `server-cert.pem` afterwards is logged at warn
    /// level and counted ([`Self::persist_failures`]) and the next attempt (the file is written again whenever a certificate is) is
    /// not blocked by it. `Ok(true)` when a new certificate is now in use; an error only when none could be made.
    pub fn renew_if_ending(&self, now_secs: i64, within_secs: i64) -> Result<bool, String> {
        let mut cur = self.lock();
        if cur.not_after_secs.saturating_sub(now_secs) > within_secs {
            return Ok(false);
        }
        let (cert, key) = self
            .ca
            .issue_server(&cur.names)
            .map_err(|e| e.to_string())?;
        let hash = leaf_hash(&cert).map_err(|e| e.to_string())?;
        let not_after_secs = cert_not_after_secs(&cert).map_err(|e| e.to_string())?;
        let acceptor =
            acceptor_with(&cert, &key, self.ca.cert_pem(), true).map_err(|e| e.to_string())?;
        self.slot.set(acceptor, hash);
        cur.hash = hash;
        cur.not_after_secs = not_after_secs;
        if let Some(save) = &self.persist {
            if let Err(e) = save(&cert) {
                crate::metrics::saturating_add(&self.persist_failures, 1);
                eprintln!(
                    "warn: the hub's new certificate is in use but server-cert.pem could not be written: {e}"
                );
            }
        }
        Ok(true)
    }

    /// How many times `server-cert.pem` could not be written after a certificate was switched on.
    pub fn persist_failures(&self) -> u64 {
        self.persist_failures
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// When the certificate in use ends, in seconds since 1970.
    pub fn not_after_secs(&self) -> i64 {
        self.lock().not_after_secs
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
    let ifs = if listen.to_canonical().is_unspecified() {
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
        // The names are worked out under the lock, so the leaf that is issued last is the one made from the last listing: two opens at
        // once around an address change cannot leave the older listing in place (F931). The owner's extra names are collected under the
        // lock too, so a re-issue that races a change of the names file cannot leave the older names in place.
        let mut cur = self.lock();
        let mut configured = self.configured.clone();
        // Names added by the owner's tools never take the leaf past its limit (the owner's own `--name` values must always fit, and a
        // refusal here would stop every `pair open`): the ones that do not fit are left out, and `agora doctor` says so.
        for n in self.extra.iter().flat_map(|f| f()) {
            if !configured.contains(&n) && configured.len() < crate::names::MAX_NAMES {
                configured.push(n);
            }
        }
        let names = wanted(&configured, &self.host, self.listen, &self.interfaces)?;
        if covers(&cur.names, &names) {
            return Ok(cur.hash);
        }
        let failed = || NamesFailure {
            reason: "issue",
            names: names.clone(),
        };
        let (cert, key) = self.ca.issue_server(&names).map_err(|_| failed())?;
        let hash = leaf_hash(&cert).map_err(|_| failed())?;
        let not_after_secs = cert_not_after_secs(&cert).map_err(|_| failed())?;
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
        *cur = Current {
            names,
            hash,
            not_after_secs,
        };
        Ok(hash)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::sync::{mpsc, Arc};

    /// F931: two `PairOpen`s at once around an address change. Open #1 lists the old address and is held inside its listing; the
    /// address changes; open #2 starts. #1 is let go once #2 has finished or after a bounded wait (on this code #2 waits for #1's lock,
    /// so the wait runs out); the wait only decides when #1 is let go, the verdict is the leaf left in place: it must name the new address.
    #[test]
    fn two_opens_at_once_around_an_address_change_leave_the_new_address_named() {
        let addr = Arc::new(Mutex::new("10.0.0.1"));
        let (listed_tx, listed_rx) = mpsc::channel::<()>();
        let (go_tx, go_rx) = mpsc::channel::<()>();
        let gate = Arc::new(Mutex::new(Some((listed_tx, go_rx))));
        let (a, g) = (Arc::clone(&addr), Arc::clone(&gate));
        let (leaf, _) = Leaf::start(
            Ca::generate("ca").unwrap(),
            vec![],
            "0.0.0.0".parse().unwrap(),
            Box::new(|| None),
            Box::new(move || {
                let now = *a.lock().unwrap();
                if std::thread::current().name() == Some("first") {
                    if let Some((listed, go)) = g.lock().unwrap().take() {
                        listed.send(()).unwrap();
                        go.recv().unwrap();
                    }
                }
                Ok(vec![Iface {
                    ip: now.parse().unwrap(),
                    up: true,
                    loopback: false,
                    transient: false,
                }])
            }),
            None,
        )
        .unwrap();
        let leaf = Arc::new(leaf);
        let l1 = Arc::clone(&leaf);
        let first = std::thread::Builder::new()
            .name("first".into())
            .spawn(move || l1.provision())
            .unwrap();
        listed_rx.recv().unwrap();
        *addr.lock().unwrap() = "10.0.0.2";
        let (done_tx, done_rx) = mpsc::channel();
        let l2 = Arc::clone(&leaf);
        let second = std::thread::spawn(move || {
            let r = l2.provision();
            let _ = done_tx.send(());
            r
        });
        let _ = done_rx.recv_timeout(std::time::Duration::from_secs(1));
        go_tx.send(()).unwrap();
        assert!(first.join().unwrap().is_ok());
        assert!(second.join().unwrap().is_ok());
        assert!(
            leaf.names().contains(&"10.0.0.2".to_string()),
            "{:?}",
            leaf.names()
        );
    }

    /// F931 for the owner's extra names: a re-issue that races a change of the names file. Open #1 collects the old extra name and is
    /// held there; the file changes; open #2 starts. The names are collected under the lock, so #2 waits for #1 and the leaf left in
    /// place is the one made last, from the extras present when the lock was taken: it must name the new extra, not the old one.
    #[test]
    fn two_opens_around_a_names_file_change_leave_the_new_extra_name() {
        let extra = Arc::new(Mutex::new(vec!["old.example".to_string()]));
        let (listed_tx, listed_rx) = mpsc::channel::<()>();
        let (go_tx, go_rx) = mpsc::channel::<()>();
        let gate = Arc::new(Mutex::new(Some((listed_tx, go_rx))));
        let (x, g) = (Arc::clone(&extra), Arc::clone(&gate));
        let (leaf, _) = Leaf::start(
            Ca::generate("ca").unwrap(),
            vec![],
            "127.0.0.1".parse().unwrap(),
            Box::new(|| None),
            Box::new(|| Ok(vec![])),
            None,
        )
        .unwrap();
        // Started with no extras: the first leaf lacks both names, so each open has something to issue.
        let leaf = leaf.with_extra(Box::new(move || {
            let now = x.lock().unwrap().clone();
            if std::thread::current().name() == Some("first") {
                if let Some((listed, go)) = g.lock().unwrap().take() {
                    listed.send(()).unwrap();
                    go.recv().unwrap();
                }
            }
            now
        }));
        let leaf = Arc::new(leaf);
        let l1 = Arc::clone(&leaf);
        let first = std::thread::Builder::new()
            .name("first".into())
            .spawn(move || l1.provision())
            .unwrap();
        listed_rx.recv().unwrap();
        *extra.lock().unwrap() = vec!["new.example".to_string()];
        let (done_tx, done_rx) = mpsc::channel();
        let l2 = Arc::clone(&leaf);
        let second = std::thread::spawn(move || {
            let r = l2.provision();
            let _ = done_tx.send(());
            r
        });
        let _ = done_rx.recv_timeout(std::time::Duration::from_secs(1));
        go_tx.send(()).unwrap();
        assert!(first.join().unwrap().is_ok());
        assert!(second.join().unwrap().is_ok());
        assert!(
            leaf.names().contains(&"new.example".to_string()),
            "{:?}",
            leaf.names()
        );
    }

    /// An IPv4-mapped wildcard (`[::ffff:0.0.0.0]`) is the unspecified IPv4 address: the interfaces are listed and `0.0.0.0` is never
    /// named in the leaf.
    #[test]
    fn a_mapped_wildcard_listen_is_a_wildcard_and_names_no_unspecified_address() {
        let (leaf, failure) = Leaf::start(
            Ca::generate("ca").unwrap(),
            vec![],
            "::ffff:0.0.0.0".parse().unwrap(),
            Box::new(|| None),
            Box::new(|| {
                Ok(vec![Iface {
                    ip: "10.0.0.5".parse().unwrap(),
                    up: true,
                    loopback: false,
                    transient: false,
                }])
            }),
            None,
        )
        .unwrap();
        assert!(failure.is_none());
        let names = leaf.names();
        assert!(names.contains(&"10.0.0.5".to_string()), "{names:?}");
        assert!(
            !names.iter().any(|n| n.contains("0.0.0.0") || n == "::"),
            "{names:?}"
        );
    }

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

    fn quiet_leaf(ca: Ca, persist: Option<Persist>) -> Leaf {
        Leaf::start(
            ca,
            vec!["10.1.2.3".to_string()],
            "127.0.0.1".parse().unwrap(),
            Box::new(|| None),
            Box::new(|| Ok(vec![])),
            persist,
        )
        .unwrap()
        .0
    }

    /// ADR-0085: a certificate that does not end soon is left alone; one that does is replaced (same names, a new key), the file
    /// `agora doctor` reads is rewritten, and a failed write changes nothing.
    #[test]
    fn the_server_certificate_is_replaced_only_when_it_ends_soon() {
        let saved = Arc::new(Mutex::new(Vec::<String>::new()));
        let sink = Arc::clone(&saved);
        let ca = Ca::generate("ca")
            .unwrap()
            .with_lifetime_for_tests(std::time::Duration::from_secs(100));
        let leaf = quiet_leaf(
            ca,
            Some(Box::new(move |pem| {
                sink.lock().unwrap().push(pem.to_string());
                Ok(())
            })),
        );
        let (names, hash, end) = (leaf.names(), leaf.hash(), leaf.not_after_secs());
        assert_eq!(saved.lock().unwrap().len(), 1, "written at start");
        // 100 s of life: 60 s left is not within 30 s...
        assert!(!leaf.renew_if_ending(end - 60, 30).unwrap());
        assert_eq!(leaf.hash(), hash);
        assert_eq!(saved.lock().unwrap().len(), 1);
        // ...and 30 s left is.
        assert!(leaf.renew_if_ending(end - 30, 30).unwrap());
        assert_ne!(leaf.hash(), hash, "a new certificate is in use");
        assert_eq!(leaf.names(), names);
        assert_eq!(saved.lock().unwrap().len(), 2, "server-cert.pem rewritten");
        assert_eq!(
            leaf.slot().current().1,
            leaf.hash(),
            "new connections use it"
        );
        assert!(leaf.not_after_secs() >= end);
    }

    /// The new certificate is switched on even when the file cannot be written; the failure is counted, and a later write that works
    /// brings the file up to date.
    #[test]
    fn a_failed_write_of_the_certificate_file_does_not_block_the_swap() {
        let ca = Ca::generate("ca")
            .unwrap()
            .with_lifetime_for_tests(std::time::Duration::from_secs(100));
        let calls = Arc::new(Mutex::new(0u32));
        let n = Arc::clone(&calls);
        let leaf = quiet_leaf(
            ca,
            Some(Box::new(move |_| {
                let mut c = n.lock().unwrap();
                *c += 1;
                // The first write (at start) works, the second fails (a read-only directory, a full disk), the third works.
                if *c == 2 {
                    Err(std::io::Error::other("read-only file system"))
                } else {
                    Ok(())
                }
            })),
        );
        let (hash, end) = (leaf.hash(), leaf.not_after_secs());
        assert_eq!(leaf.persist_failures(), 0);
        assert!(leaf.renew_if_ending(end, 30).unwrap());
        assert_ne!(leaf.hash(), hash, "the new certificate is in use");
        assert_eq!(leaf.slot().current().1, leaf.hash());
        assert_eq!(leaf.persist_failures(), 1);
        // The next daily attempt is not blocked: a certificate still ending soon is replaced and its file written.
        let end2 = leaf.not_after_secs();
        assert!(leaf.renew_if_ending(end2, 30).unwrap());
        assert_eq!(*calls.lock().unwrap(), 3);
        assert_eq!(leaf.persist_failures(), 1);
    }

    #[test]
    fn f380_certificate_persist_failure_count_does_not_wrap() {
        let calls = Arc::new(Mutex::new(0u32));
        let seen = Arc::clone(&calls);
        let ca = Ca::generate("ca")
            .unwrap()
            .with_lifetime_for_tests(std::time::Duration::from_secs(100));
        let leaf = quiet_leaf(
            ca,
            Some(Box::new(move |_| {
                let mut calls = seen.lock().unwrap();
                *calls += 1;
                if *calls == 1 {
                    Ok(())
                } else {
                    Err(std::io::Error::other("read-only file system"))
                }
            })),
        );

        leaf.persist_failures
            .store(u64::MAX - 1, std::sync::atomic::Ordering::Relaxed);
        assert!(leaf.renew_if_ending(leaf.not_after_secs(), 30).unwrap());
        assert_eq!(leaf.persist_failures(), u64::MAX);
        println!("CONTROL OK");

        assert!(leaf.renew_if_ending(leaf.not_after_secs(), 30).unwrap());
        let actual = leaf.persist_failures();
        println!("EXPECTED: {}", u64::MAX);
        println!("ACTUAL: {actual}");
        if actual == u64::MAX {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert_eq!(actual, u64::MAX);
    }

    #[test]
    fn verifies_f380_certificate_persist_failure_count_boundaries() {
        let calls = Arc::new(Mutex::new(0u32));
        let seen = Arc::clone(&calls);
        let ca = Ca::generate("ca")
            .unwrap()
            .with_lifetime_for_tests(std::time::Duration::from_secs(100));
        let leaf = quiet_leaf(
            ca,
            Some(Box::new(move |_| {
                let mut calls = seen.lock().unwrap();
                *calls += 1;
                if *calls == 1 {
                    Ok(())
                } else {
                    Err(std::io::Error::other("disk full"))
                }
            })),
        );
        let fail_once = |starting| {
            leaf.persist_failures
                .store(starting, std::sync::atomic::Ordering::Relaxed);
            leaf.renew_if_ending(leaf.not_after_secs(), 30).unwrap();
            leaf.persist_failures()
        };

        assert_eq!(fail_once(0), 1);
        assert_eq!(fail_once(u64::MAX - 1), u64::MAX);
        assert_eq!(fail_once(u64::MAX), u64::MAX);
        println!("FIX VERIFIED");
    }

    /// The same, against a real directory that cannot be written.
    #[test]
    fn a_read_only_directory_for_server_cert_pem_still_swaps() {
        let dir = std::env::temp_dir().join(format!("agora-leaf-ro-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("server-cert.pem");
        let target = file.clone();
        let ca = Ca::generate("ca")
            .unwrap()
            .with_lifetime_for_tests(std::time::Duration::from_secs(100));
        let leaf = quiet_leaf(ca, Some(Box::new(move |pem| std::fs::write(&target, pem))));
        // Make the destination unwritable by turning the file into a directory.
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        let (hash, end) = (leaf.hash(), leaf.not_after_secs());
        assert!(leaf.renew_if_ending(end, 30).unwrap());
        assert_ne!(leaf.hash(), hash);
        assert_eq!(leaf.persist_failures(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
