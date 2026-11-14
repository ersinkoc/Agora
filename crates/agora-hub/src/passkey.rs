//! Passkeys (WebAuthn, ADR-0015): the owner signs in by unlocking a key held by their device instead of typing a code.
//!
//! Only what the hub needs is implemented: ES256 (ECDSA P-256) credentials, no attestation. The browser hands over the
//! public key (`getPublicKey()`, SPKI) when a passkey is added by someone already signed in, so no CBOR is parsed. A
//! sign-in checks the challenge (single use, short lived), the origin, the relying party id hash, that the user was
//! present and verified, the signature counter, and the signature itself. The relying party id is the host the browser
//! used; the browser itself refuses an id that is not its own origin.

use std::sync::Mutex;

use ring::signature::{UnparsedPublicKey, ECDSA_P256_SHA256_ASN1};
use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::store::SqliteStore;

/// How long a challenge may wait for its answer.
pub const CHALLENGE_TTL_MS: u64 = 5 * 60 * 1000;
/// Challenges waiting at once (older ones are dropped first): bounds memory against a flood of `begin` calls.
const MAX_PENDING: usize = 32;
/// Passkeys one hub keeps.
pub const MAX_PASSKEYS: i64 = 32;

#[derive(Debug, PartialEq, Eq)]
pub enum PasskeyError {
    /// Anything wrong with the request or the proof: not told apart on purpose.
    Refused,
    Store(String),
}

impl From<rusqlite::Error> for PasskeyError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Store(e.to_string())
    }
}

impl From<crate::store::StoreError> for PasskeyError {
    fn from(e: crate::store::StoreError) -> Self {
        Self::Store(e.to_string())
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

pub fn b64url_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..=c.len() {
            out.push(B64[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

/// Decodes unpadded base64url; anything else (padding, other characters) is `None`.
pub fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for ch in s.bytes() {
        let v = B64.iter().position(|&b| b == ch)? as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    // A single leftover character (6 bits) cannot come from any byte string.
    (s.len() % 4 != 1).then_some(out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Register,
    Login,
}

/// Challenges handed out and not yet answered.
#[derive(Default)]
pub struct Challenges {
    pending: Mutex<Vec<(Purpose, String, u64)>>,
}

impl Challenges {
    pub fn new() -> Self {
        Self::default()
    }

    /// A new random challenge (base64url of 32 bytes).
    pub fn issue(&self, purpose: Purpose, now_ms: u64) -> Result<String, PasskeyError> {
        let mut raw = [0u8; 32];
        getrandom::getrandom(&mut raw).map_err(|e| PasskeyError::Store(e.to_string()))?;
        let challenge = b64url_encode(&raw);
        let mut p = self.pending.lock().map_err(|_| PasskeyError::Refused)?;
        p.retain(|(_, _, exp)| *exp > now_ms);
        while p.len() >= MAX_PENDING {
            p.remove(0);
        }
        p.push((
            purpose,
            challenge.clone(),
            now_ms.saturating_add(CHALLENGE_TTL_MS),
        ));
        Ok(challenge)
    }

    /// Spends a challenge: true once, only for the purpose it was issued for, only while it is fresh.
    pub fn take(&self, purpose: Purpose, challenge: &str, now_ms: u64) -> bool {
        let Ok(mut p) = self.pending.lock() else {
            return false;
        };
        let Some(i) = p
            .iter()
            .position(|(pu, c, _)| *pu == purpose && c == challenge)
        else {
            return false;
        };
        let (_, _, exp) = p.remove(i);
        exp > now_ms
    }
}

/// The relying party id for a `Host` header value: the host without its port.
pub fn rp_id(host: &str) -> &str {
    if host.starts_with('[') {
        return host.split(']').next().map_or(host, |h| &h[1..]);
    }
    host.split(':').next().unwrap_or(host)
}

// A P-256 SubjectPublicKeyInfo is this 26-byte header followed by the 65-byte uncompressed point.
pub const SPKI_P256_PREFIX: [u8; 26] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a,
    0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];

fn point_from_spki(spki: &[u8]) -> Option<Vec<u8>> {
    let point = spki.strip_prefix(&SPKI_P256_PREFIX[..])?;
    (point.len() == 65 && point[0] == 4).then(|| point.to_vec())
}

/// Checks `clientDataJSON`: its type, the (single use) challenge and the origin. Returns the raw bytes' hash input.
fn check_client_data(
    client_data: &[u8],
    kind: &str,
    purpose: Purpose,
    challenges: &Challenges,
    host: &str,
    now_ms: u64,
) -> bool {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(client_data) else {
        return false;
    };
    let s = |k: &str| v.get(k).and_then(|x| x.as_str());
    let origin_ok = s("origin").is_some_and(|o| {
        o.strip_prefix("https://")
            .or_else(|| o.strip_prefix("http://"))
            == Some(host)
    });
    s("type") == Some(kind)
        && origin_ok
        && s("challenge").is_some_and(|c| challenges.take(purpose, c, now_ms))
}

/// Adds a passkey. The caller has already checked that the request comes from a signed-in session.
#[allow(clippy::too_many_arguments)]
pub fn register(
    store: &SqliteStore,
    challenges: &Challenges,
    host: &str,
    id: &str,
    public_key_spki: &[u8],
    client_data: &[u8],
    now_ms: u64,
    user: Option<&str>,
) -> Result<(), PasskeyError> {
    let id_ok = !id.is_empty()
        && id.len() <= 1024
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_');
    let key = point_from_spki(public_key_spki);
    if !id_ok
        || !check_client_data(
            client_data,
            "webauthn.create",
            Purpose::Register,
            challenges,
            host,
            now_ms,
        )
    {
        return Err(PasskeyError::Refused);
    }
    let Some(key) = key else {
        return Err(PasskeyError::Refused);
    };
    let conn = store.lock()?;
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM passkeys", [], |r| r.get(0))?;
    if n >= MAX_PASSKEYS {
        return Err(PasskeyError::Refused);
    }
    let added = conn.execute(
        "INSERT OR IGNORE INTO passkeys (id, public_key, sign_count, created_ms, user_id) VALUES (?1, ?2, 0, ?3, ?4)",
        params![id, key, i64::try_from(now_ms).unwrap_or(i64::MAX), user],
    )?;
    // An id that already exists is not replaced: nobody can swap the key of a known passkey.
    if added == 1 {
        Ok(())
    } else {
        Err(PasskeyError::Refused)
    }
}

/// Ids of the passkeys the browser may offer.
pub fn ids(store: &SqliteStore) -> Result<Vec<String>, PasskeyError> {
    let conn = store.lock()?;
    let mut st = conn.prepare("SELECT id FROM passkeys ORDER BY created_ms")?;
    let rows = st.query_map([], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Verifies a sign-in assertion. `Ok` means the holder of that passkey is here (present and verified).
#[allow(clippy::too_many_arguments)]
pub fn login(
    store: &SqliteStore,
    challenges: &Challenges,
    host: &str,
    id: &str,
    auth_data: &[u8],
    client_data: &[u8],
    signature: &[u8],
    now_ms: u64,
) -> Result<Option<String>, PasskeyError> {
    let stored: Option<(Vec<u8>, i64, Option<String>)> = store
        .lock()?
        .query_row(
            "SELECT public_key, sign_count, user_id FROM passkeys WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    // The challenge is spent even when the passkey is unknown, so a miss costs the attacker the challenge too.
    let client_ok = check_client_data(
        client_data,
        "webauthn.get",
        Purpose::Login,
        challenges,
        host,
        now_ms,
    );
    let (Some((key, count, user)), true) = (stored, client_ok) else {
        return Err(PasskeyError::Refused);
    };
    // authenticatorData: rpIdHash (32) | flags (1) | signCount (4) | ...
    if auth_data.len() < 37
        || auth_data[..32] != Sha256::digest(rp_id(host).as_bytes())[..]
        || auth_data[32] & 0x01 == 0 // user present
        || auth_data[32] & 0x04 == 0
    // user verified
    {
        return Err(PasskeyError::Refused);
    }
    let new_count = i64::from(u32::from_be_bytes([
        auth_data[33],
        auth_data[34],
        auth_data[35],
        auth_data[36],
    ]));
    // Counters that count must go up; an authenticator that always says 0 is allowed (synced passkeys).
    if (new_count != 0 || count != 0) && new_count <= count {
        return Err(PasskeyError::Refused);
    }
    let mut signed = auth_data.to_vec();
    signed.extend_from_slice(&Sha256::digest(client_data));
    UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, &key)
        .verify(&signed, signature)
        .map_err(|_| PasskeyError::Refused)?;
    store.lock()?.execute(
        "UPDATE passkeys SET sign_count = ?2 WHERE id = ?1",
        params![id, new_count],
    )?;
    // Whose passkey it is: the session that follows is that person's (None: the owner's).
    Ok(user)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use ring::rand::SystemRandom;
    use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_ASN1_SIGNING};

    const HOST: &str = "hub.example:7443";

    struct Device {
        pair: EcdsaKeyPair,
    }

    impl Device {
        fn new() -> Self {
            let rng = SystemRandom::new();
            let pkcs8 =
                EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
            Self {
                pair: EcdsaKeyPair::from_pkcs8(
                    &ECDSA_P256_SHA256_ASN1_SIGNING,
                    pkcs8.as_ref(),
                    &rng,
                )
                .unwrap(),
            }
        }
        fn spki(&self) -> Vec<u8> {
            let mut v = SPKI_P256_PREFIX.to_vec();
            v.extend_from_slice(self.pair.public_key().as_ref());
            v
        }
        fn client_data(kind: &str, challenge: &str, origin: &str) -> Vec<u8> {
            serde_json::json!({"type": kind, "challenge": challenge, "origin": origin})
                .to_string()
                .into_bytes()
        }
        fn auth_data(host: &str, flags: u8, count: u32) -> Vec<u8> {
            let mut a = Sha256::digest(rp_id(host).as_bytes()).to_vec();
            a.push(flags);
            a.extend_from_slice(&count.to_be_bytes());
            a
        }
        fn sign(&self, auth: &[u8], client: &[u8]) -> Vec<u8> {
            let mut m = auth.to_vec();
            m.extend_from_slice(&Sha256::digest(client));
            self.pair
                .sign(&SystemRandom::new(), &m)
                .unwrap()
                .as_ref()
                .to_vec()
        }
    }

    fn enroll(store: &SqliteStore, ch: &Challenges, d: &Device, id: &str) {
        let c = ch.issue(Purpose::Register, 0).unwrap();
        let cd = Device::client_data("webauthn.create", &c, "https://hub.example:7443");
        register(store, ch, HOST, id, &d.spki(), &cd, 1, None).unwrap();
    }

    fn attempt(
        store: &SqliteStore,
        ch: &Challenges,
        d: &Device,
        id: &str,
        flags: u8,
        count: u32,
        origin: &str,
    ) -> Result<(), PasskeyError> {
        let c = ch.issue(Purpose::Login, 10).unwrap();
        let cd = Device::client_data("webauthn.get", &c, origin);
        let auth = Device::auth_data(HOST, flags, count);
        let sig = d.sign(&auth, &cd);
        login(store, ch, HOST, id, &auth, &cd, &sig, 20).map(|_| ())
    }

    #[test]
    fn base64url_round_trips_and_rejects_padding() {
        for n in 0..20u8 {
            let bytes: Vec<u8> = (0..n).map(|i| i.wrapping_mul(37)).collect();
            assert_eq!(b64url_decode(&b64url_encode(&bytes)).unwrap(), bytes);
        }
        assert_eq!(b64url_decode("AQID=").as_deref(), None);
        assert_eq!(b64url_decode("A"), None);
    }

    #[test]
    fn rp_id_is_the_host_without_port() {
        assert_eq!(rp_id("hub.example:7443"), "hub.example");
        assert_eq!(rp_id("localhost"), "localhost");
        assert_eq!(rp_id("[::1]:7443"), "::1");
    }

    #[test]
    fn a_registered_passkey_signs_in_and_the_counter_must_rise() {
        let store = SqliteStore::open_in_memory().unwrap();
        let ch = Challenges::new();
        let d = Device::new();
        enroll(&store, &ch, &d, "cred-1");
        assert_eq!(ids(&store).unwrap(), vec!["cred-1".to_string()]);
        let origin = "https://hub.example:7443";
        assert_eq!(attempt(&store, &ch, &d, "cred-1", 0x05, 1, origin), Ok(()));
        assert_eq!(
            attempt(&store, &ch, &d, "cred-1", 0x05, 1, origin),
            Err(PasskeyError::Refused),
            "a replayed or cloned counter"
        );
        assert_eq!(attempt(&store, &ch, &d, "cred-1", 0x05, 2, origin), Ok(()));
    }

    #[test]
    fn an_authenticator_that_never_counts_is_allowed() {
        let store = SqliteStore::open_in_memory().unwrap();
        let ch = Challenges::new();
        let d = Device::new();
        enroll(&store, &ch, &d, "c");
        let origin = "https://hub.example:7443";
        assert_eq!(attempt(&store, &ch, &d, "c", 0x05, 0, origin), Ok(()));
        assert_eq!(attempt(&store, &ch, &d, "c", 0x05, 0, origin), Ok(()));
    }

    #[test]
    fn wrong_proofs_are_refused() {
        let store = SqliteStore::open_in_memory().unwrap();
        let ch = Challenges::new();
        let d = Device::new();
        enroll(&store, &ch, &d, "c");
        let good = "https://hub.example:7443";
        // Another site's origin, a missing user-verified flag, another key, an unknown id.
        assert!(attempt(&store, &ch, &d, "c", 0x05, 1, "https://evil.example").is_err());
        assert!(attempt(&store, &ch, &d, "c", 0x01, 1, good).is_err());
        assert!(attempt(&store, &ch, &Device::new(), "c", 0x05, 1, good).is_err());
        assert!(attempt(&store, &ch, &d, "other", 0x05, 1, good).is_err());
        assert_eq!(attempt(&store, &ch, &d, "c", 0x05, 1, good), Ok(()));
    }

    #[test]
    fn a_challenge_works_once_only_and_for_its_purpose_and_expires() {
        let ch = Challenges::new();
        let c = ch.issue(Purpose::Login, 0).unwrap();
        assert!(!ch.take(Purpose::Register, &c, 1));
        assert!(ch.take(Purpose::Login, &c, 1));
        assert!(!ch.take(Purpose::Login, &c, 1));
        let late = ch.issue(Purpose::Login, 0).unwrap();
        assert!(!ch.take(Purpose::Login, &late, CHALLENGE_TTL_MS + 1));
    }

    #[test]
    fn an_existing_credential_id_is_not_overwritten_and_bad_keys_are_refused() {
        let store = SqliteStore::open_in_memory().unwrap();
        let ch = Challenges::new();
        enroll(&store, &ch, &Device::new(), "c");
        let other = Device::new();
        let c = ch.issue(Purpose::Register, 0).unwrap();
        let cd = Device::client_data("webauthn.create", &c, "https://hub.example:7443");
        assert!(register(&store, &ch, HOST, "c", &other.spki(), &cd, 1, None).is_err());
        let c = ch.issue(Purpose::Register, 0).unwrap();
        let cd = Device::client_data("webauthn.create", &c, "https://hub.example:7443");
        assert!(register(&store, &ch, HOST, "d", b"not a key", &cd, 1, None).is_err());
    }

    /// A passkey belongs to the person whose session added it: signing in with it gives that person's session, never the owner's.
    /// (An audit found that a member could add a passkey and sign in as the owner with it.)
    #[test]
    fn a_members_passkey_signs_in_as_that_member_and_never_as_the_owner() {
        use crate::signin::{HubSessions, SignIn};
        use crate::users::{Role, Users};
        use crate::web::SessionApi;
        use std::sync::Arc;
        let hub = Arc::new(crate::hub::Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap());
        let api = HubSessions::new(Arc::clone(&hub));
        let si = SignIn::new(hub.store(), Arc::new(crate::enroll::Lockout::new()));
        // From another machine, with a screen password set: a passkey assertion stands in for the password (ADR-0067).
        crate::password::ScreenPassword::with_iterations(hub.store(), 1_000)
            .set("a long screen password")
            .unwrap();
        let away = crate::password::Client::new(Some([198, 51, 100, 7].into()), "");
        let now = crate::server::now_ms();
        let member = Users::new(hub.store())
            .add("Eda", Role::Member, now)
            .unwrap();
        let tok = si
            .redeem(&si.create_code_for(Some(&member.id), now).unwrap(), now)
            .unwrap();
        let owner_tok = si.redeem(&si.create_code(now).unwrap(), now).unwrap();
        let b = |v: &[u8]| b64url_encode(v);
        let register_with = |tok: &str, id: &str, d: &Device| {
            // From another machine with a password set, a session alone cannot add a passkey.
            let (st, _) = api.passkey(
                "register-begin",
                Some(tok),
                HOST,
                &away,
                &serde_json::json!({}),
            );
            assert_eq!(st, 401);
            let pw = serde_json::json!({ "password": "a long screen password" });
            let (st, out) = api.passkey("register-begin", Some(tok), HOST, &away, &pw);
            assert_eq!(st, 200);
            let ch = serde_json::from_str::<serde_json::Value>(&out).unwrap()["challenge"]
                .as_str()
                .unwrap()
                .to_string();
            let cd = Device::client_data("webauthn.create", &ch, "https://hub.example:7443");
            let (st, _) = api.passkey(
                "register-finish",
                Some(tok),
                HOST,
                &away,
                &serde_json::json!({"id": id, "publicKey": b(&d.spki()), "clientDataJSON": b(&cd), "password": "a long screen password"}),
            );
            assert_eq!(st, 200);
        };
        let sign_in_with = |id: &str, d: &Device, count: u32| -> String {
            let (_, out) = api.passkey("login-begin", None, HOST, &away, &serde_json::json!({}));
            let ch = serde_json::from_str::<serde_json::Value>(&out).unwrap()["challenge"]
                .as_str()
                .unwrap()
                .to_string();
            let cd = Device::client_data("webauthn.get", &ch, "https://hub.example:7443");
            let auth = Device::auth_data(HOST, 0x05, count);
            let sig = d.sign(&auth, &cd);
            let (st, out) = api.passkey(
                "login-finish",
                None,
                HOST,
                &away,
                &serde_json::json!({"id": id, "authenticatorData": b(&auth), "clientDataJSON": b(&cd), "signature": b(&sig)}),
            );
            assert_eq!(st, 200);
            serde_json::from_str::<serde_json::Value>(&out).unwrap()["token"]
                .as_str()
                .unwrap()
                .to_string()
        };
        let (dm, dow) = (Device::new(), Device::new());
        register_with(&tok, "cred-member", &dm);
        register_with(&owner_tok, "cred-owner", &dow);

        let m = si
            .access(&sign_in_with("cred-member", &dm, 1), now)
            .unwrap();
        assert_eq!(
            (m.role, m.user.as_str()),
            (Role::Member, member.id.as_str()),
            "the member stays a member"
        );
        assert!(
            m.projects.is_some(),
            "and keeps only the projects they were given"
        );
        let o = si
            .access(&sign_in_with("cred-owner", &dow, 1), now)
            .unwrap();
        assert_eq!((o.role, o.user.as_str()), (Role::Owner, "owner"));

        // Removing the person removes their passkey with them.
        Users::new(hub.store()).remove(&member.id).unwrap();
        let (_, out) = api.passkey("login-begin", None, HOST, &away, &serde_json::json!({}));
        assert!(!out.contains("cred-member"), "{out}");
    }
}
