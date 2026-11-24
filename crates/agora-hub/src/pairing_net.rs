//! Pairing at run time (ADR-0065, `docs/specs/pairing.md`): the connections and the certificate authority around the pure state in
//! [`crate::pairing`]. Everything that decides something is there; this file only does what its answers say.
//!
//! - [`serve_pairing`]: a machine on a TLS connection with no client certificate. `PairRequest`, the challenge, `PairReveal`, then it
//!   waits (on the owning connection, the only place a result is ever sent) for the owner's decision, and gets one certificate or
//!   none.
//! - [`owner_command`]: the owner's `PairOpen`, `PairClose`, `PairStatus`, `PairApprove` and `PairDeny`, from a signed-in window.
//!   Owner only; anyone else gets `auth.forbidden`.
//! - [`WindowFeed`]: the cards an owner's window renders. In memory only: the number is on the wire to that window and nowhere else
//!   (never in the event store, never in a log or an audit row).
//!
//! The browser path (spec section 6) is not here yet.

use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use agora_protocol::frame::{kinds, STREAM_COMMANDS, STREAM_CONTROL, STREAM_EVENTS};
use agora_protocol::v1::{self, command_ok, command_request, command_response};
use agora_protocol::ws::Conn;
use agora_protocol::{Error, ErrorCode, Frame};
use prost::Message;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::ca::{self, Ca};
use crate::enroll::{EnrollError, Enrollment};
use crate::hub::Hub;
use crate::pairing::{Commit, ConnId, Kind, Note, Pairing, Peer, Refusal, Reveal, Verdict};
use crate::server::{error_response, now_ms, ServeError};

/// The pairing state's own clock (spec section 7, T33): milliseconds that only go forward, from a fixed point in this process. Every
/// deadline (the switch's 10 minutes, a session's 5 minutes, the 30 seconds for a reveal) is on it, so a wall clock that is set
/// back cannot keep a window or a request open longer. Windows and the audit trail are given wall time ([`to_wall`]).
fn base() -> &'static (std::time::Instant, u64) {
    static BASE: std::sync::OnceLock<(std::time::Instant, u64)> = std::sync::OnceLock::new();
    BASE.get_or_init(|| (std::time::Instant::now(), now_ms()))
}

/// Now on the pairing clock. Starts at 1000 so that no real time is 0.
pub(crate) fn mono() -> u64 {
    1_000 + u64::try_from(base().0.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// A moment on the pairing clock as hub wall-clock milliseconds, for what a window displays. 0 stays 0.
pub(crate) fn to_wall(mono_ms: u64) -> u64 {
    if mono_ms == 0 {
        return 0;
    }
    base().1.saturating_add(mono_ms.saturating_sub(1_000))
}
use crate::users::{Access, Role};

/// The pairing state shared by every connection of one hub, and the signal that wakes a waiting connection when the owner decided.
pub struct Shared {
    state: Mutex<Pairing>,
    decided: tokio::sync::watch::Sender<u64>,
    /// Makes sure the hub's certificate names the addresses it is reachable at, before the switch opens (spec section 5b).
    names: Mutex<Option<std::sync::Arc<dyn crate::leaf::Provision>>>,
}

impl Default for Shared {
    fn default() -> Self {
        Self::new()
    }
}

impl Shared {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(Pairing::new()),
            decided: tokio::sync::watch::channel(0).0,
            names: Mutex::new(None),
        }
    }

    /// Runs `f` on the state. Keep it short: nothing in it may wait.
    pub fn with<R>(&self, f: impl FnOnce(&mut Pairing) -> R) -> R {
        f(&mut self.state.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// The hub offers pairing from the moment it knows its own certificate's hash (see [`Pairing::set_leaf`]).
    pub fn enable(&self, leaf_hash: [u8; 32]) {
        self.with(|p| p.set_leaf(leaf_hash));
    }

    pub fn enabled(&self) -> bool {
        self.with(|p| p.enabled())
    }

    /// From now on every `PairOpen` first makes sure the certificate names the hub's addresses, and is refused when it cannot.
    pub fn provision_names(&self, p: std::sync::Arc<dyn crate::leaf::Provision>) {
        *self.names.lock().unwrap_or_else(|e| e.into_inner()) = Some(p);
    }

    fn provisioner(&self) -> Option<std::sync::Arc<dyn crate::leaf::Provision>> {
        self.names.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// True while the switch is open (a timeout closes it).
    pub fn is_open(&self) -> bool {
        self.with(|p| p.status(mono()).open)
    }

    /// Tells the connections that wait that something changed (a decision, the switch closing).
    pub fn wake(&self) {
        self.decided.send_modify(|v| *v = v.wrapping_add(1));
    }

    fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.decided.subscribe()
    }
}

/// What the TLS layer knows about a pairing connection: where it came from and its exporter (docs/specs/pairing.md section 5).
#[derive(Debug, Clone, Copy)]
pub struct PairPeer {
    pub addr: IpAddr,
    pub exporter: [u8; 32],
    /// The hash of the certificate this very connection was shown, when the listener knows it (it does when the certificate can be
    /// replaced, spec section 5b); `None` means the hub's current one.
    pub leaf_hash: Option<[u8; 32]>,
}

static NEXT_CONN: AtomicU64 = AtomicU64::new(1);

/// Ends the connection's session when the connection ends, however it ends.
struct Guard<'a> {
    hub: &'a Hub,
    conn: ConnId,
}

impl Drop for Guard<'_> {
    fn drop(&mut self) {
        self.hub
            .pairing()
            .with(|p| p.connection_closed(self.conn, mono()));
    }
}

fn reply(request_id: &str, result: command_ok::Result) -> v1::CommandResponse {
    v1::CommandResponse {
        request_id: request_id.to_string(),
        result: Some(command_response::Result::Ok(v1::CommandOk {
            result: Some(result),
        })),
    }
}

async fn send<S>(conn: &mut Conn<S>, r: &v1::CommandResponse) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    conn.send(&Frame {
        stream_id: STREAM_COMMANDS,
        kind: 1,
        seq: None,
        payload: r.encode_to_vec(),
    })
    .await?;
    Ok(())
}

async fn refuse<S>(conn: &mut Conn<S>, request_id: &str, e: &Error) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    send(conn, &error_response(request_id, e)).await?;
    conn.close().await;
    Err(ServeError::Refused)
}

fn audit(hub: &Hub, who: &str, action: &str, node: &str, outcome: &str) {
    let _ = crate::audit::Audit::new(hub.store()).record(now_ms(), who, action, node, outcome);
}

/// The refusals of the last minute, once a minute, as a rate (section 8): a flood cannot fill the log.
fn report_rates(hub: &Hub) {
    for r in hub.pairing().with(|p| p.take_refusal_rates(mono())) {
        let kind = match r.kind {
            Kind::Machine => "machine",
            Kind::Browser => "browser",
        };
        audit(
            hub,
            &format!("pair:{}", r.address),
            "pair.refused",
            kind,
            &format!("count={}", r.count),
        );
    }
}

/// The longest a machine waiting for the owner goes without looking at its session (F810).
const RECHECK: Duration = Duration::from_secs(1);

fn sleep_for(until: Option<u64>) -> Duration {
    Duration::from_millis(until.map_or(1_000, |t| t.saturating_sub(mono()).max(1)))
}

/// A machine on a pairing connection. `first` is its first frame, already read. One connection, one pairing, then it is closed.
pub async fn serve_pairing<S>(
    mut conn: Conn<S>,
    hub: &Hub,
    ca: Option<&Ca>,
    first: Frame,
    peer: PairPeer,
) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let invalid = |field: &str| {
        ServeError::Protocol(Error::new(ErrorCode::ProtocolInvalidMessage).with("field", field))
    };
    if (first.stream_id, first.kind) != (STREAM_COMMANDS, 0) {
        return Err(invalid("frame"));
    }
    let req =
        v1::CommandRequest::decode(first.payload.as_slice()).map_err(|_| invalid("request"))?;
    let r = match req.command {
        Some(command_request::Command::PairRequest(r)) => r,
        // A reveal as a connection's first message names a session this connection cannot own (section 5a).
        Some(command_request::Command::PairReveal(_)) if hub.pairing().enabled() => {
            let me = Peer {
                conn: ConnId(NEXT_CONN.fetch_add(1, Ordering::Relaxed)),
                addr: peer.addr,
            };
            let e = hub.pairing().with(|p| p.unknown(mono(), me)).to_error();
            return refuse(&mut conn, &req.request_id, &e).await;
        }
        _ => return Err(ServeError::Refused),
    };
    if !hub.pairing().enabled() {
        return refuse(
            &mut conn,
            &req.request_id,
            &Error::new(ErrorCode::ProtocolCapabilityMissing).with("capability", "pairing.v1"),
        )
        .await;
    }
    report_rates(hub);
    let me = Peer {
        conn: ConnId(NEXT_CONN.fetch_add(1, Ordering::Relaxed)),
        addr: peer.addr,
    };
    let _guard = Guard { hub, conn: me.conn };
    let who = format!("pair:{}", peer.addr);

    // 1-2. The commitment, then the challenge.
    let started = hub.pairing().with(|p| {
        p.request_on(
            mono(),
            me,
            (peer.exporter, peer.leaf_hash),
            Commit {
                commitment: &r.commitment,
                name: &r.node_name,
                os: &r.os,
                version: &r.version,
            },
        )
    });
    let (pair_id, nonce_h) = match started {
        Ok(v) => v,
        Err(refusal) => return refuse(&mut conn, &req.request_id, &refusal.to_error()).await,
    };
    audit(hub, &who, "pair.request", &r.node_name, "ok");
    send(
        &mut conn,
        &reply(
            &req.request_id,
            command_ok::Result::PairChallenge(v1::PairChallenge {
                pair_id: pair_id.to_vec(),
                nonce_h: nonce_h.to_vec(),
            }),
        ),
    )
    .await?;

    // 3. The reveal, inside its 30 seconds. The session can end before it (its time, the switch closing, an approval of another
    // machine): the state is looked at on every turn, at least once a second, so the machine is told at once and with what ended it
    // (`pair.closed` for a closing, F1460), and pings cannot hold a connection whose session is over (F1461).
    let reveal_id: String;
    let mut ended = hub.pairing().subscribe();
    loop {
        let over = match hub.pairing().with(|p| p.verdict(me.conn, mono())) {
            Verdict::Waiting => None,
            Verdict::Expired => Some(Refusal::Expired),
            _ => Some(Refusal::Closed),
        };
        if let Some(refusal) = over {
            let e = refusal.to_error();
            audit(hub, &who, "pair.reveal", &r.node_name, e.code.as_str());
            return refuse(&mut conn, &req.request_id, &e).await;
        }
        let wait = sleep_for(hub.pairing().with(|p| p.deadline_of(me.conn))).min(RECHECK);
        let got = tokio::select! {
            // A frame already here comes first: a reveal that arrived is answered as a reveal (under its own id).
            biased;
            got = tokio::time::timeout(wait, conn.recv()) => got,
            _ = ended.changed() => continue,
        };
        match got {
            // Nothing came in time: the state decides at the top of the loop (and counts it).
            Err(_) => {}
            Ok(Ok(Some(f))) if (f.stream_id, f.kind) == (STREAM_CONTROL, kinds::PING) => {
                conn.send(&Frame {
                    stream_id: STREAM_CONTROL,
                    kind: kinds::PONG,
                    seq: None,
                    payload: f.payload,
                })
                .await?;
            }
            Ok(Ok(Some(f))) => {
                let revealed = if (f.stream_id, f.kind) == (STREAM_COMMANDS, 0) {
                    v1::CommandRequest::decode(f.payload.as_slice())
                        .ok()
                        .and_then(|q| match q.command {
                            Some(command_request::Command::PairReveal(v)) => {
                                Some((q.request_id, v))
                            }
                            _ => None,
                        })
                } else {
                    None
                };
                let Some((rid, v)) = revealed else {
                    let e = hub
                        .pairing()
                        .with(|p| p.violation(me.conn, mono()))
                        .to_error();
                    audit(hub, &who, "pair.reveal", &r.node_name, e.code.as_str());
                    return refuse(&mut conn, &req.request_id, &e).await;
                };
                // The CSR must parse and be signed by its own key; its hash is of the DER the machine signed.
                let csr_hash = (v.csr_pem.len() <= 8192)
                    .then(|| ca::csr_der(&v.csr_pem).ok())
                    .flatten()
                    .map(|der| <[u8; 32]>::from(Sha256::digest(&der)));
                let done = hub.pairing().with(|p| {
                    p.reveal(
                        mono(),
                        me,
                        Reveal {
                            pair_id: &v.pair_id,
                            nonce_n: &v.nonce_n,
                            csr_pem: &v.csr_pem,
                            csr_hash,
                            node_id_ok: ca::valid_node_id(&r.node_name),
                        },
                    )
                });
                if let Err(refusal) = done {
                    let e = refusal.to_error();
                    audit(hub, &who, "pair.reveal", &r.node_name, e.code.as_str());
                    return refuse(&mut conn, &rid, &e).await;
                }
                reveal_id = rid;
                break;
            }
            Ok(Ok(None)) | Ok(Err(_)) => return Ok(()),
        }
    }

    // 4-5. The number is on the owner's card; wait for the owner's decision on this connection.
    let mut changed = hub.pairing().subscribe();
    let verdict = loop {
        let v = hub.pairing().with(|p| p.verdict(me.conn, mono()));
        if v != Verdict::Waiting {
            break v;
        }
        // F810: the switch can close without a `wake` (the window's fifth failure, its timeout), so look again at least once a second.
        let wait = sleep_for(hub.pairing().with(|p| p.deadline_of(me.conn))).min(RECHECK);
        tokio::select! {
            _ = changed.changed() => {}
            () = tokio::time::sleep(wait) => {}
            got = conn.recv() => match got {
                Ok(Some(f)) if (f.stream_id, f.kind) == (STREAM_CONTROL, kinds::PING) => {
                    conn.send(&Frame {
                        stream_id: STREAM_CONTROL,
                        kind: kinds::PONG,
                        seq: None,
                        payload: f.payload,
                    })
                    .await?;
                }
                Ok(Some(_)) => {
                    // Anything but a ping while waiting ends the session.
                    let e = hub.pairing().with(|p| p.violation(me.conn, mono())).to_error();
                    return refuse(&mut conn, &reveal_id, &e).await;
                }
                Ok(None) | Err(_) => return Ok(()),
            },
        }
    };
    match verdict {
        Verdict::Waiting => Ok(()),
        Verdict::Approved(issue) => {
            // Issued or not, the name is let go before anything is written (a machine that stops reading must not hold it).
            let let_go = || hub.pairing().with(|p| p.issued(me.conn));
            let Some(ca) = ca else {
                let_go();
                return refuse(&mut conn, &reveal_id, &Error::new(ErrorCode::Internal)).await;
            };
            // For the frozen tuple only: the CSR must still be the one the commitment covered.
            let same = ca::csr_der(&issue.csr_pem)
                .is_ok_and(|der| <[u8; 32]>::from(Sha256::digest(&der)) == issue.csr_hash);
            if !same {
                let_go();
                return refuse(&mut conn, &reveal_id, &Refusal::Mismatch.to_error()).await;
            }
            let issued = Enrollment::new(hub.store()).issue_paired(
                ca,
                &issue.node_id,
                &issue.csr_pem,
                issue.replace,
                now_ms(),
            );
            let_go();
            match issued {
                Ok(cert) => {
                    audit(hub, &who, "pair.issued", &issue.node_id, &cert.serial_hex);
                    send(
                        &mut conn,
                        &reply(
                            &reveal_id,
                            command_ok::Result::PairResult(v1::PairResult {
                                approved: true,
                                cert_pem: cert.cert_pem,
                                ca_pem: ca.cert_pem().to_string(),
                            }),
                        ),
                    )
                    .await?;
                    conn.close().await;
                    Ok(())
                }
                Err(EnrollError::NameInUse) => {
                    audit(hub, &who, "pair.issued", &issue.node_id, "pair.state");
                    refuse(
                        &mut conn,
                        &reveal_id,
                        &Refusal::State("name_in_use").to_error(),
                    )
                    .await
                }
                Err(_) => {
                    audit(hub, &who, "pair.issued", &issue.node_id, "internal");
                    refuse(&mut conn, &reveal_id, &Error::new(ErrorCode::Internal)).await
                }
            }
        }
        Verdict::Denied => {
            send(
                &mut conn,
                &reply(
                    &reveal_id,
                    command_ok::Result::PairResult(v1::PairResult::default()),
                ),
            )
            .await?;
            conn.close().await;
            Ok(())
        }
        Verdict::Expired => refuse(&mut conn, &reveal_id, &Refusal::Expired.to_error()).await,
        Verdict::Closed => refuse(&mut conn, &reveal_id, &Refusal::Closed.to_error()).await,
    }
}

/// The owner's pairing commands, answered by the hub. `None` for any other command. Owner only; the hub must offer `pairing.v1`.
/// `PairRequest` and `PairReveal` are a machine's messages and are accepted on a pairing connection only: from a window they are
/// refused as invalid.
pub fn owner_command(
    hub: &Hub,
    who: &Access,
    req: &v1::CommandRequest,
) -> Option<v1::CommandResponse> {
    use command_request::Command as C;
    let action = match &req.command {
        Some(C::PairOpen(_)) => "pair.open",
        Some(C::PairClose(_)) => "pair.close",
        Some(C::PairStatus(_)) => "pair.status",
        Some(C::PairApprove(_)) => "pair.approve",
        Some(C::PairDeny(_)) => "pair.deny",
        Some(C::PairRequest(_) | C::PairReveal(_)) => {
            return Some(error_response(
                &req.request_id,
                &Error::new(ErrorCode::ProtocolInvalidMessage).with("field", "command"),
            ))
        }
        _ => return None,
    };
    let fail = |e: Error| Some(error_response(&req.request_id, &e));
    if !req.target_node.is_empty() {
        return fail(Error::new(ErrorCode::ProtocolInvalidMessage).with("field", "target_node"));
    }
    if who.role != Role::Owner {
        audit(hub, &who.user, action, "", "auth.forbidden");
        return fail(Error::new(ErrorCode::AuthForbidden).with("action", action));
    }
    if !hub.pairing().enabled() {
        return fail(
            Error::new(ErrorCode::ProtocolCapabilityMissing).with("capability", "pairing.v1"),
        );
    }
    report_rates(hub);
    let now = mono();
    let pairing = hub.pairing();
    let status = |p: &mut Pairing| {
        let s = p.status(now);
        command_ok::Result::PairStatus(v1::PairStatusResult {
            open: s.open,
            closes_at_ms: to_wall(s.closes_at_ms),
            pending: s.pending,
        })
    };
    let empty = || command_ok::Result::Empty(v1::Empty {});
    let result: Result<(command_ok::Result, String), Error> = match &req.command {
        Some(C::PairOpen(_)) => {
            // Spec section 5b: the certificate must name the addresses a machine will dial before the switch opens; when it cannot,
            // the switch stays closed and the owner is told which names are missing.
            let leaf = pairing.provisioner().map(|p| p.provision());
            match leaf {
                Some(Err(f)) => Err(Error::new(ErrorCode::PairNamesMissing)
                    .with("reason", f.reason)
                    .with("names", f.names.join(", ").as_str())),
                other => {
                    if let Some(Ok(hash)) = other {
                        pairing.with(|p| p.set_leaf(hash));
                    }
                    Ok((
                        pairing.with(|p| {
                            p.open(now);
                            status(p)
                        }),
                        String::new(),
                    ))
                }
            }
        }
        Some(C::PairClose(_)) => {
            pairing.with(|p| p.close(now));
            pairing.wake();
            Ok((empty(), String::new()))
        }
        Some(C::PairStatus(_)) => Ok((pairing.with(status), String::new())),
        Some(C::PairApprove(a)) => {
            let node = pairing.with(|p| {
                p.tick(now);
                p.pending_node_id(&a.pair_id).map(str::to_string)
            });
            // A node id that belongs to another machine is refused unless the owner says "replace"; the request stays pending.
            let taken = match (&node, a.replace) {
                (Some(n), false) => Enrollment::new(hub.store()).node_in_use(n),
                _ => Ok(false),
            };
            let node_name = node.clone().unwrap_or_default();
            match taken {
                Err(_) => Err(Error::new(ErrorCode::Internal)),
                Ok(true) => Err(Refusal::State("name_in_use").to_error()),
                Ok(false) => pairing
                    .with(|p| p.approve(now, &a.pair_id, a.replace))
                    .map(|()| (empty(), node_name))
                    .map_err(|r| r.to_error()),
            }
        }
        Some(C::PairDeny(d)) => {
            let node = pairing
                .with(|p| p.pending_node_id(&d.pair_id).map(str::to_string))
                .unwrap_or_default();
            pairing
                .with(|p| p.deny(now, &d.pair_id))
                .map(|()| (empty(), node))
                .map_err(|r| r.to_error())
        }
        _ => return None,
    };
    match result {
        Ok((ok, node)) => {
            if action != "pair.status" {
                audit(hub, &who.user, action, &node, "ok");
            }
            pairing.wake();
            Some(reply(&req.request_id, ok))
        }
        Err(e) => {
            audit(hub, &who.user, action, "", e.code.as_str());
            fail(e)
        }
    }
}

/// What an owner's window is told about pairing, read from the shared state: the cards waiting when it connects, then each change,
/// batched to at most one push a second (section 6). In memory only.
pub struct WindowFeed {
    cursor: u64,
    not_before: u64,
}

fn card_frame(c: &crate::pairing::Card) -> Frame {
    let kind = match c.kind {
        Kind::Machine => v1::PairKind::Machine,
        Kind::Browser => v1::PairKind::Browser,
    };
    Frame {
        stream_id: STREAM_EVENTS,
        kind: kinds::PAIR_REQUESTED,
        seq: None,
        payload: v1::PairRequested {
            pair_id: c.pair_id.to_vec(),
            kind: kind as i32,
            name: c.name.clone(),
            os: c.os.clone(),
            address: c.address.clone(),
            number: c.number.clone(),
            expires_ms: to_wall(c.expires_ms),
        }
        .encode_to_vec(),
    }
}

fn outcome_of(o: crate::pairing::Outcome) -> v1::PairOutcome {
    use crate::pairing::Outcome as O;
    match o {
        O::Approved => v1::PairOutcome::Approved,
        O::Denied => v1::PairOutcome::Denied,
        O::Expired => v1::PairOutcome::Expired,
        O::Closed => v1::PairOutcome::Closed,
    }
}

fn note_frame(n: &Note) -> Frame {
    match n {
        Note::Requested(c) => card_frame(c),
        Note::Decided { pair_id, outcome } => Frame {
            stream_id: STREAM_EVENTS,
            kind: kinds::PAIR_DECIDED,
            seq: None,
            payload: v1::PairDecided {
                pair_id: pair_id.to_vec(),
                outcome: outcome_of(*outcome) as i32,
            }
            .encode_to_vec(),
        },
    }
}

impl WindowFeed {
    /// A window that has just connected: the cards waiting now (one frame each) and the feed for what comes next.
    pub fn start(hub: &Hub) -> (Self, Vec<Frame>) {
        let (cards, cursor) = hub.pairing().with(|p| p.cards(mono()));
        (
            Self {
                cursor,
                not_before: 0,
            },
            cards.iter().map(card_frame).collect(),
        )
    }

    /// What changed since the last call; empty when nothing did or when a push went out less than a second ago.
    pub fn poll(&mut self, hub: &Hub) -> Vec<Frame> {
        let now = mono();
        if now < self.not_before {
            return Vec::new();
        }
        let (notes, next) = hub.pairing().with(|p| p.notes_after(now, self.cursor));
        self.cursor = next;
        if !notes.is_empty() {
            self.not_before = now + 1_000;
        }
        notes.iter().map(note_frame).collect()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn who(role: Role) -> Access {
        Access {
            role,
            user: "u".into(),
            projects: None,
            session: String::new(),
        }
    }

    fn req(c: command_request::Command, target: &str) -> v1::CommandRequest {
        v1::CommandRequest {
            request_id: "r".into(),
            idempotency_key: String::new(),
            target_node: target.into(),
            command: Some(c),
        }
    }

    fn code(r: Option<v1::CommandResponse>) -> ErrorCode {
        match r.unwrap().result.unwrap() {
            command_response::Result::Err(e) => ErrorCode::parse(&e.code),
            command_response::Result::Ok(_) => panic!("expected a refusal"),
        }
    }

    fn all() -> Vec<command_request::Command> {
        use command_request::Command as C;
        vec![
            C::PairOpen(v1::PairOpen {}),
            C::PairClose(v1::PairClose {}),
            C::PairStatus(v1::PairStatus {}),
            C::PairApprove(v1::PairApprove::default()),
            C::PairDeny(v1::PairDeny::default()),
        ]
    }

    fn hub(enabled: bool) -> Hub {
        let hub = Hub::open(crate::store::SqliteStore::open_in_memory().unwrap()).unwrap();
        if enabled {
            hub.pairing().enable([1; 32]);
        }
        hub
    }

    #[test]
    fn the_pairing_clock_only_goes_forward_and_maps_to_wall_time_for_windows() {
        let a = mono();
        std::thread::sleep(Duration::from_millis(5));
        let b = mono();
        assert!(b > a && a >= 1_000);
        // A moment on the pairing clock is shown as wall time close to now; zero stays zero.
        let wall = to_wall(b);
        assert!(wall.abs_diff(now_ms()) < 2_000, "{wall} vs {}", now_ms());
        assert_eq!(to_wall(0), 0);
    }

    #[test]
    fn a_hub_that_cannot_pair_answers_capability_missing_and_other_commands_are_not_ours() {
        let h = hub(false);
        for c in all() {
            assert_eq!(
                code(owner_command(&h, &who(Role::Owner), &req(c, ""))),
                ErrorCode::ProtocolCapabilityMissing
            );
        }
        assert!(owner_command(
            &h,
            &who(Role::Owner),
            &req(
                command_request::Command::AgentList(v1::AgentList::default()),
                "n"
            )
        )
        .is_none());
    }

    #[test]
    fn a_member_a_viewer_and_a_command_aimed_at_a_node_are_refused_and_nothing_opens() {
        let h = hub(true);
        for role in [Role::Member, Role::Viewer] {
            for c in all() {
                assert_eq!(
                    code(owner_command(&h, &who(role), &req(c, ""))),
                    ErrorCode::AuthForbidden
                );
            }
        }
        for c in all() {
            assert_eq!(
                code(owner_command(&h, &who(Role::Owner), &req(c, "laptop"))),
                ErrorCode::ProtocolInvalidMessage
            );
        }
        assert!(!h.pairing().with(|p| p.status(mono())).open);
        // The refusals are on the audit trail with the code.
        let rows = crate::audit::Audit::new(h.store())
            .list(0, None, 100)
            .unwrap();
        assert!(rows
            .iter()
            .any(|r| r.outcome == "auth.forbidden" && r.action == "pair.open"));
    }

    #[test]
    fn the_owner_opens_reads_and_closes_the_switch() {
        let h = hub(true);
        let o = who(Role::Owner);
        let status = |r: Option<v1::CommandResponse>| match r.unwrap().result.unwrap() {
            command_response::Result::Ok(v1::CommandOk {
                result: Some(command_ok::Result::PairStatus(s)),
            }) => s,
            other => panic!("{other:?}"),
        };
        let s = status(owner_command(
            &h,
            &o,
            &req(command_request::Command::PairOpen(v1::PairOpen {}), ""),
        ));
        assert!(s.open);
        assert!(s.closes_at_ms > now_ms());
        let s = status(owner_command(
            &h,
            &o,
            &req(command_request::Command::PairStatus(v1::PairStatus {}), ""),
        ));
        assert_eq!((s.open, s.pending), (true, 0));
        owner_command(
            &h,
            &o,
            &req(command_request::Command::PairClose(v1::PairClose {}), ""),
        )
        .unwrap();
        let s = status(owner_command(
            &h,
            &o,
            &req(command_request::Command::PairStatus(v1::PairStatus {}), ""),
        ));
        assert!(!s.open && s.closes_at_ms == 0);
    }

    /// A machine on an in-memory pairing connection that has its challenge and has not revealed yet.
    async fn challenged(hub: &std::sync::Arc<Hub>, last: u8) -> Conn<tokio::io::DuplexStream> {
        let (a, b) = tokio::io::duplex(1 << 16);
        let h = std::sync::Arc::clone(hub);
        tokio::spawn(async move {
            let mut conn = agora_protocol::ws::accept(b).await.unwrap();
            let first = conn.recv().await.unwrap().unwrap();
            let peer = PairPeer {
                addr: IpAddr::from([10, 0, 0, last]),
                exporter: [last; 32],
                leaf_hash: None,
            };
            let _ = serve_pairing(conn, &h, None, first, peer).await;
        });
        let mut m = agora_protocol::ws::connect(a, "ws://localhost/")
            .await
            .unwrap();
        let r = req(
            command_request::Command::PairRequest(v1::PairRequest {
                commitment: vec![last; 32],
                node_name: format!("m{last}"),
                os: "linux".into(),
                version: "0.0".into(),
            }),
            "",
        );
        m.send(&Frame {
            stream_id: STREAM_COMMANDS,
            kind: 0,
            seq: None,
            payload: r.encode_to_vec(),
        })
        .await
        .unwrap();
        let f = m.recv().await.unwrap().unwrap();
        let ok = v1::CommandResponse::decode(f.payload.as_slice()).unwrap();
        assert!(matches!(
            ok.result,
            Some(command_response::Result::Ok(v1::CommandOk {
                result: Some(command_ok::Result::PairChallenge(_))
            }))
        ));
        m
    }

    /// The code of the next answer that is not a pong; the machine pings every 200 ms while it waits (at most `within`).
    async fn answer_while_pinging(
        m: &mut Conn<tokio::io::DuplexStream>,
        within: Duration,
    ) -> Option<ErrorCode> {
        let until = tokio::time::Instant::now() + within;
        while tokio::time::Instant::now() < until {
            m.send(&Frame {
                stream_id: STREAM_CONTROL,
                kind: kinds::PING,
                seq: None,
                payload: Vec::new(),
            })
            .await
            .ok()?;
            let Ok(got) = tokio::time::timeout(Duration::from_millis(200), m.recv()).await else {
                continue;
            };
            let f = got.ok()??;
            if (f.stream_id, f.kind) == (STREAM_CONTROL, kinds::PONG) {
                continue;
            }
            return Some(code(Some(
                v1::CommandResponse::decode(f.payload.as_slice()).unwrap(),
            )));
        }
        None
    }

    /// An open hub with one machine that has its challenge and has not revealed, after the switch closed: by the owner's
    /// `PairClose` (`wake`), or in the state with nothing to wake the connection, as its timeout or a fifth failure closes it.
    async fn closed_before_the_reveal(wake: bool) -> Conn<tokio::io::DuplexStream> {
        let h = std::sync::Arc::new(hub(true));
        let o = who(Role::Owner);
        owner_command(
            &h,
            &o,
            &req(command_request::Command::PairOpen(v1::PairOpen {}), ""),
        );
        let m = challenged(&h, 1).await;
        if wake {
            owner_command(
                &h,
                &o,
                &req(command_request::Command::PairClose(v1::PairClose {}), ""),
            );
        } else {
            h.pairing().with(|p| p.close(mono()));
        }
        m
    }

    /// The connection ends soon after the answer (a bounded wait: a regression fails here instead of hanging).
    async fn ends(m: &mut Conn<tokio::io::DuplexStream>) -> bool {
        matches!(
            tokio::time::timeout(Duration::from_secs(4), m.recv()).await,
            Ok(Ok(None) | Err(_))
        )
    }

    /// F1460 (spec sections 2 and 12): a machine that has its challenge but has not revealed when the switch closes is told
    /// `pair.closed`, at once, not `pair.expired` at its 30-second reveal deadline.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_machine_that_has_not_revealed_when_the_switch_closes_is_told_closed_at_once() {
        // Woken by the owner's command, and (within the one-second recheck) with nothing to wake it.
        for wake in [true, false] {
            let mut m = closed_before_the_reveal(wake).await;
            let told = tokio::time::timeout(Duration::from_secs(4), m.recv())
                .await
                .unwrap_or_else(|_| {
                    panic!("not told within 4 s that the switch closed (wake: {wake})")
                })
                .unwrap()
                .unwrap();
            assert_eq!(
                code(Some(
                    v1::CommandResponse::decode(told.payload.as_slice()).unwrap()
                )),
                ErrorCode::PairClosed
            );
            // The hub ended the connection after the answer.
            assert!(ends(&mut m).await);
        }
    }

    /// F1461 (spec sections 7 and 12, "Idle"): a machine whose session ended before its reveal cannot hold its connection open by
    /// pinging; it is answered and the connection ends.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pinging_does_not_hold_open_a_machine_whose_session_ended_before_its_reveal() {
        let mut m = closed_before_the_reveal(true).await;
        assert_eq!(
            answer_while_pinging(&mut m, Duration::from_secs(4)).await,
            Some(ErrorCode::PairClosed),
            "a pinging machine was held open without an answer"
        );
        assert!(ends(&mut m).await);
    }

    #[test]
    fn a_machines_messages_from_a_window_are_invalid_whoever_sends_them() {
        let h = hub(true);
        for c in [
            command_request::Command::PairRequest(v1::PairRequest::default()),
            command_request::Command::PairReveal(v1::PairReveal::default()),
        ] {
            assert_eq!(
                code(owner_command(&h, &who(Role::Owner), &req(c, ""))),
                ErrorCode::ProtocolInvalidMessage
            );
        }
    }
}
