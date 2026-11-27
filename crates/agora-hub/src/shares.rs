//! ADR-0122 step 2: the shared skills and MCP servers the hub keeps. Only the owner publishes or removes one, each with a word signed by
//! one of the owner's devices (`agora-share-item-v1`); the hub checks it as the nodes will (the device, the signature, the content's
//! hash, the item's rules, a newer `issued_ms` than the word it replaces) and keeps the newest word per name, a removal included, so
//! it can hand a node the word that says an item is gone. The hub cannot make up or widen an item: every node checks the signature
//! against its own trusted devices (step 3).

use agora_protocol::share::{
    check_item, check_scope, content_bytes, signed_message, Action, MAX_ITEMS,
};
use agora_protocol::v1::{
    ShareItemChanged, ShareItemInfo, ShareItemKind, ShareItemRefused, ShareItemSigned,
};
use agora_protocol::{Error, ErrorCode};
use prost::Message;
use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::store::SqliteStore;

/// How far ahead of the hub's clock a word's `issued_ms` may be: a device whose clock runs fast still publishes, but a word cannot be
/// dated so far ahead that no later word could ever replace it.
pub const MAX_AHEAD_MS: u64 = 5 * 60 * 1000;
/// The most refusals kept per item (one per machine).
const MAX_REFUSALS: usize = 256;

fn sha256_hex(b: &[u8]) -> String {
    Sha256::digest(b)
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect()
}

/// An item's content hash, as the signature names it.
pub fn item_hash(item: &agora_protocol::v1::ShareItem) -> String {
    sha256_hex(&content_bytes(item, sha256_hex))
}

fn refused(reason: &str) -> Error {
    Error::new(ErrorCode::ShareRefused).with("reason", reason)
}

fn invalid(field: &str) -> Error {
    Error::new(ErrorCode::ProtocolInvalidMessage).with("field", field)
}

fn internal<E>(_: E) -> Error {
    Error::new(ErrorCode::Internal)
}

pub struct Shares<'a> {
    store: &'a SqliteStore,
}

impl<'a> Shares<'a> {
    pub fn new(store: &'a SqliteStore) -> Self {
        Self { store }
    }

    /// Checks the owner's signed word (a publish when it carries an item, a removal when not) and keeps it as the newest for its name.
    /// `user` is the signed-in owner; the signing device must be one of theirs and not revoked. Returns the change to announce.
    pub fn apply(
        &self,
        user: &str,
        w: &ShareItemSigned,
        now_ms: u64,
    ) -> Result<ShareItemChanged, Error> {
        let scope = w.scope.clone().unwrap_or_default();
        check_scope(&scope).map_err(invalid)?;
        if w.issued_ms == 0 || w.issued_ms > now_ms.saturating_add(MAX_AHEAD_MS) {
            return Err(invalid("issued_ms"));
        }
        let (action, kind, hash) = match &w.item {
            Some(item) => {
                check_item(item).map_err(invalid)?;
                if item.name != w.name {
                    return Err(invalid("name"));
                }
                let hash = item_hash(item);
                if w.item_hash != hash {
                    return Err(invalid("item_hash"));
                }
                (Action::Publish, item.kind, hash)
            }
            None => {
                if !agora_protocol::share::valid_name(&w.name, 64) || !w.item_hash.is_empty() {
                    return Err(invalid("name"));
                }
                (
                    Action::Remove,
                    ShareItemKind::Unspecified as i32,
                    String::new(),
                )
            }
        };
        let message = signed_message(action, &w.name, &hash, &scope, w.issued_ms);
        let mut conn = self.store.lock().map_err(internal)?;
        let key: Option<Vec<u8>> = conn
            .query_row(
                "SELECT public_key FROM user_devices WHERE device_id = ?1 AND user_id = ?2 AND revoked_ms = 0",
                params![w.signer_device_id, user],
                |r| r.get(0),
            )
            .optional()
            .map_err(internal)?;
        let signed_by_owner =
            key.is_some_and(|k| crate::devices::verifies(&k, &message, &w.signature));
        if !signed_by_owner {
            return Err(Error::new(ErrorCode::ApprovalSignatureInvalid)
                .with("approval_id", w.name.as_str()));
        }
        let tx = crate::store::write_tx(&mut conn).map_err(internal)?;
        let newest: Option<u64> = tx
            .query_row(
                "SELECT issued_ms FROM shared_items WHERE name = ?1",
                [&w.name],
                |r| r.get::<_, i64>(0),
            )
            .optional()
            .map_err(internal)?
            .map(|v| u64::try_from(v).unwrap_or(0));
        if newest.is_some_and(|n| n >= w.issued_ms) {
            return Err(refused("older"));
        }
        if action == Action::Publish {
            let live: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM shared_items WHERE kind != 0 AND name != ?1",
                    [&w.name],
                    |r| r.get(0),
                )
                .map_err(internal)?;
            if usize::try_from(live).unwrap_or(usize::MAX) >= MAX_ITEMS {
                return Err(refused("limit"));
            }
        }
        if action == Action::Remove && newest.is_none() {
            return Err(Error::new(ErrorCode::ShareUnknown).with("name", w.name.as_str()));
        }
        tx.execute(
            "INSERT INTO shared_items (name, kind, item_hash, issued_ms, signer, signed) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(name) DO UPDATE SET kind = ?2, item_hash = ?3, issued_ms = ?4, signer = ?5, signed = ?6",
            params![
                w.name,
                kind,
                hash,
                i64::try_from(w.issued_ms).map_err(internal)?,
                w.signer_device_id,
                w.encode_to_vec()
            ],
        )
        .map_err(internal)?;
        // What machines said of the word it replaces no longer holds.
        tx.execute(
            "DELETE FROM shared_item_refusals WHERE name = ?1",
            [&w.name],
        )
        .map_err(internal)?;
        tx.commit().map_err(internal)?;
        Ok(ShareItemChanged {
            name: w.name.clone(),
            kind,
            item_hash: hash,
            scope: Some(scope),
            issued_ms: w.issued_ms,
        })
    }

    /// The live items (not the removals), by name, each with what machines refused of it; the signed word only `with_content`.
    pub fn list(&self, with_content: bool) -> Result<Vec<ShareItemInfo>, Error> {
        let conn = self.store.lock().map_err(internal)?;
        let mut stmt = conn
            .prepare("SELECT signed FROM shared_items WHERE kind != 0 ORDER BY name")
            .map_err(internal)?;
        let words: Vec<Vec<u8>> = stmt
            .query_map([], |r| r.get(0))
            .map_err(internal)?
            .collect::<Result<_, _>>()
            .map_err(internal)?;
        let mut out = Vec::with_capacity(words.len());
        for bytes in words {
            let w = ShareItemSigned::decode(bytes.as_slice()).map_err(internal)?;
            let mut refusals = conn
                .prepare_cached(
                    "SELECT node_id, item_hash, code, reason FROM shared_item_refusals WHERE name = ?1 ORDER BY node_id",
                )
                .map_err(internal)?;
            let refused: Vec<ShareItemRefused> = refusals
                .query_map([&w.name], |r| {
                    Ok(ShareItemRefused {
                        node_id: r.get(0)?,
                        name: w.name.clone(),
                        item_hash: r.get(1)?,
                        code: r.get(2)?,
                        reason: r.get(3)?,
                    })
                })
                .map_err(internal)?
                .collect::<Result<_, _>>()
                .map_err(internal)?;
            out.push(ShareItemInfo {
                name: w.name.clone(),
                kind: w.item.as_ref().map_or(0, |i| i.kind),
                item_hash: w.item_hash.clone(),
                scope: w.scope.clone(),
                issued_ms: w.issued_ms,
                signer_device_id: w.signer_device_id.clone(),
                signed: with_content.then(|| w.clone()),
                refused,
            });
        }
        Ok(out)
    }

    /// The newest word on `name` for a node: the item (when `item_hash` is empty or names it) or the removal that ended it.
    pub fn get(&self, name: &str, item_hash: &str) -> Result<ShareItemSigned, Error> {
        let conn = self.store.lock().map_err(internal)?;
        let row: Option<(String, Vec<u8>)> = conn
            .query_row(
                "SELECT item_hash, signed FROM shared_items WHERE name = ?1",
                [name],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(internal)?;
        match row {
            Some((hash, bytes)) if item_hash.is_empty() || item_hash == hash => {
                ShareItemSigned::decode(bytes.as_slice()).map_err(internal)
            }
            _ => Err(Error::new(ErrorCode::ShareUnknown).with("name", name)),
        }
    }

    /// A machine said it will not give an item to its agents. Kept against the item it names (one row per machine, the newest),
    /// only while that is still the item's current version.
    pub fn refusal(&self, node_id: &str, r: &ShareItemRefused, now_ms: u64) -> Result<(), Error> {
        let mut conn = self.store.lock().map_err(internal)?;
        let tx = crate::store::write_tx(&mut conn).map_err(internal)?;
        let current: Option<String> = tx
            .query_row(
                "SELECT item_hash FROM shared_items WHERE name = ?1 AND kind != 0",
                [&r.name],
                |row| row.get(0),
            )
            .optional()
            .map_err(internal)?;
        if current.as_deref() != Some(r.item_hash.as_str()) {
            return Ok(());
        }
        let kept: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM shared_item_refusals WHERE name = ?1 AND node_id != ?2",
                params![r.name, node_id],
                |row| row.get(0),
            )
            .map_err(internal)?;
        if usize::try_from(kept).unwrap_or(usize::MAX) >= MAX_REFUSALS {
            return Ok(());
        }
        let cut = |s: &str, n: usize| s.chars().take(n).collect::<String>();
        tx.execute(
            "INSERT INTO shared_item_refusals (name, node_id, item_hash, code, reason, at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(name, node_id) DO UPDATE SET item_hash = ?3, code = ?4, reason = ?5, at_ms = ?6",
            params![
                r.name,
                node_id,
                r.item_hash,
                cut(&r.code, 64),
                cut(&r.reason, 64),
                i64::try_from(now_ms).unwrap_or(i64::MAX)
            ],
        )
        .map_err(internal)?;
        tx.commit().map_err(internal)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use agora_protocol::v1::{McpServerDef, ShareItem, ShareScope};
    use ring::rand::SystemRandom;
    use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};

    struct Device {
        id: String,
        key: EcdsaKeyPair,
    }

    fn device(store: &SqliteStore, id: &str, user: &str) -> Device {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng)
            .unwrap();
        store
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO user_devices (device_id, user_id, name, public_key, created_ms, last_used_ms) VALUES (?1, ?2, 'd', ?3, 1, 1)",
                params![id, user, key.public_key().as_ref().to_vec()],
            )
            .unwrap();
        Device { id: id.into(), key }
    }

    fn item(name: &str, command: &str) -> ShareItem {
        ShareItem {
            name: name.into(),
            kind: ShareItemKind::McpServer as i32,
            mcp: Some(McpServerDef {
                command: command.into(),
                ..Default::default()
            }),
            skill: None,
        }
    }

    fn word(d: &Device, name: &str, item: Option<ShareItem>, issued_ms: u64) -> ShareItemSigned {
        let hash = item.as_ref().map(item_hash).unwrap_or_default();
        let scope = ShareScope::default();
        let action = if item.is_some() {
            Action::Publish
        } else {
            Action::Remove
        };
        let sig = d
            .key
            .sign(
                &SystemRandom::new(),
                &signed_message(action, name, &hash, &scope, issued_ms),
            )
            .unwrap();
        ShareItemSigned {
            item,
            scope: Some(scope),
            issued_ms,
            item_hash: hash,
            signer_device_id: d.id.clone(),
            signature: sig.as_ref().to_vec(),
            name: name.into(),
        }
    }

    const NOW: u64 = 1_764_245_000_000;

    #[test]
    fn the_owner_publishes_changes_and_removes_an_item_and_a_node_gets_the_newest_word() {
        let store = SqliteStore::open_in_memory().unwrap();
        let owner = device(&store, "web-1", "owner");
        let s = Shares::new(&store);
        let changed = s
            .apply(
                "owner",
                &word(&owner, "db", Some(item("db", "npx")), NOW),
                NOW,
            )
            .unwrap();
        assert_eq!((changed.name.as_str(), changed.item_hash.len()), ("db", 64));
        let listed = s.list(false).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].signed.is_none(), "no content unless asked");
        assert!(s.list(true).unwrap()[0].signed.is_some());
        // A node gets it by name and hash, or by name alone.
        assert_eq!(
            s.get("db", &changed.item_hash).unwrap().item_hash,
            changed.item_hash
        );
        assert!(s.get("db", "").is_ok());
        assert_eq!(
            s.get("db", "beef").unwrap_err().code,
            ErrorCode::ShareUnknown
        );
        // A newer word replaces it; an older or equal one is refused.
        let v2 = s
            .apply(
                "owner",
                &word(&owner, "db", Some(item("db", "uvx")), NOW + 1),
                NOW,
            )
            .unwrap();
        assert_ne!(v2.item_hash, changed.item_hash);
        let old = s
            .apply(
                "owner",
                &word(&owner, "db", Some(item("db", "npx")), NOW),
                NOW,
            )
            .unwrap_err();
        assert_eq!(
            (
                old.code,
                old.params.get("reason").cloned().map(|v| format!("{v:?}"))
            ),
            (
                ErrorCode::ShareRefused,
                Some(format!("{:?}", agora_protocol::ParamValue::from("older")))
            )
        );
        // A removal ends it, and is what a node is handed for that name.
        let gone = s
            .apply("owner", &word(&owner, "db", None, NOW + 2), NOW)
            .unwrap();
        assert!(gone.item_hash.is_empty());
        assert!(s.list(false).unwrap().is_empty());
        let w = s.get("db", "").unwrap();
        assert!(w.item.is_none() && w.name == "db");
        // An old publish cannot bring it back.
        assert_eq!(
            s.apply(
                "owner",
                &word(&owner, "db", Some(item("db", "uvx")), NOW + 1),
                NOW
            )
            .unwrap_err()
            .code,
            ErrorCode::ShareRefused
        );
    }

    #[test]
    fn only_a_word_the_owner_signed_for_this_content_is_kept() {
        let store = SqliteStore::open_in_memory().unwrap();
        let owner = device(&store, "web-1", "owner");
        let member = device(&store, "web-2", "ayse");
        let s = Shares::new(&store);
        let sig_invalid = |r: Result<ShareItemChanged, Error>| {
            r.unwrap_err().code == ErrorCode::ApprovalSignatureInvalid
        };
        // Another person's device, or the owner's word sent as someone else.
        assert!(sig_invalid(s.apply(
            "owner",
            &word(&member, "db", Some(item("db", "npx")), NOW),
            NOW
        )));
        assert!(sig_invalid(s.apply(
            "ayse",
            &word(&owner, "db", Some(item("db", "npx")), NOW),
            NOW
        )));
        // Content changed after signing (the hash no longer matches), or a scope widened after signing.
        let mut w = word(&owner, "db", Some(item("db", "npx")), NOW);
        w.item = Some(item("db", "sh"));
        assert_eq!(
            s.apply("owner", &w, NOW)
                .unwrap_err()
                .params
                .get("field")
                .cloned()
                .map(|v| format!("{v:?}")),
            Some(format!(
                "{:?}",
                agora_protocol::ParamValue::from("item_hash")
            ))
        );
        let mut w = word(&owner, "db", Some(item("db", "npx")), NOW);
        w.scope = Some(ShareScope {
            project_id: String::new(),
            kinds: vec!["codex".into()],
        });
        assert!(sig_invalid(s.apply("owner", &w, NOW)));
        // A revoked device no longer signs.
        store
            .lock()
            .unwrap()
            .execute(
                "UPDATE user_devices SET revoked_ms = 5 WHERE device_id = 'web-1'",
                [],
            )
            .unwrap();
        assert!(sig_invalid(s.apply(
            "owner",
            &word(&owner, "db", Some(item("db", "npx")), NOW),
            NOW
        )));
    }

    #[test]
    fn a_word_dated_too_far_ahead_or_an_item_that_breaks_a_rule_is_refused_and_the_hub_keeps_at_most_64(
    ) {
        let store = SqliteStore::open_in_memory().unwrap();
        let owner = device(&store, "web-1", "owner");
        let s = Shares::new(&store);
        let field = |r: Result<ShareItemChanged, Error>| {
            r.unwrap_err()
                .params
                .get("field")
                .cloned()
                .map(|v| format!("{v:?}"))
        };
        assert_eq!(
            field(s.apply(
                "owner",
                &word(
                    &owner,
                    "db",
                    Some(item("db", "npx")),
                    NOW + MAX_AHEAD_MS + 1
                ),
                NOW
            )),
            Some(format!(
                "{:?}",
                agora_protocol::ParamValue::from("issued_ms")
            ))
        );
        assert_eq!(
            field(s.apply("owner", &word(&owner, "db", Some(item("db", "")), NOW), NOW)),
            Some(format!("{:?}", agora_protocol::ParamValue::from("command")))
        );
        assert_eq!(
            s.apply("owner", &word(&owner, "nothing", None, NOW), NOW)
                .unwrap_err()
                .code,
            ErrorCode::ShareUnknown
        );
        for i in 0..MAX_ITEMS {
            let n = format!("i{i}");
            s.apply("owner", &word(&owner, &n, Some(item(&n, "npx")), NOW), NOW)
                .unwrap();
        }
        let r = s
            .apply(
                "owner",
                &word(&owner, "one-more", Some(item("one-more", "npx")), NOW),
                NOW,
            )
            .unwrap_err();
        assert_eq!(
            r.params.get("reason").cloned().map(|v| format!("{v:?}")),
            Some(format!("{:?}", agora_protocol::ParamValue::from("limit")))
        );
        // A new version of a kept name is not one more.
        s.apply(
            "owner",
            &word(&owner, "i0", Some(item("i0", "uvx")), NOW + 1),
            NOW,
        )
        .unwrap();
    }

    #[test]
    fn a_machines_refusal_is_kept_for_the_current_version_only() {
        let store = SqliteStore::open_in_memory().unwrap();
        let owner = device(&store, "web-1", "owner");
        let s = Shares::new(&store);
        let v1 = s
            .apply(
                "owner",
                &word(&owner, "db", Some(item("db", "npx")), NOW),
                NOW,
            )
            .unwrap();
        let refusal = |hash: &str| ShareItemRefused {
            node_id: "mac".into(),
            name: "db".into(),
            item_hash: hash.into(),
            code: "share.refused".into(),
            reason: "policy_off".into(),
        };
        s.refusal("mac", &refusal("stale"), NOW).unwrap();
        assert!(
            s.list(false).unwrap()[0].refused.is_empty(),
            "a refusal of another version is not kept"
        );
        s.refusal("mac", &refusal(&v1.item_hash), NOW).unwrap();
        assert_eq!(s.list(false).unwrap()[0].refused[0].reason, "policy_off");
        // A new version clears what machines said of the old one.
        s.apply(
            "owner",
            &word(&owner, "db", Some(item("db", "uvx")), NOW + 1),
            NOW,
        )
        .unwrap();
        assert!(s.list(false).unwrap()[0].refused.is_empty());
    }
}
