//! The machine's side of pairing (ADR-0065, `docs/specs/pairing.md` sections 5 and 5a): dial a hub whose CA this machine does not
//! know yet, agree on a short number with it, wait for the owner to compare and approve, then check what came back before keeping
//! anything.
//!
//! Two parts, so the part that decides who may join can be read and tested on its own:
//! - [`Exchange`] is the protocol as a plain state machine: no sockets, no clock, no randomness of its own. It builds the
//!   commitment, checks the hub's challenge, computes the number and turns the hub's answer into the certificate and CA.
//! - [`pair`] is the connection around it: TLS 1.3 with a verifier that is **private to this module** (it accepts a leaf it cannot
//!   chain, and nothing else), the exporter and leaf hash read from that connection, the timeouts, and the final check of the
//!   certificate. It returns only what passed that check; a failure stores nothing because this module stores nothing.
//!
//! What is never kept or shown: the nonces, the commitment, the exporter and the number are values of this process's memory. The
//! number goes to the caller's `show` closure once, for the person; no type here prints it in `Debug`, and nothing writes it down.

use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agora_protocol::frame::STREAM_COMMANDS;
use agora_protocol::v1::{
    command_ok, command_request, command_response, CommandRequest, CommandResponse, PairChallenge,
    PairRequest, PairResult,
};
use agora_protocol::ws::{self, TransportError};
use agora_protocol::{ErrorCode, Frame};
use prost::Message;
use ring::{digest, hkdf};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use x509_parser::prelude::FromDer;

/// The label of the TLS exporter both sides read (section 5): RFC 9266's construction with this protocol's own label.
const EXPORTER_LABEL: &[u8] = b"EXPORTER-agora-pair-v1";

/// The port a hub listens on when the person typed no port (the front door's default, ADR-0062).
pub const DEFAULT_PORT: u16 = 7443;

/// The six digits both screens show, `482193`. Its `Debug` hides it, so a log line or a panic message that formats a value that holds
/// it cannot leak it; the only way to read it is [`Number::digits`] or [`Number::grouped`], which the caller uses to show the person.
#[derive(Clone, PartialEq, Eq)]
pub struct Number(String);

impl Number {
    /// `482193`.
    pub fn digits(&self) -> &str {
        &self.0
    }

    /// `482-193`, as people read it.
    pub fn grouped(&self) -> String {
        format!("{}-{}", &self.0[..3], &self.0[3..])
    }
}

impl std::fmt::Debug for Number {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Number(..)")
    }
}

/// Why pairing did not finish. Nothing was stored in any of these cases (this module stores nothing).
#[derive(Debug, PartialEq, Eq)]
pub enum PairError {
    /// The address could not be understood as `host` or `host:port`.
    BadAddress,
    /// The hub could not be reached (nobody listens, the name does not resolve, the network is down).
    Unreachable(String),
    /// The hub does not speak TLS 1.3; pairing needs the exporter and refuses anything older (section 5).
    Tls13Required,
    /// The handshake failed for another reason, for example a peer that could not prove it holds its certificate's key.
    TlsFailed(String),
    /// The hub did not answer in time. `waiting_for` is `"challenge"` or `"decision"`.
    Timeout { waiting_for: &'static str },
    /// The hub refused with a stable error code. `state` and `seconds` are the parameters some codes carry.
    Hub {
        code: String,
        state: Option<String>,
        seconds: Option<i64>,
    },
    /// The owner pressed Deny.
    Denied,
    /// The hub closed the connection without an answer.
    Closed,
    /// The hub sent something the protocol does not allow at this step. The session is over.
    Unexpected,
    /// `pair.name_unverified` (machine-local, section 5 step 6b): the address that was dialled is not a name in the hub's certificate,
    /// or that certificate does not chain to the CA the hub delivered. Nothing is kept.
    NameUnverified { name: String },
    /// The certificate the hub delivered is not for this machine's key and node id, or does not chain to the CA it delivered.
    CertificateInvalid(&'static str),
}

/// A pairing that finished and passed every check (section 5 step 6): the certificate for this machine's own key, the CA it chains to
/// and the CA's fingerprint, which the person can compare with the hub's.
#[derive(Debug)]
pub struct Verified {
    pub cert_pem: String,
    pub ca_pem: String,
    /// SHA-256 of the CA certificate's DER, lowercase hex.
    pub ca_fingerprint: String,
    /// The name that was dialled (the host part of the address), for the link folder.
    pub host: String,
}

// ---- the arithmetic ----

fn sha256(parts: &[&[u8]]) -> [u8; 32] {
    let mut c = digest::Context::new(&digest::SHA256);
    for p in parts {
        c.update(p);
    }
    let d = c.finish();
    let mut out = [0u8; 32];
    out.copy_from_slice(d.as_ref());
    out
}

/// `SHA-256("agora/pair/v1/commit" || nonce_n || csr_hash)` (section 5, step 1).
pub fn commitment(nonce_n: &[u8; 16], csr_hash: &[u8; 32]) -> [u8; 32] {
    sha256(&[b"agora/pair/v1/commit", nonce_n, csr_hash])
}

struct EightBytes;

impl hkdf::KeyType for EightBytes {
    fn len(&self) -> usize {
        8
    }
}

/// The number (section 5, step 4): `HKDF-SHA256(salt = "agora/pair/v1", ikm = exporter || leaf_hash || csr_hash || nonce_n || nonce_h,
/// info = "sas")`, its first 8 bytes as a big-endian integer, modulo 10^6. The hub has its own implementation
/// (`agora_hub::pairing::sas`); both are checked against the same vector, computed outside the code.
pub fn sas(
    exporter: &[u8; 32],
    leaf_hash: &[u8; 32],
    csr_hash: &[u8; 32],
    nonce_n: &[u8; 16],
    nonce_h: &[u8; 16],
) -> Number {
    let mut ikm = Vec::with_capacity(32 * 3 + 16 * 2);
    for part in [
        &exporter[..],
        &leaf_hash[..],
        &csr_hash[..],
        &nonce_n[..],
        &nonce_h[..],
    ] {
        ikm.extend_from_slice(part);
    }
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, b"agora/pair/v1").extract(&ikm);
    let mut out = [0u8; 8];
    // Eight bytes are far below the limit (255 x 32), so expanding cannot fail; a failure would give a number that cannot match,
    // which the owner sees as two different numbers and denies.
    if let Ok(okm) = prk.expand(&[b"sas"], EightBytes) {
        if okm.fill(&mut out).is_err() {
            out = [0; 8];
        }
    }
    Number(format!("{:06}", u64::from_be_bytes(out) % 1_000_000))
}

// ---- the state machine ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Nothing sent.
    New,
    /// `PairRequest` sent; the challenge is next.
    Requested,
    /// `PairReveal` sent; the owner's answer is next.
    Revealed,
    /// An answer was taken (or the session broke a rule). Nothing more is accepted.
    Over,
}

/// The messages of one pairing, in order, and what must hold at each step. One value is one session: a retry is a new value with a
/// fresh nonce (section 5a). It holds the secrets of the exchange and does not implement `Debug`.
pub struct Exchange {
    phase: Phase,
    exporter: [u8; 32],
    leaf_hash: [u8; 32],
    csr_hash: [u8; 32],
    nonce_n: [u8; 16],
}

impl Exchange {
    /// `exporter` and `leaf_hash` are what this machine's side of the connection saw; `csr_der` is the CSR it will send, `nonce_n`
    /// 16 fresh random bytes that are never reused.
    pub fn new(exporter: [u8; 32], leaf_hash: [u8; 32], csr_der: &[u8], nonce_n: [u8; 16]) -> Self {
        Self {
            phase: Phase::New,
            exporter,
            leaf_hash,
            csr_hash: sha256(&[csr_der]),
            nonce_n,
        }
    }

    /// Step 1: the commitment. Only once, and first.
    pub fn request(
        &mut self,
        node_name: &str,
        os: &str,
        version: &str,
    ) -> Result<PairRequest, PairError> {
        if self.phase != Phase::New {
            return self.broken();
        }
        self.phase = Phase::Requested;
        Ok(PairRequest {
            commitment: commitment(&self.nonce_n, &self.csr_hash).to_vec(),
            node_name: node_name.into(),
            os: os.into(),
            version: version.into(),
        })
    }

    /// Step 3 and 4: the hub's challenge in, the reveal and the number out. A challenge of the wrong shape ends the session.
    pub fn challenge(
        &mut self,
        c: &PairChallenge,
        csr_pem: &str,
    ) -> Result<(agora_protocol::v1::PairReveal, Number), PairError> {
        if self.phase != Phase::Requested {
            return self.broken();
        }
        let (Ok(nonce_h), true) = (
            <[u8; 16]>::try_from(c.nonce_h.as_slice()),
            c.pair_id.len() == 16,
        ) else {
            return self.broken();
        };
        self.phase = Phase::Revealed;
        let number = sas(
            &self.exporter,
            &self.leaf_hash,
            &self.csr_hash,
            &self.nonce_n,
            &nonce_h,
        );
        Ok((
            agora_protocol::v1::PairReveal {
                pair_id: c.pair_id.clone(),
                nonce_n: self.nonce_n.to_vec(),
                csr_pem: csr_pem.into(),
            },
            number,
        ))
    }

    /// Step 5: the owner's answer. Once; a second answer, or one before the reveal, ends the session. A Deny is `Denied`; an approval
    /// that carries no certificate or no CA is `Unexpected` (nothing to keep).
    pub fn result(&mut self, r: &PairResult) -> Result<(String, String), PairError> {
        if self.phase != Phase::Revealed {
            return self.broken();
        }
        self.phase = Phase::Over;
        if !r.approved {
            return Err(PairError::Denied);
        }
        if r.cert_pem.is_empty() || r.ca_pem.is_empty() {
            return Err(PairError::Unexpected);
        }
        Ok((r.cert_pem.clone(), r.ca_pem.clone()))
    }

    fn broken<T>(&mut self) -> Result<T, PairError> {
        self.phase = Phase::Over;
        Err(PairError::Unexpected)
    }
}

// ---- the check of what the hub delivered (section 5, step 6) ----

/// Splits what the person typed, `host`, `host:port`, `[v6]` or `[v6]:port`, into the host and the address to dial (the default
/// port when none was typed). `None` when it is empty or not an address at all.
pub fn split_address(typed: &str) -> Option<(String, String)> {
    let t = typed.trim();
    let t = t.strip_prefix("wss://").unwrap_or(t).trim_end_matches('/');
    if t.is_empty() || t.contains(['/', ' ', '\t', '@']) {
        return None;
    }
    let (host, port) = if let Some(rest) = t.strip_prefix('[') {
        let (h, after) = rest.split_once(']')?;
        let port = match after {
            "" => DEFAULT_PORT,
            a => a.strip_prefix(':')?.parse().ok()?,
        };
        (h.to_string(), port)
    } else if t.parse::<IpAddr>().is_ok() {
        // A bare IPv6 address has colons that are not a port.
        (t.to_string(), DEFAULT_PORT)
    } else {
        match t.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse().ok()?),
            None => (t.to_string(), DEFAULT_PORT),
        }
    };
    if host.is_empty() || port == 0 {
        return None;
    }
    let addr = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    Some((host, addr))
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn roots_of(ca_pem: &str) -> Result<rustls::RootCertStore, PairError> {
    let mut roots = rustls::RootCertStore::empty();
    let mut count = 0;
    for c in CertificateDer::pem_slice_iter(ca_pem.as_bytes()) {
        let c = c.map_err(|_| PairError::CertificateInvalid("ca"))?;
        roots
            .add(c)
            .map_err(|_| PairError::CertificateInvalid("ca"))?;
        count += 1;
    }
    // One CA, perhaps with its previous generation; never a bundle the hub could use to widen what this machine trusts.
    if count == 0 || count > 2 {
        return Err(PairError::CertificateInvalid("ca"));
    }
    Ok(roots)
}

/// The lower-case hex of the first certificate's SHA-256 in `ca_pem`: the fingerprint the person can compare with the hub's.
pub fn ca_fingerprint(ca_pem: &str) -> Option<String> {
    let der = CertificateDer::pem_slice_iter(ca_pem.as_bytes())
        .next()?
        .ok()?;
    Some(
        sha256(&[der.as_ref()])
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    )
}

/// Everything section 5 step 6 asks before a machine keeps a certificate. All of it must hold, or nothing is kept:
/// (b) the leaf the handshake showed chains to `ca_pem` (the only root) and `host` is one of its names (`pair.name_unverified`
/// otherwise); (a) the certificate chains to the same CA, is not expired, names `node_id`, and carries the public key of this machine's
/// own CSR (the private key never left, so a certificate for any other key is of no use and means the hub is not what it says).
pub fn check_delivery(
    ca_pem: &str,
    cert_pem: &str,
    leaf_der: &[u8],
    host: &str,
    node_id: &str,
    csr_der: &[u8],
) -> Result<String, PairError> {
    use rustls::client::WebPkiServerVerifier;
    use rustls::server::WebPkiClientVerifier;
    let roots = Arc::new(roots_of(ca_pem)?);
    let now = UnixTime::now();

    // (b) The hub's own leaf, as the handshake showed it, against the CA the hub just delivered and the name that was dialled.
    let unverified = || PairError::NameUnverified {
        name: host.to_string(),
    };
    let name = ServerName::try_from(host.to_string()).map_err(|_| unverified())?;
    let leaf_ok = WebPkiServerVerifier::builder_with_provider(Arc::clone(&roots), provider())
        .build()
        .map_err(|_| PairError::CertificateInvalid("ca"))?
        .verify_server_cert(
            &CertificateDer::from(leaf_der.to_vec()),
            &[],
            &name,
            &[],
            now,
        )
        .is_ok();
    if !leaf_ok {
        return Err(unverified());
    }

    // (a) The certificate this machine will use.
    let der = CertificateDer::pem_slice_iter(cert_pem.as_bytes())
        .next()
        .ok_or(PairError::CertificateInvalid("certificate"))?
        .map_err(|_| PairError::CertificateInvalid("certificate"))?;
    WebPkiClientVerifier::builder_with_provider(roots, provider())
        .build()
        .map_err(|_| PairError::CertificateInvalid("ca"))?
        .verify_client_cert(&der, &[], now)
        .map_err(|_| PairError::CertificateInvalid("chain"))?;
    let (_, cert) = x509_parser::parse_x509_certificate(der.as_ref())
        .map_err(|_| PairError::CertificateInvalid("certificate"))?;
    let subject = cert
        .subject()
        .iter_common_name()
        .next()
        .and_then(|cn| cn.as_str().ok());
    if subject != Some(node_id) {
        return Err(PairError::CertificateInvalid("subject"));
    }
    let (_, csr) = x509_parser::certification_request::X509CertificationRequest::from_der(csr_der)
        .map_err(|_| PairError::CertificateInvalid("request"))?;
    if csr.certification_request_info.subject_pki.raw != cert.tbs_certificate.subject_pki.raw {
        return Err(PairError::CertificateInvalid("key"));
    }
    ca_fingerprint(ca_pem).ok_or(PairError::CertificateInvalid("ca"))
}

// ---- the connection ----

/// A verifier that accepts a leaf this machine cannot chain to anything yet, and remembers it (section 5). It is the only way the
/// pairing connection differs from an ordinary one, and it relaxes exactly one check, **trust in the issuer**:
/// - the TLS 1.3 handshake signature is verified with the crypto provider's own check against the leaf's key (a party that merely
///   replays a leaf it saw cannot sign, so it cannot pass); it never answers "valid" without checking;
/// - an empty leaf is refused; TLS 1.2 signatures are refused (pairing is TLS 1.3 only, and the config offers nothing else).
///
/// Private to this module, so the node link and every other connection cannot pick it up by accident.
#[derive(Debug)]
struct UnchainedLeaf {
    provider: Arc<rustls::crypto::CryptoProvider>,
    seen: Mutex<Option<Vec<u8>>>,
}

impl ServerCertVerifier for UnchainedLeaf {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if end_entity.is_empty() {
            return Err(rustls::Error::NoCertificatesPresented);
        }
        *self.seen.lock().unwrap_or_else(|e| e.into_inner()) = Some(end_entity.as_ref().to_vec());
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::PeerIncompatible(
            rustls::PeerIncompatible::Tls12NotOffered,
        ))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// How long each wait lasts. The defaults follow the hub's own limits (section 7).
#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    /// Connecting and the TLS handshake.
    pub connect: Duration,
    /// The hub's challenge after the request, and any answer that is not the owner's decision.
    pub answer: Duration,
    /// The owner's decision: the hub's session lasts 5 minutes from the reveal, so this is a little more.
    pub decision: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(10),
            answer: Duration::from_secs(15),
            decision: Duration::from_secs(5 * 60 + 30),
        }
    }
}

/// What this machine asks for.
pub struct Wanted<'a> {
    /// What the person typed: `host`, `host:port`, `[v6]:port`.
    pub address: &'a str,
    /// The node id this machine asks to be (the hub checks it by the rules enrolment uses).
    pub node_name: &'a str,
    pub os: &'a str,
    pub version: &'a str,
    /// The CSR of the key this machine generated and keeps; the private key is never given to this module.
    pub csr_pem: &'a str,
    /// The CSR's DER (the bytes whose hash is committed to).
    pub csr_der: &'a [u8],
}

fn tls_failure(e: &std::io::Error) -> PairError {
    use rustls::{AlertDescription, Error};
    let inner = e.get_ref().and_then(|i| i.downcast_ref::<Error>());
    match inner {
        Some(Error::PeerIncompatible(_))
        | Some(Error::AlertReceived(AlertDescription::ProtocolVersion)) => PairError::Tls13Required,
        Some(other) => PairError::TlsFailed(other.to_string()),
        None => PairError::TlsFailed(e.to_string()),
    }
}

fn hub_error(e: &agora_protocol::v1::ErrorInfo) -> PairError {
    use agora_protocol::v1::param_value::Value;
    let param = |k: &str| e.params.get(k).and_then(|v| v.value.as_ref());
    PairError::Hub {
        code: e.code.clone(),
        state: match param("state") {
            Some(Value::Str(s)) => Some(s.clone()),
            _ => None,
        },
        seconds: match param("seconds") {
            Some(Value::Int(i)) => Some(*i),
            _ => None,
        },
    }
}

/// Sends `command` and waits for the hub's answer.
async fn ask<S>(
    conn: &mut ws::Conn<S>,
    send: Option<command_request::Command>,
    wait: Duration,
    waiting_for: &'static str,
) -> Result<command_ok::Result, PairError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    if let Some(command) = send {
        let req = CommandRequest {
            request_id: "pair".into(),
            idempotency_key: String::new(),
            target_node: String::new(),
            command: Some(command),
        };
        conn.send(&Frame {
            stream_id: STREAM_COMMANDS,
            kind: 0,
            seq: None,
            payload: req.encode_to_vec(),
        })
        .await
        .map_err(|_| PairError::Closed)?;
    }
    let f = match tokio::time::timeout(wait, conn.recv()).await {
        Err(_) => return Err(PairError::Timeout { waiting_for }),
        Ok(Ok(Some(f))) => f,
        Ok(Ok(None)) => return Err(PairError::Closed),
        Ok(Err(TransportError::Refused(code))) => {
            return Err(PairError::Hub {
                code,
                state: None,
                seconds: None,
            })
        }
        Ok(Err(_)) => return Err(PairError::Closed),
    };
    if (f.stream_id, f.kind) != (STREAM_COMMANDS, 1) {
        return Err(PairError::Unexpected);
    }
    let r = CommandResponse::decode(f.payload.as_slice()).map_err(|_| PairError::Unexpected)?;
    match r.result {
        Some(command_response::Result::Ok(ok)) => ok.result.ok_or(PairError::Unexpected),
        Some(command_response::Result::Err(e)) => Err(hub_error(&e)),
        None => Err(PairError::Unexpected),
    }
}

async fn run(
    w: &Wanted<'_>,
    t: &Timeouts,
    show: &mut dyn FnMut(&Number),
) -> Result<Verified, PairError> {
    let (host, addr) = split_address(w.address).ok_or(PairError::BadAddress)?;
    let verifier = Arc::new(UnchainedLeaf {
        provider: provider(),
        seen: Mutex::new(None),
    });
    // TLS 1.3 only: no renegotiation, no extended-master-secret question, and the exporter the number depends on.
    let cfg = rustls::ClientConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| PairError::TlsFailed(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(verifier.clone())
        .with_no_client_auth();
    let name = ServerName::try_from(host.clone()).map_err(|_| PairError::BadAddress)?;
    let tcp = match tokio::time::timeout(t.connect, agora_protocol::dial::connect(&addr)).await {
        Err(_) => return Err(PairError::Unreachable("timed out".into())),
        Ok(Err(e)) => return Err(PairError::Unreachable(e.to_string())),
        Ok(Ok(s)) => s,
    };
    let tls = match tokio::time::timeout(
        t.connect,
        tokio_rustls::TlsConnector::from(Arc::new(cfg)).connect(name, tcp),
    )
    .await
    {
        Err(_) => return Err(PairError::Unreachable("timed out".into())),
        Ok(Err(e)) => return Err(tls_failure(&e)),
        Ok(Ok(s)) => s,
    };
    // Session binding: read before any pairing message, from this side of this connection.
    let (exporter, leaf_der) = {
        let conn = tls.get_ref().1;
        if conn.protocol_version() != Some(rustls::ProtocolVersion::TLSv1_3) {
            return Err(PairError::Tls13Required);
        }
        let exporter: [u8; 32] = conn
            .export_keying_material([0u8; 32], EXPORTER_LABEL, Some(b""))
            .map_err(|e| PairError::TlsFailed(e.to_string()))?;
        let seen = verifier
            .seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| PairError::TlsFailed("no certificate".into()))?;
        (exporter, seen)
    };
    let leaf_hash = sha256(&[&leaf_der]);
    let mut conn = ws::connect(tls, &format!("wss://{addr}/"))
        .await
        .map_err(|_| PairError::Closed)?;

    let mut nonce_n = [0u8; 16];
    getrandom::getrandom(&mut nonce_n)
        .map_err(|e| PairError::TlsFailed(format!("no randomness: {e}")))?;
    let mut ex = Exchange::new(exporter, leaf_hash, w.csr_der, nonce_n);

    let request = ex.request(w.node_name, w.os, w.version)?;
    let challenge = match ask(
        &mut conn,
        Some(command_request::Command::PairRequest(request)),
        t.answer,
        "challenge",
    )
    .await?
    {
        command_ok::Result::PairChallenge(c) => c,
        _ => return Err(PairError::Unexpected),
    };
    let (reveal, number) = ex.challenge(&challenge, w.csr_pem)?;
    ask_reveal(&mut conn, reveal).await?;
    // From here the owner decides. The person is shown the number now, not at the end.
    show(&number);
    drop(number);
    let result = match ask(&mut conn, None, t.decision, "decision").await? {
        command_ok::Result::PairResult(r) => r,
        _ => return Err(PairError::Unexpected),
    };
    let (cert_pem, ca_pem) = ex.result(&result)?;
    let ca_fingerprint =
        check_delivery(&ca_pem, &cert_pem, &leaf_der, &host, w.node_name, w.csr_der)?;
    conn.close().await;
    Ok(Verified {
        cert_pem,
        ca_pem,
        ca_fingerprint,
        host,
    })
}

async fn ask_reveal<S>(
    conn: &mut ws::Conn<S>,
    reveal: agora_protocol::v1::PairReveal,
) -> Result<(), PairError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let req = CommandRequest {
        request_id: "pair-reveal".into(),
        idempotency_key: String::new(),
        target_node: String::new(),
        command: Some(command_request::Command::PairReveal(reveal)),
    };
    conn.send(&Frame {
        stream_id: STREAM_COMMANDS,
        kind: 0,
        seq: None,
        payload: req.encode_to_vec(),
    })
    .await
    .map_err(|_| PairError::Closed)
}

/// Pairs this machine with the hub at `w.address`: blocks the calling thread until the owner decided or something failed. `show` is
/// called once, with the number, as soon as the reveal is sent; it is where the person sees it, and nothing else in this crate
/// writes it anywhere. On `Ok` every check of section 5 step 6 passed.
pub fn pair(
    w: &Wanted<'_>,
    t: &Timeouts,
    show: &mut dyn FnMut(&Number),
) -> Result<Verified, PairError> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| PairError::Unreachable(e.to_string()))?;
    rt.block_on(run(w, t, show))
}

/// The stable error code to show for a failure that carries one, so a caller can find its sentence in `locales/`.
impl PairError {
    pub fn error_code(&self) -> Option<ErrorCode> {
        match self {
            Self::Hub { code, .. } => Some(ErrorCode::parse(code)),
            Self::Denied => Some(ErrorCode::PairDenied),
            Self::NameUnverified { .. } => Some(ErrorCode::PairNameUnverified),
            _ => None,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn exchange() -> Exchange {
        Exchange::new([1; 32], [2; 32], b"csr", [4; 16])
    }

    fn challenge() -> PairChallenge {
        PairChallenge {
            pair_id: vec![7; 16],
            nonce_h: vec![5; 16],
        }
    }

    fn approved() -> PairResult {
        PairResult {
            approved: true,
            cert_pem: "cert".into(),
            ca_pem: "ca".into(),
        }
    }

    /// The same vector the hub's own function is checked against (computed outside the code, with hkdf-extract/expand and hmac-sha256).
    #[test]
    fn the_number_matches_the_vector_the_hub_is_checked_against() {
        let n = sas(&[1; 32], &[2; 32], &[3; 32], &[4; 16], &[5; 16]);
        assert_eq!(n.digits(), "497259");
        assert_eq!(n.grouped(), "497-259");
    }

    #[test]
    fn the_number_depends_on_every_input() {
        let base = sas(&[1; 32], &[2; 32], &[3; 32], &[4; 16], &[5; 16]);
        for other in [
            sas(&[9; 32], &[2; 32], &[3; 32], &[4; 16], &[5; 16]),
            sas(&[1; 32], &[9; 32], &[3; 32], &[4; 16], &[5; 16]),
            sas(&[1; 32], &[2; 32], &[9; 32], &[4; 16], &[5; 16]),
            sas(&[1; 32], &[2; 32], &[3; 32], &[9; 16], &[5; 16]),
            sas(&[1; 32], &[2; 32], &[3; 32], &[4; 16], &[9; 16]),
        ] {
            assert_ne!(base, other);
        }
    }

    #[test]
    fn the_number_never_shows_in_debug_output() {
        let n = sas(&[1; 32], &[2; 32], &[3; 32], &[4; 16], &[5; 16]);
        assert!(!format!("{n:?}").contains("497"));
        let e: Result<(), PairError> = Err(PairError::Unexpected);
        assert!(!format!("{e:?}").contains("497"));
    }

    #[test]
    fn the_commitment_is_the_hash_of_label_nonce_and_csr_hash() {
        let csr_hash = sha256(&[b"csr"]);
        let mut e = exchange();
        let req = e.request("laptop", "linux", "0.3").unwrap();
        assert_eq!(req.commitment, commitment(&[4; 16], &csr_hash).to_vec());
        assert_eq!(
            req.commitment,
            sha256(&[b"agora/pair/v1/commit", &[4; 16], &csr_hash]).to_vec()
        );
        assert_eq!(
            (req.node_name.as_str(), req.os.as_str()),
            ("laptop", "linux")
        );
    }

    #[test]
    fn the_messages_go_in_order_and_each_once() {
        let mut e = exchange();
        // A challenge before a request, and an answer before a reveal, end the session.
        assert_eq!(
            e.challenge(&challenge(), "csr").unwrap_err(),
            PairError::Unexpected
        );
        let mut e = exchange();
        e.request("a", "b", "c").unwrap();
        assert_eq!(e.result(&approved()).unwrap_err(), PairError::Unexpected);

        let mut e = exchange();
        e.request("a", "b", "c").unwrap();
        // A second request is not a retry.
        assert_eq!(e.request("a", "b", "c").unwrap_err(), PairError::Unexpected);

        let mut e = exchange();
        e.request("a", "b", "c").unwrap();
        let (reveal, number) = e.challenge(&challenge(), "the csr").unwrap();
        assert_eq!((reveal.pair_id.len(), reveal.nonce_n.len()), (16, 16));
        assert_eq!(reveal.nonce_n, vec![4u8; 16]);
        assert_eq!(reveal.csr_pem, "the csr");
        assert_eq!(
            number,
            sas(&[1; 32], &[2; 32], &sha256(&[b"csr"]), &[4; 16], &[5; 16])
        );
        // A second challenge ends it; nothing is accepted afterwards.
        assert_eq!(
            e.challenge(&challenge(), "x").unwrap_err(),
            PairError::Unexpected
        );
        assert_eq!(e.result(&approved()).unwrap_err(), PairError::Unexpected);
    }

    #[test]
    fn a_challenge_of_the_wrong_shape_ends_the_session() {
        for bad in [
            PairChallenge {
                pair_id: vec![7; 15],
                nonce_h: vec![5; 16],
            },
            PairChallenge {
                pair_id: vec![7; 16],
                nonce_h: vec![5; 17],
            },
            PairChallenge::default(),
        ] {
            let mut e = exchange();
            e.request("a", "b", "c").unwrap();
            assert_eq!(e.challenge(&bad, "csr").unwrap_err(), PairError::Unexpected);
            assert_eq!(
                e.challenge(&challenge(), "csr").unwrap_err(),
                PairError::Unexpected
            );
        }
    }

    #[test]
    fn the_owners_answer_is_taken_once() {
        let mut e = exchange();
        e.request("a", "b", "c").unwrap();
        e.challenge(&challenge(), "csr").unwrap();
        assert_eq!(
            e.result(&approved()).unwrap(),
            ("cert".to_string(), "ca".to_string())
        );
        assert_eq!(e.result(&approved()).unwrap_err(), PairError::Unexpected);
    }

    #[test]
    fn a_deny_and_an_empty_approval_give_nothing_to_keep() {
        let go = |r: &PairResult| {
            let mut e = exchange();
            e.request("a", "b", "c").unwrap();
            e.challenge(&challenge(), "csr").unwrap();
            e.result(r)
        };
        assert_eq!(go(&PairResult::default()).unwrap_err(), PairError::Denied);
        let mut no_cert = approved();
        no_cert.cert_pem.clear();
        assert_eq!(go(&no_cert).unwrap_err(), PairError::Unexpected);
        let mut no_ca = approved();
        no_ca.ca_pem.clear();
        assert_eq!(go(&no_ca).unwrap_err(), PairError::Unexpected);
    }

    #[test]
    fn addresses_split_into_a_host_and_a_dial_address() {
        let ok = |s: &str| split_address(s);
        assert_eq!(
            ok("hub.lan"),
            Some(("hub.lan".into(), "hub.lan:7443".into()))
        );
        assert_eq!(
            ok("192.168.1.5:9000"),
            Some(("192.168.1.5".into(), "192.168.1.5:9000".into()))
        );
        assert_eq!(
            ok(" wss://hub.lan:1/ "),
            Some(("hub.lan".into(), "hub.lan:1".into()))
        );
        assert_eq!(ok("[::1]"), Some(("::1".into(), "[::1]:7443".into())));
        assert_eq!(
            ok("[fe80::1]:9"),
            Some(("fe80::1".into(), "[fe80::1]:9".into()))
        );
        assert_eq!(ok("::1"), Some(("::1".into(), "[::1]:7443".into())));
        for bad in [
            "", "  ", ":7443", "hub:0", "hub:x", "a b", "user@hub", "hub/path", "[::1",
        ] {
            assert_eq!(ok(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_ca_fingerprint_is_the_hash_of_the_first_certificate() {
        assert_eq!(ca_fingerprint(""), None);
        assert_eq!(ca_fingerprint("not a certificate"), None);
    }
}
