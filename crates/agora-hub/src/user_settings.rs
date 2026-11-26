//! Each person's own settings values, kept by the hub so the phone and the laptop agree (ADR-0113 decision 6, build step 4;
//! docs/ux/themes.md "Keeping themes the same on your devices").
//!
//! - **Opaque.** A value is bytes the hub never parses, checks or shows: every client checks what it reads before using it. The hub
//!   only measures it: at most [`MAX_VALUE_BYTES`] per person and key, refused whole over that (`settings.too_large`).
//! - **Per person.** Values are keyed by the session's person (`Access::user`, `owner` on a hub with one person), never by anything a
//!   request names, so nobody reads or writes another person's. Removing a person forgets their values (`users.rs`).
//! - **A closed set of keys.** Only [`KEYS`]; anything else is `protocol.invalid_message` (`key`), so the hub is no general store.
//! - **Versioned.** Each write raises the value's version by one. A write names the version it was based on; one based on any other
//!   version is refused with `settings.stale` naming the current one, and nothing changes. The check and the write are one transaction
//!   under the write lock, so two writes based on the same version never both land.

use agora_protocol::v1::{UserSettingsSet, UserSettingsValue};
use agora_protocol::{Error, ErrorCode};
use rusqlite::{params, OptionalExtension};

use crate::store::{write_tx, SqliteStore};

/// The largest value kept per person and key: 256 KiB (ADR-0113 decision 6).
pub const MAX_VALUE_BYTES: usize = 256 * 1024;
/// The keys a person may keep. `themes`: their themes, theme packs and theme choice, as the window writes them.
pub const KEYS: &[&str] = &["themes"];

fn to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn to_u64(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

fn internal() -> Error {
    Error::new(ErrorCode::Internal).with("incident_id", "settings")
}

/// The key a request names, when it is one the hub keeps.
pub fn known_key(key: &str) -> Option<&'static str> {
    KEYS.iter().copied().find(|k| *k == key)
}

fn check_key(key: &str) -> Result<&'static str, Error> {
    known_key(key).ok_or_else(|| Error::new(ErrorCode::ProtocolInvalidMessage).with("field", "key"))
}

pub struct UserSettings<'a> {
    store: &'a SqliteStore,
}

impl<'a> UserSettings<'a> {
    pub fn new(store: &'a SqliteStore) -> Self {
        Self { store }
    }

    /// `user`'s own value under `key`; version 0 and empty for one never written.
    pub fn get(&self, user: &str, key: &str) -> Result<UserSettingsValue, Error> {
        let key = check_key(key)?;
        let conn = self.store.lock().map_err(|_| internal())?;
        let row: Option<(i64, Vec<u8>, i64)> = conn
            .query_row(
                "SELECT version, value, updated_ms FROM user_settings WHERE user_id = ?1 AND key = ?2",
                params![user, key],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|_| internal())?;
        let (version, value, updated_ms) = row.unwrap_or((0, Vec::new(), 0));
        Ok(UserSettingsValue {
            key: key.to_string(),
            value,
            version: to_u64(version),
            updated_ms: to_u64(updated_ms),
        })
    }

    /// Replaces `user`'s own value under `set.key` when `set.base_version` is the current version; answers the new value. The size is
    /// checked before the database is touched.
    pub fn set(
        &self,
        user: &str,
        set: &UserSettingsSet,
        now_ms: u64,
    ) -> Result<UserSettingsValue, Error> {
        let key = check_key(&set.key)?;
        if set.value.len() > MAX_VALUE_BYTES {
            return Err(Error::new(ErrorCode::SettingsTooLarge).with("max", MAX_VALUE_BYTES as i64));
        }
        let mut conn = self.store.lock().map_err(|_| internal())?;
        let tx = write_tx(&mut conn).map_err(|_| internal())?;
        let current: i64 = tx
            .query_row(
                "SELECT version FROM user_settings WHERE user_id = ?1 AND key = ?2",
                params![user, key],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| internal())?
            .unwrap_or(0);
        if to_u64(current) != set.base_version {
            return Err(Error::new(ErrorCode::SettingsStale).with("version", current));
        }
        let version = current.saturating_add(1);
        tx.execute(
            "INSERT INTO user_settings (user_id, key, version, value, updated_ms) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(user_id, key) DO UPDATE SET version = excluded.version, value = excluded.value,
                 updated_ms = excluded.updated_ms",
            params![user, key, version, set.value, to_i64(now_ms)],
        )
        .map_err(|_| internal())?;
        tx.commit().map_err(|_| internal())?;
        Ok(UserSettingsValue {
            key: key.to_string(),
            value: set.value.clone(),
            version: to_u64(version),
            updated_ms: now_ms,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn store() -> SqliteStore {
        SqliteStore::open_in_memory().unwrap()
    }

    fn set(key: &str, value: &[u8], base: u64) -> UserSettingsSet {
        UserSettingsSet {
            key: key.into(),
            value: value.to_vec(),
            base_version: base,
        }
    }

    #[test]
    fn a_value_never_written_is_version_zero_and_empty() {
        let s = store();
        let v = UserSettings::new(&s).get("eda", "themes").unwrap();
        assert_eq!(
            v,
            UserSettingsValue {
                key: "themes".into(),
                value: vec![],
                version: 0,
                updated_ms: 0
            }
        );
    }

    #[test]
    fn each_write_raises_the_version_and_a_stale_one_changes_nothing() {
        let s = store();
        let u = UserSettings::new(&s);
        let first = u.set("eda", &set("themes", b"{\"a\":1}", 0), 10).unwrap();
        assert_eq!((first.version, first.updated_ms), (1, 10));
        let second = u.set("eda", &set("themes", b"{\"a\":2}", 1), 11).unwrap();
        assert_eq!(second.version, 2);
        // Two windows read version 1; the second to write is refused and names the current version.
        for base in [0, 1, 3] {
            let e = u.set("eda", &set("themes", b"lost", base), 12).unwrap_err();
            assert_eq!(e.code, ErrorCode::SettingsStale, "base {base}");
            assert_eq!(
                e,
                Error::new(ErrorCode::SettingsStale).with("version", 2_i64)
            );
        }
        let now = u.get("eda", "themes").unwrap();
        assert_eq!(
            (now.value.as_slice(), now.version, now.updated_ms),
            (&b"{\"a\":2}"[..], 2, 11)
        );
    }

    #[test]
    fn the_cap_is_256_kib_and_a_larger_value_is_refused_whole() {
        let s = store();
        let u = UserSettings::new(&s);
        let at_cap = vec![b'x'; MAX_VALUE_BYTES];
        assert_eq!(
            u.set("eda", &set("themes", &at_cap, 0), 1).unwrap().version,
            1
        );
        let over = vec![b'y'; MAX_VALUE_BYTES + 1];
        let e = u.set("eda", &set("themes", &over, 1), 2).unwrap_err();
        assert_eq!(
            e,
            Error::new(ErrorCode::SettingsTooLarge).with("max", 262_144_i64)
        );
        assert_eq!(u.get("eda", "themes").unwrap().value, at_cap);
    }

    #[test]
    fn each_person_has_their_own_value() {
        let s = store();
        let u = UserSettings::new(&s);
        u.set("eda", &set("themes", b"eda's", 0), 1).unwrap();
        assert_eq!(u.get("bo", "themes").unwrap().version, 0);
        assert!(u.get("bo", "themes").unwrap().value.is_empty());
        // Bo's own version counter starts from nothing: Eda's version is no base for Bo's write.
        assert_eq!(
            u.set("bo", &set("themes", b"bo's", 1), 2).unwrap_err().code,
            ErrorCode::SettingsStale
        );
        u.set("bo", &set("themes", b"bo's", 0), 2).unwrap();
        assert_eq!(u.get("eda", "themes").unwrap().value, b"eda's");
        assert_eq!(u.get("bo", "themes").unwrap().value, b"bo's");
    }

    #[test]
    fn only_known_keys_are_kept() {
        let s = store();
        let u = UserSettings::new(&s);
        let bad = Error::new(ErrorCode::ProtocolInvalidMessage).with("field", "key");
        for key in ["", "Themes", "themes ", "layout", "../themes"] {
            assert_eq!(u.get("eda", key).unwrap_err(), bad, "{key:?}");
            assert_eq!(
                u.set("eda", &set(key, b"x", 0), 1).unwrap_err(),
                bad,
                "{key:?}"
            );
        }
    }

    /// Removing a person forgets their settings, and nobody else's.
    #[test]
    fn removing_a_person_forgets_their_settings() {
        use crate::users::{Role, Users};
        let s = store();
        let users = Users::new(&s);
        let eda = users.add("Eda", Role::Viewer, 1).unwrap();
        let bo = users.add("Bo", Role::Member, 2).unwrap();
        let u = UserSettings::new(&s);
        u.set(&eda.id, &set("themes", b"e", 0), 3).unwrap();
        u.set(&bo.id, &set("themes", b"b", 0), 3).unwrap();
        assert!(users.remove(&eda.id).unwrap());
        assert_eq!(u.get(&eda.id, "themes").unwrap().version, 0);
        assert_eq!(u.get(&bo.id, "themes").unwrap().value, b"b");
    }
}
