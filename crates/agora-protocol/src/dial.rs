//! Opening a TCP connection to a `host:port`, shared by the clients of the hub (the node's uplink, the terminal console; the desktop
//! tunnel keeps a copy so the desktop shell does not link this crate) and the hub's own outgoing requests.
//!
//! A loopback name such as `localhost` resolves to both `::1` and `127.0.0.1`. Every listener of ours binds IPv4, and on Windows
//! a refused connection to `::1` is retried for about two seconds before the next address is tried, so dialing `localhost` in
//! resolver order wastes that time on every connection. When every resolved address is loopback, IPv4 is tried first; IPv6 stays
//! as the fallback, so a hub that listens only on `::1` is still reached. Any other name keeps the resolver's order.

use std::io;
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

/// The order to try `addrs` in: unchanged, unless every address is loopback, then IPv4 before IPv6 (each family keeps its own
/// order).
pub fn order(mut addrs: Vec<SocketAddr>) -> Vec<SocketAddr> {
    if !addrs.is_empty() && addrs.iter().all(|a| a.ip().is_loopback()) {
        // A stable sort: `false` (IPv4) before `true` (IPv6).
        addrs.sort_by_key(SocketAddr::is_ipv6);
    }
    addrs
}

/// How long one address of a loopback name is tried by [`connect_bounded`]. A listener on this machine answers a connection at
/// once, or refuses it at once, except on Windows, where a refused connection is retried for about two seconds before it fails:
/// a node waiting for a restarting hub on the same machine would spend most of each redial there. Generous for a loaded machine.
pub const LOOPBACK_ATTEMPT: Duration = Duration::from_millis(500);

/// The limit for each attempt on `addrs` in [`connect_bounded`]: [`LOOPBACK_ATTEMPT`] when every address is loopback (the same
/// rule as [`order`]), none otherwise (the operating system's own connect timeout, as before).
pub fn attempt_limit(addrs: &[SocketAddr]) -> Option<Duration> {
    (!addrs.is_empty() && addrs.iter().all(|a| a.ip().is_loopback())).then_some(LOOPBACK_ATTEMPT)
}

fn no_address() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "no address")
}

/// Connect to `to` (a `host:port` or a `(host, port)`), trying each resolved address in [`order`], each for at most `timeout`.
/// The error is the last address's.
pub fn connect_timeout(to: impl ToSocketAddrs, timeout: Duration) -> io::Result<TcpStream> {
    let addrs = order(to.to_socket_addrs()?.collect());
    let mut last = no_address();
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, timeout) {
            Ok(s) => return Ok(s),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Connect to `host_port`, trying each resolved address in [`order`]. The error is the last address's.
#[cfg(feature = "dial")]
pub async fn connect(host_port: &str) -> io::Result<tokio::net::TcpStream> {
    let addrs = order(tokio::net::lookup_host(host_port).await?.collect());
    let mut last = no_address();
    for addr in addrs {
        match tokio::net::TcpStream::connect(addr).await {
            Ok(s) => return Ok(s),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// [`connect`], but each address of a loopback name is given up after [`LOOPBACK_ATTEMPT`] (see [`attempt_limit`]) and counts as
/// refused; other names are not bounded here. For a client that dials again on its own schedule (the node's uplink).
#[cfg(feature = "dial")]
pub async fn connect_bounded(host_port: &str) -> io::Result<tokio::net::TcpStream> {
    let addrs = order(tokio::net::lookup_host(host_port).await?.collect());
    let limit = attempt_limit(&addrs);
    let mut last = no_address();
    for addr in addrs {
        let attempt = tokio::net::TcpStream::connect(addr);
        let r = match limit {
            Some(limit) => tokio::time::timeout(limit, attempt)
                .await
                .unwrap_or_else(|_| Err(io::Error::from(io::ErrorKind::TimedOut))),
            None => attempt.await,
        };
        match r {
            Ok(s) => return Ok(s),
            Err(e) => last = e,
        }
    }
    Err(last)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn a(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    #[test]
    fn loopback_tries_ipv4_first_and_keeps_ipv6_as_fallback() {
        let got = order(vec![a("[::1]:7000"), a("127.0.0.1:7000")]);
        assert_eq!(got, vec![a("127.0.0.1:7000"), a("[::1]:7000")]);
    }

    #[test]
    fn loopback_keeps_order_within_each_family() {
        let got = order(vec![
            a("[::1]:1"),
            a("127.0.0.2:1"),
            a("[::1]:2"),
            a("127.0.0.1:1"),
        ]);
        assert_eq!(
            got,
            vec![
                a("127.0.0.2:1"),
                a("127.0.0.1:1"),
                a("[::1]:1"),
                a("[::1]:2")
            ]
        );
    }

    #[test]
    fn other_names_keep_the_resolver_order() {
        let mixed = vec![a("[2001:db8::1]:443"), a("192.0.2.1:443")];
        assert_eq!(order(mixed.clone()), mixed);
        // Loopback mixed with a routable address is not a loopback name: the resolver's order stands.
        let half = vec![a("[::1]:443"), a("192.0.2.1:443")];
        assert_eq!(order(half.clone()), half);
    }

    #[test]
    fn ipv6_only_loopback_is_kept() {
        assert_eq!(order(vec![a("[::1]:9")]), vec![a("[::1]:9")]);
        assert!(order(Vec::new()).is_empty());
    }

    #[test]
    fn localhost_resolution_puts_ipv4_first() {
        // Whatever this machine's resolver returns for `localhost`, an IPv4 address (if any) comes before every IPv6 one.
        let Ok(found) = "localhost:80".to_socket_addrs() else {
            return;
        };
        let got = order(found.collect());
        if let Some(first_v6) = got.iter().position(SocketAddr::is_ipv6) {
            assert!(got[first_v6..].iter().all(SocketAddr::is_ipv6));
        }
    }

    #[test]
    fn connect_timeout_reaches_an_ipv4_listener_by_name() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let s = connect_timeout(("localhost", port), Duration::from_secs(5)).unwrap();
        assert!(s.peer_addr().unwrap().is_ipv4());
    }

    #[test]
    fn only_loopback_names_get_the_short_attempt_limit() {
        assert_eq!(
            attempt_limit(&[a("[::1]:7000"), a("127.0.0.1:7000")]),
            Some(LOOPBACK_ATTEMPT)
        );
        assert_eq!(attempt_limit(&[a("127.0.0.2:1")]), Some(LOOPBACK_ATTEMPT));
        assert_eq!(attempt_limit(&[a("192.0.2.1:443")]), None);
        assert_eq!(attempt_limit(&[a("[2001:db8::1]:443")]), None);
        // Loopback mixed with a routable address is not a loopback name.
        assert_eq!(attempt_limit(&[a("[::1]:443"), a("192.0.2.1:443")]), None);
        assert_eq!(attempt_limit(&[]), None);
        assert!(LOOPBACK_ATTEMPT <= Duration::from_millis(500));
    }

    #[cfg(feature = "dial")]
    #[tokio::test]
    async fn connect_bounded_reaches_a_listener_and_fails_fast_without_one() {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        let s = connect_bounded(&format!("localhost:{port}")).await.unwrap();
        assert!(s.peer_addr().unwrap().is_ipv4());
        drop((s, l));
        // Nobody listens now: refused (or, on Windows, given up) within the limit for each address.
        let started = std::time::Instant::now();
        assert!(connect_bounded(&format!("localhost:{port}")).await.is_err());
        assert!(
            started.elapsed() < LOOPBACK_ATTEMPT * 3,
            "{:?}",
            started.elapsed()
        );
    }

    #[cfg(feature = "dial")]
    #[tokio::test]
    async fn connect_reaches_an_ipv4_listener_by_name() {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        let s = connect(&format!("localhost:{port}")).await.unwrap();
        assert!(s.peer_addr().unwrap().is_ipv4());
    }
}
