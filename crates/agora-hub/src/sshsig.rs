//! Ed25519 signatures as OpenSSH writes them (`ssh-keygen -Y sign`, the format the release files are signed in, `docs/security/`),
//! checked here with `ring` (already in the tree): no program is run and no crate is added. ADR-0114 decision 7 signs the community
//! plugin catalog's index this way, so its maintainer signs with the same tool and the same kind of key as a release.
//!
//! - **A public key** is one OpenSSH line, `ssh-ed25519 <base64> [comment]`, or an allowed-signers line `<principal> ssh-ed25519
//!   <base64>` (as `docs/security/release-key.pub`). Anything else is refused, a private key first of all, so a mistaken path never
//!   puts a private key into the hub's database.
//! - **A signature** is the armoured `SSHSIG` blob (PROTOCOL.sshsig): the signer's public key, a namespace, a hash algorithm
//!   (`sha256` or `sha512`) and the Ed25519 signature over `"SSHSIG" || namespace || reserved || hash algorithm || H(message)`. It is
//!   accepted only when its key is the pinned key, its namespace the one asked for, and the signature verifies under the pinned key.
//!
//! Everything is bounded: a key line of at most [`MAX_KEY_TEXT`] bytes, a signature of at most [`MAX_SIGNATURE_TEXT`].

use ring::signature::{UnparsedPublicKey, ED25519};
use sha2::{Digest, Sha256, Sha512};

/// The key type, and the only one accepted.
pub const KEY_TYPE: &str = "ssh-ed25519";
/// The largest public key file read.
pub const MAX_KEY_TEXT: usize = 4 * 1024;
/// The largest signature file read.
pub const MAX_SIGNATURE_TEXT: usize = 8 * 1024;
const MAGIC: &[u8] = b"SSHSIG";
const BEGIN: &str = "-----BEGIN SSH SIGNATURE-----";
const END: &str = "-----END SSH SIGNATURE-----";

/// Why a public key was not accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyError {
    /// The text is a private key: never kept.
    Private,
    /// Not one `ssh-ed25519` public key (another type, several keys, bad base64, a wrong length, too long).
    Invalid,
}

/// A pinned Ed25519 public key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicKey {
    /// The 32 key bytes.
    raw: [u8; 32],
}

impl PublicKey {
    /// Reads a public key file's text (see the module comment).
    pub fn parse(text: &str) -> Result<PublicKey, KeyError> {
        if text.contains("PRIVATE KEY") {
            return Err(KeyError::Private);
        }
        if text.len() > MAX_KEY_TEXT {
            return Err(KeyError::Invalid);
        }
        let lines: Vec<&str> = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect();
        let [line] = lines.as_slice() else {
            return Err(KeyError::Invalid);
        };
        let words: Vec<&str> = line.split_whitespace().collect();
        let at = words
            .iter()
            .position(|w| *w == KEY_TYPE)
            .filter(|i| *i <= 1)
            .ok_or(KeyError::Invalid)?;
        let blob = words
            .get(at + 1)
            .and_then(|b| base64_decode(b))
            .ok_or(KeyError::Invalid)?;
        Self::from_blob(&blob).ok_or(KeyError::Invalid)
    }

    /// From the SSH wire form: `string "ssh-ed25519" || string key(32)`, nothing after.
    fn from_blob(blob: &[u8]) -> Option<PublicKey> {
        let mut r = Reader(blob);
        if r.string()? != KEY_TYPE.as_bytes() {
            return None;
        }
        let raw: [u8; 32] = r.string()?.try_into().ok()?;
        r.0.is_empty().then_some(PublicKey { raw })
    }

    /// The SSH wire form of the key.
    pub fn blob(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(51);
        put_string(&mut out, KEY_TYPE.as_bytes());
        put_string(&mut out, &self.raw);
        out
    }

    /// The key as one OpenSSH line, without a comment: what the hub keeps.
    pub fn line(&self) -> String {
        format!("{KEY_TYPE} {}", base64_encode(&self.blob()))
    }

    /// The fingerprint `ssh-keygen -l` prints: `SHA256:` and the unpadded base64 of the SHA-256 of the wire form.
    pub fn fingerprint(&self) -> String {
        let b = base64_encode(&Sha256::digest(self.blob()));
        format!("SHA256:{}", b.trim_end_matches('='))
    }
}

/// Why a signature was not accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigError {
    /// Not an armoured `SSHSIG` signature of a kind this reads (too long, bad base64, another version, another key type or hash).
    Malformed,
    /// Made by another key than the pinned one.
    WrongKey,
    /// Made for another use (namespace) than the one asked for.
    Namespace,
    /// The signature does not verify: the signed text was changed, or the signature was.
    Invalid,
}

impl SigError {
    /// The stable code the command line words.
    pub fn code(self) -> &'static str {
        match self {
            SigError::Malformed => "malformed",
            SigError::WrongKey => "wrong-key",
            SigError::Namespace => "namespace",
            SigError::Invalid => "invalid",
        }
    }
}

/// Checks the armoured signature `armoured` of `message` under `key`, for `namespace`.
pub fn verify(
    key: &PublicKey,
    namespace: &str,
    message: &[u8],
    armoured: &str,
) -> Result<(), SigError> {
    if armoured.len() > MAX_SIGNATURE_TEXT {
        return Err(SigError::Malformed);
    }
    let body = armoured.trim();
    let inner = body
        .strip_prefix(BEGIN)
        .and_then(|r| r.strip_suffix(END))
        .ok_or(SigError::Malformed)?;
    let b64: String = inner.split_whitespace().collect();
    let blob = base64_decode(&b64).ok_or(SigError::Malformed)?;
    let mut r = Reader(&blob);
    if r.take(MAGIC.len()) != Some(MAGIC) || r.u32() != Some(1) {
        return Err(SigError::Malformed);
    }
    let signer = r.string().ok_or(SigError::Malformed)?;
    let ns = r.string().ok_or(SigError::Malformed)?;
    let reserved = r.string().ok_or(SigError::Malformed)?;
    let hash = r.string().ok_or(SigError::Malformed)?;
    let sig = r.string().ok_or(SigError::Malformed)?;
    if !r.0.is_empty() {
        return Err(SigError::Malformed);
    }
    let mut s = Reader(sig);
    if s.string() != Some(KEY_TYPE.as_bytes()) {
        return Err(SigError::Malformed);
    }
    let raw_sig = s.string().filter(|b| b.len() == 64);
    let Some(raw_sig) = raw_sig.filter(|_| s.0.is_empty()) else {
        return Err(SigError::Malformed);
    };
    let digest: Vec<u8> = match hash {
        b"sha512" => Sha512::digest(message).to_vec(),
        b"sha256" => Sha256::digest(message).to_vec(),
        _ => return Err(SigError::Malformed),
    };
    if PublicKey::from_blob(signer).as_ref() != Some(key) {
        return Err(SigError::WrongKey);
    }
    if ns != namespace.as_bytes() {
        return Err(SigError::Namespace);
    }
    let signed = signed_data(namespace, reserved, hash, &digest);
    UnparsedPublicKey::new(&ED25519, key.raw)
        .verify(&signed, raw_sig)
        .map_err(|_| SigError::Invalid)
}

/// What the Ed25519 signature covers (PROTOCOL.sshsig): public so a test can sign as `ssh-keygen` does, with a key of its own.
pub fn signed_data(namespace: &str, reserved: &[u8], hash: &[u8], digest: &[u8]) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    put_string(&mut out, namespace.as_bytes());
    put_string(&mut out, reserved);
    put_string(&mut out, hash);
    put_string(&mut out, digest);
    out
}

/// The armoured signature `ssh-keygen -Y sign` writes for `key` (its wire form), `namespace` and the raw Ed25519 signature `sig` over
/// [`signed_data`] with the `sha512` hash: public so a test can sign as the tool does.
pub fn armour(key_blob: &[u8], namespace: &str, sig: &[u8]) -> String {
    let mut blob = MAGIC.to_vec();
    blob.extend_from_slice(&1u32.to_be_bytes());
    put_string(&mut blob, key_blob);
    put_string(&mut blob, namespace.as_bytes());
    put_string(&mut blob, b"");
    put_string(&mut blob, b"sha512");
    let mut inner = Vec::new();
    put_string(&mut inner, KEY_TYPE.as_bytes());
    put_string(&mut inner, sig);
    put_string(&mut blob, &inner);
    let b64 = base64_encode(&blob);
    let mut out = format!("{BEGIN}\n");
    for chunk in b64.as_bytes().chunks(70) {
        out.push_str(&String::from_utf8_lossy(chunk));
        out.push('\n');
    }
    out.push_str(END);
    out.push('\n');
    out
}

/// The SHA-512 a signature of `message` covers, for [`signed_data`] in a test.
pub fn sha512(message: &[u8]) -> Vec<u8> {
    Sha512::digest(message).to_vec()
}

fn put_string(out: &mut Vec<u8>, b: &[u8]) {
    // Every string written here is a key, a namespace, a hash or a signature: far below 4 GiB.
    out.extend_from_slice(&u32::try_from(b.len()).unwrap_or(u32::MAX).to_be_bytes());
    out.extend_from_slice(b);
}

/// The SSH wire format, read from the front.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Some(head)
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_be_bytes(self.take(4)?.try_into().ok()?))
    }

    fn string(&mut self) -> Option<&'a [u8]> {
        let n = usize::try_from(self.u32()?).ok()?;
        self.take(n)
    }
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding.
pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Standard base64, strictly: padded to a multiple of four, no other character, no bits left over.
fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    if b.is_empty() || !b.len().is_multiple_of(4) {
        return None;
    }
    let pad = b.iter().rev().take_while(|c| **c == b'=').count();
    if pad > 2 {
        return None;
    }
    let mut out = Vec::with_capacity(b.len() / 4 * 3);
    for (ci, chunk) in b.chunks(4).enumerate() {
        let last = ci + 1 == b.len() / 4;
        let mut n = 0u32;
        let mut valid = 0;
        for (i, c) in chunk.iter().enumerate() {
            let v = match c {
                b'A'..=b'Z' => c - b'A',
                b'a'..=b'z' => c - b'a' + 26,
                b'0'..=b'9' => c - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                b'=' if last && i >= 4 - pad => {
                    continue;
                }
                _ => return None,
            };
            n |= u32::from(v) << (18 - 6 * i);
            valid += 1;
        }
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        let keep = valid - 1;
        // Bits past the last whole byte must be zero, so one text decodes to one byte string only.
        if (keep == 1 && n & 0xFFFF != 0) || (keep == 2 && n & 0xFF != 0) {
            return None;
        }
        out.extend_from_slice(&bytes[..keep]);
    }
    Some(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use ring::rand::SystemRandom;
    use ring::signature::{Ed25519KeyPair, KeyPair};

    fn pair() -> Ed25519KeyPair {
        let doc = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        Ed25519KeyPair::from_pkcs8(doc.as_ref()).unwrap()
    }

    fn public(p: &Ed25519KeyPair) -> PublicKey {
        PublicKey {
            raw: p.public_key().as_ref().try_into().unwrap(),
        }
    }

    fn sign(p: &Ed25519KeyPair, ns: &str, msg: &[u8]) -> String {
        let key = public(p);
        let sig = p.sign(&signed_data(ns, b"", b"sha512", &sha512(msg)));
        armour(&key.blob(), ns, sig.as_ref())
    }

    #[test]
    fn base64_round_trips_and_is_strict() {
        for n in 0..10 {
            let b: Vec<u8> = (0..n).map(|i: u32| (i * 37 % 256) as u8).collect();
            if b.is_empty() {
                continue;
            }
            assert_eq!(base64_decode(&base64_encode(&b)).unwrap(), b);
        }
        assert_eq!(base64_decode("QQ==").unwrap(), b"A");
        for bad in ["", "QQ", "QR==", "Q===", "QQ=A", "Q Q=", "QQ==QQ=="] {
            assert_eq!(base64_decode(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_public_key_line_is_read_and_a_private_key_never() {
        let p = pair();
        let k = public(&p);
        let line = k.line();
        assert_eq!(PublicKey::parse(&line).unwrap(), k);
        assert_eq!(
            PublicKey::parse(&format!("{line} maintainer@catalog\n")).unwrap(),
            k
        );
        assert_eq!(
            PublicKey::parse(&format!("agora-plugin-catalog {line}")).unwrap(),
            k
        );
        assert_eq!(
            PublicKey::parse(
                "-----BEGIN OPENSSH PRIVATE KEY-----\nb3Blbn\n-----END OPENSSH PRIVATE KEY-----"
            ),
            Err(KeyError::Private)
        );
        for bad in [
            String::new(),
            "ssh-rsa AAAAB3NzaC1yc2E".to_string(),
            format!("{line}\n{line}"),
            "ssh-ed25519 !!!".to_string(),
            format!("a b {line}"),
            format!("ssh-ed25519 {}", base64_encode(b"short")),
        ] {
            assert_eq!(PublicKey::parse(&bad), Err(KeyError::Invalid), "{bad}");
        }
        assert!(k.fingerprint().starts_with("SHA256:"));
        assert!(!k.fingerprint().ends_with('='));
    }

    #[test]
    fn a_signature_verifies_only_for_its_key_namespace_and_message() {
        let p = pair();
        let k = public(&p);
        let sig = sign(&p, "agora-plugin-catalog", b"index");
        assert_eq!(verify(&k, "agora-plugin-catalog", b"index", &sig), Ok(()));
        assert_eq!(
            verify(&k, "agora-plugin-catalog", b"index!", &sig),
            Err(SigError::Invalid)
        );
        assert_eq!(verify(&k, "file", b"index", &sig), Err(SigError::Namespace));
        let other = public(&pair());
        assert_eq!(
            verify(&other, "agora-plugin-catalog", b"index", &sig),
            Err(SigError::WrongKey)
        );
        // A signature by another key that claims the pinned key in its blob does not verify.
        let q = pair();
        let forged = armour(
            &k.blob(),
            "agora-plugin-catalog",
            q.sign(&signed_data(
                "agora-plugin-catalog",
                b"",
                b"sha512",
                &sha512(b"index"),
            ))
            .as_ref(),
        );
        assert_eq!(
            verify(&k, "agora-plugin-catalog", b"index", &forged),
            Err(SigError::Invalid)
        );
        for bad in [
            "",
            "-----BEGIN SSH SIGNATURE-----\n!!\n-----END SSH SIGNATURE-----",
        ] {
            assert_eq!(
                verify(&k, "agora-plugin-catalog", b"index", bad),
                Err(SigError::Malformed)
            );
        }
        let long = format!("{sig}{}", " ".repeat(MAX_SIGNATURE_TEXT));
        assert_eq!(
            verify(&k, "agora-plugin-catalog", b"index", &long),
            Err(SigError::Malformed)
        );
    }
}
