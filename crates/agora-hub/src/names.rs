//! The names the hub's TLS leaf must carry (docs/specs/pairing.md section 5b): pure selection, no sockets and no system calls.
//!
//! A machine that pairs checks that the address it dialled is one of the leaf's names, so the leaf must name every address the hub can
//! be reached at: the `--name` values the owner gave, `localhost`, the host name, and the addresses of the interfaces the hub listens on.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// At most this many names in the leaf (the owner's own `--name` values always fit or the hub refuses; the rest is cut from the end).
pub const MAX_NAMES: usize = 16;
/// A DNS name is at most 253 bytes.
const MAX_NAME_BYTES: usize = 253;

/// One address of one interface, as the platform lists it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Iface {
    pub ip: IpAddr,
    pub up: bool,
    pub loopback: bool,
    /// A temporary (privacy) or deprecated IPv6 address: it changes or goes away by itself, so it is never named.
    pub transient: bool,
}

/// What the hub wants in its leaf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Names {
    /// In a fixed order (so "the same set" is "the same list"): the owner's names, `localhost`, the host name, IPv4 addresses
    /// ascending, IPv6 addresses ascending.
    pub all: Vec<String>,
    /// Automatic names (never an owner's) that did not fit under [`MAX_NAMES`].
    pub dropped: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NamesError {
    /// A `--name` that cannot be a name in a certificate.
    Invalid(String),
    /// More owner names than the leaf can carry.
    TooMany,
}

/// True for what a certificate can carry as a DNS name: letters, digits, `-`, `.` and `_`, not empty, within the length limit, not an
/// IP address.
fn dns_name_ok(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= MAX_NAME_BYTES
        && !n.starts_with('.')
        && n.parse::<IpAddr>().is_err()
        && n.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_'))
}

/// The canonical text of a name: an IP address in its shortest form, a DNS name in lower case.
fn canonical(n: &str) -> Option<String> {
    let n = n.trim();
    if let Ok(ip) = n.parse::<IpAddr>() {
        return Some(ip.to_string());
    }
    if let Some(inner) = n.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
        return inner.parse::<Ipv6Addr>().ok().map(|ip| ip.to_string());
    }
    if n.contains(['[', ']']) {
        return None;
    }
    dns_name_ok(n).then(|| n.to_ascii_lowercase())
}

/// A name the owner's tools may put in the certificate on their own (not typed by the owner as `--name`): the canonical form, or
/// `None`. Stricter than a `--name`: letters, digits, `-` and `.` only; labels of 1 to 63 characters that do not start or end with
/// `-`; no wildcard, no empty label, no space, control or direction-changing character (the check is on the bytes, so non-ASCII text
/// never passes). An IP address is accepted in its shortest form.
pub fn checked_name(n: &str) -> Option<String> {
    let n = n.strip_suffix('.').unwrap_or(n);
    if n.is_empty() || n.len() > MAX_NAME_BYTES || !n.is_ascii() {
        return None;
    }
    if let Ok(ip) = n.parse::<IpAddr>() {
        return Some(ip.to_string());
    }
    let ok = n.split('.').all(|l| {
        (1..=63).contains(&l.len())
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    });
    ok.then(|| n.to_ascii_lowercase())
}

fn v4_link_local(a: Ipv4Addr) -> bool {
    a.octets()[0] == 169 && a.octets()[1] == 254
}

fn v6_link_local(a: Ipv6Addr) -> bool {
    a.segments()[0] & 0xffc0 == 0xfe80
}

/// An address a machine on the network could be given: not loopback, unspecified, multicast or link-local (a link-local address needs a
/// zone to be dialled, which a certificate cannot carry), and not an IPv4-mapped IPv6 address.
fn usable(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(a) => {
            !(a.is_loopback()
                || a.is_unspecified()
                || a.is_multicast()
                || a.is_broadcast()
                || v4_link_local(a))
        }
        IpAddr::V6(a) => {
            !(a.is_loopback()
                || a.is_unspecified()
                || a.is_multicast()
                || v6_link_local(a)
                || a.to_ipv4_mapped().is_some())
        }
    }
}

/// Selects the names. `listen` is the hub's listen address: a wildcard (`0.0.0.0` or `::`) means every usable address of every
/// interface that is up (`0.0.0.0` only the IPv4 ones; `::` both, as a dual-stack socket serves both); a specific address means that
/// address alone, as the owner chose it (even a loopback one).
pub fn select(
    configured: &[String],
    host: Option<&str>,
    listen: IpAddr,
    interfaces: &[Iface],
) -> Result<Names, NamesError> {
    let mut owner: Vec<String> = Vec::new();
    for n in configured {
        let c = canonical(n).ok_or_else(|| NamesError::Invalid(n.clone()))?;
        if !owner.contains(&c) {
            owner.push(c);
        }
    }
    if owner.len() > MAX_NAMES {
        return Err(NamesError::TooMany);
    }
    let mut auto: Vec<String> = vec!["localhost".to_string()];
    if let Some(h) = host.and_then(canonical) {
        // A host name that is an address is not a name.
        if h.parse::<IpAddr>().is_err() {
            auto.push(h);
        }
    }
    let mut v4: Vec<Ipv4Addr> = Vec::new();
    let mut v6: Vec<Ipv6Addr> = Vec::new();
    // `[::ffff:0.0.0.0]` is the IPv4 wildcard: it is told apart from a named address by its canonical form, and never named.
    let wildcard = listen.to_canonical();
    if wildcard.is_unspecified() {
        for i in interfaces
            .iter()
            .filter(|i| i.up && !i.loopback && !i.transient)
        {
            match (i.ip, wildcard) {
                (IpAddr::V4(a), _) if usable(i.ip) => v4.push(a),
                (IpAddr::V6(a), IpAddr::V6(_)) if usable(i.ip) => v6.push(a),
                _ => {}
            }
        }
    } else {
        match listen {
            IpAddr::V4(a) => v4.push(a),
            IpAddr::V6(a) => {
                // An IPv4-mapped listen address (`::ffff:a.b.c.d`) serves the IPv4 address it stands for, which is what a machine
                // dials (F930); the form the owner typed is named too, for a machine given that literal (Codex review of F930).
                if let Some(m) = a.to_ipv4_mapped() {
                    v4.push(m);
                }
                v6.push(a);
            }
        }
    }
    v4.sort();
    v6.sort();
    auto.extend(v4.into_iter().map(|a| a.to_string()));
    auto.extend(v6.into_iter().map(|a| a.to_string()));

    let mut all = owner;
    let mut dropped = 0;
    for n in auto {
        if all.contains(&n) {
            continue;
        }
        if all.len() < MAX_NAMES {
            all.push(n);
        } else {
            dropped += 1;
        }
    }
    Ok(Names { all, dropped })
}

/// True when every name in `wanted` is already in `have` (an address that went away does not force a new leaf).
pub fn covers(have: &[String], wanted: &[String]) -> bool {
    wanted.iter().all(|w| have.contains(w))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn up(s: &str) -> Iface {
        Iface {
            ip: ip(s),
            up: true,
            loopback: false,
            transient: false,
        }
    }

    const ANY4: &str = "0.0.0.0";

    #[test]
    fn a_checked_name_is_plain_letters_digits_hyphens_and_dots() {
        assert_eq!(
            checked_name("Box.Tail-1.ts.net."),
            Some("box.tail-1.ts.net".into())
        );
        assert_eq!(
            checked_name("100.101.102.103"),
            Some("100.101.102.103".into())
        );
        for bad in [
            "",
            ".",
            "*.ts.net",
            "a..b",
            "-a.ts.net",
            "a-.ts.net",
            "a b.ts.net",
            "a\u{202e}b.ts.net",
            "a\nb.ts.net",
            "caf\u{e9}.ts.net",
            "a_b.ts.net",
            "a/b",
            "a:b",
        ] {
            assert_eq!(checked_name(bad), None, "{bad:?}");
        }
        assert_eq!(checked_name(&format!("{}.ts.net", "a".repeat(64))), None);
        assert_eq!(checked_name(&format!("{}.ts.net", "a.".repeat(130))), None);
    }

    #[test]
    fn a_default_hub_names_localhost_its_host_and_every_lan_address() {
        let ifs = [
            Iface {
                ip: ip("127.0.0.1"),
                up: true,
                loopback: true,
                transient: false,
            },
            up("192.168.1.20"),
            up("fe80::1"),
            up("2001:db8::5"),
        ];
        let n = select(&[], Some("Box"), ip(ANY4), &ifs).unwrap();
        // The listener is IPv4-only: the IPv6 address is not one the hub is reachable at.
        assert_eq!(n.all, ["localhost", "box", "192.168.1.20"]);
        let n = select(&[], Some("Box"), ip("::"), &ifs).unwrap();
        assert_eq!(n.all, ["localhost", "box", "192.168.1.20", "2001:db8::5"]);
    }

    #[test]
    fn loopback_link_local_multicast_down_and_mapped_addresses_are_left_out() {
        let ifs = [
            up("127.0.0.1"),
            up("169.254.3.4"),
            up("224.0.0.1"),
            up("0.0.0.0"),
            up("255.255.255.255"),
            up("::1"),
            up("fe80::abcd"),
            up("ff02::1"),
            up("::ffff:10.0.0.1"),
            Iface {
                ip: ip("10.0.0.9"),
                up: false,
                loopback: false,
                transient: false,
            },
            up("10.0.0.7"),
        ];
        let n = select(&[], None, ip("::"), &ifs).unwrap();
        assert_eq!(n.all, ["localhost", "10.0.0.7"]);
    }

    #[test]
    fn temporary_and_deprecated_addresses_are_not_named() {
        let ifs = [
            up("2001:db8::1"),
            Iface {
                ip: ip("2001:db8::2"),
                up: true,
                loopback: false,
                transient: true,
            },
        ];
        let n = select(&[], None, ip("::"), &ifs).unwrap();
        assert_eq!(n.all, ["localhost", "2001:db8::1"]);
    }

    #[test]
    fn a_specific_listen_address_is_the_only_address() {
        let ifs = [up("192.168.1.20"), up("10.0.0.7")];
        let n = select(&[], None, ip("10.0.0.7"), &ifs).unwrap();
        assert_eq!(n.all, ["localhost", "10.0.0.7"]);
        // Even a loopback address the owner chose.
        let n = select(&[], None, ip("127.0.0.1"), &ifs).unwrap();
        assert_eq!(n.all, ["localhost", "127.0.0.1"]);
    }

    #[test]
    fn the_owners_names_come_first_in_their_order_and_nothing_repeats() {
        let ifs = [up("10.0.0.7"), up("10.0.0.7")];
        let cfg = [
            "Hub.Example".to_string(),
            "localhost".into(),
            "10.0.0.7".into(),
            "hub.example".into(),
        ];
        let n = select(&cfg, Some("hub.example"), ip(ANY4), &ifs).unwrap();
        assert_eq!(n.all, ["hub.example", "localhost", "10.0.0.7"]);
    }

    #[test]
    fn addresses_are_ordered_v4_then_v6_each_ascending_so_the_same_set_is_the_same_list() {
        let a = [
            up("10.0.0.9"),
            up("2001:db8::2"),
            up("10.0.0.2"),
            up("2001:db8::1"),
        ];
        let mut b = a;
        b.reverse();
        let x = select(&[], None, ip("::"), &a).unwrap();
        let y = select(&[], None, ip("::"), &b).unwrap();
        assert_eq!(x, y);
        assert_eq!(
            x.all,
            [
                "localhost",
                "10.0.0.2",
                "10.0.0.9",
                "2001:db8::1",
                "2001:db8::2"
            ]
        );
    }

    #[test]
    fn the_count_is_capped_and_the_cut_is_counted_but_never_an_owner_name() {
        let ifs: Vec<Iface> = (1..=30).map(|i| up(&format!("10.0.0.{i}"))).collect();
        let n = select(&["a.example".to_string()], Some("h"), ip(ANY4), &ifs).unwrap();
        assert_eq!(n.all.len(), MAX_NAMES);
        assert_eq!(n.all[0], "a.example");
        assert_eq!(n.dropped, 30 + 2 + 1 - MAX_NAMES);
        // Sixteen owner names fill the leaf; seventeen are refused rather than cut.
        let many: Vec<String> = (0..17).map(|i| format!("n{i}.example")).collect();
        assert_eq!(select(&many, None, ip(ANY4), &[]), Err(NamesError::TooMany));
        assert_eq!(
            select(&many[..16], None, ip(ANY4), &ifs).unwrap().all,
            &many[..16]
        );
    }

    #[test]
    fn a_name_a_certificate_cannot_carry_is_refused_not_dropped() {
        for bad in ["", "a b", "a/b", "é.example", ".x", &"a".repeat(254)] {
            assert_eq!(
                select(&[bad.to_string()], None, ip(ANY4), &[]),
                Err(NamesError::Invalid(bad.to_string())),
                "{bad:?}"
            );
        }
        // A bracketed IPv6 address is the address.
        assert_eq!(
            select(&["[2001:db8::1]".to_string()], None, ip("127.0.0.1"), &[])
                .unwrap()
                .all[0],
            "2001:db8::1"
        );
    }

    #[test]
    fn f422_only_one_balanced_pair_may_bracket_an_ipv6_owner_name() {
        let control = select(&["[2001:db8::1]".to_string()], None, ip("127.0.0.1"), &[]).unwrap();
        assert_eq!(control.all[0], "2001:db8::1");
        println!("CONTROL OK");

        let malformed = "[2001:db8::1".to_string();
        let actual = select(std::slice::from_ref(&malformed), None, ip("127.0.0.1"), &[]);
        let expected = Err(NamesError::Invalid(malformed));
        println!("EXPECTED: {expected:?}");
        println!("ACTUAL: {actual:?}");
        if actual == expected {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert_eq!(actual, expected);
    }

    #[test]
    fn verifies_f422_owner_name_bracket_boundaries() {
        for valid in ["2001:db8::1", "[2001:db8::1]", "127.0.0.1", "hub.example"] {
            assert!(
                select(&[valid.to_string()], None, ip("127.0.0.1"), &[]).is_ok(),
                "{valid}"
            );
        }
        for invalid in [
            "[2001:db8::1",
            "2001:db8::1]",
            "[[2001:db8::1]]",
            "[127.0.0.1]",
        ] {
            assert_eq!(
                select(&[invalid.to_string()], None, ip("127.0.0.1"), &[]),
                Err(NamesError::Invalid(invalid.to_string())),
                "{invalid}"
            );
        }
        println!("FIX VERIFIED");
    }

    #[test]
    fn a_mapped_listen_address_names_the_ipv4_address_it_serves() {
        // F930: `--listen [::ffff:192.168.1.5]:8443` accepts IPv4 connections to 192.168.1.5; that is what a machine dials.
        // The form the owner typed stays named for a machine given that literal (Codex review).
        let n = select(&[], None, ip("::ffff:192.168.1.5"), &[up("10.0.0.7")]).unwrap();
        assert_eq!(n.all, ["localhost", "192.168.1.5", "::ffff:192.168.1.5"]);
    }

    #[test]
    fn a_mapped_wildcard_listen_address_is_a_wildcard_and_never_named() {
        let n = select(&[], None, ip("::ffff:0.0.0.0"), &[up("10.0.0.7")]).unwrap();
        assert_eq!(n.all, ["localhost", "10.0.0.7"]);
    }

    #[test]
    fn a_host_name_that_is_not_a_name_is_skipped() {
        for h in [Some("bad host"), Some("10.1.1.1"), Some(""), None] {
            let n = select(&[], h, ip("127.0.0.1"), &[]).unwrap();
            assert_eq!(n.all, ["localhost", "127.0.0.1"], "{h:?}");
        }
    }

    #[test]
    fn a_leaf_that_has_every_wanted_name_is_kept_even_with_extra_ones() {
        let have: Vec<String> = ["localhost", "10.0.0.7", "10.0.0.8"]
            .map(String::from)
            .into();
        assert!(covers(&have, &["localhost".into(), "10.0.0.7".into()]));
        assert!(!covers(&have, &["localhost".into(), "10.0.0.9".into()]));
        assert!(covers(&have, &[]));
    }
}
