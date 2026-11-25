//! The node's private ADR-0109 inbox records. Each file is one atomic acceptance, including
//! both the inbox payload and the information needed to answer a retry.

use std::collections::BTreeMap;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use agora_protocol::v1::DeliverTaskHandoff;
use prost::Message;

pub(crate) const MAX_RECORDS: usize = 1024;
pub(crate) const MAX_BYTES: usize = 32 * 1024 * 1024;
/// The longest a delivery may stay valid after the node accepts it: the hub's 24 hours (ADR-0109
/// amendment A) and one hour for the two clocks to differ. A later deadline is refused, so a hub
/// cannot keep the store's records past an ended run for longer than this.
pub(crate) const MAX_VALIDITY_MS: u64 = 25 * 60 * 60 * 1000;
const FORMAT: &[u8] = b"AHO1";
const HEADER_LEN: usize = 20;
pub(crate) type Id = [u8; 16];

#[derive(Debug)]
pub(crate) struct Record {
    pub number: u64,
    pub accepted_at_ms: u64,
    pub delivery: DeliverTaskHandoff,
}

impl Record {
    fn bytes(&self) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::with_capacity(HEADER_LEN + self.delivery.encoded_len());
        bytes.extend(FORMAT);
        bytes.extend(self.number.to_le_bytes());
        bytes.extend(self.accepted_at_ms.to_le_bytes());
        self.delivery.encode(&mut bytes).map_err(io::Error::other)?;
        Ok(bytes)
    }

    fn decode(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() < HEADER_LEN || !bytes.starts_with(FORMAT) {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let invalid = |_| io::Error::from(io::ErrorKind::InvalidData);
        let number = u64::from_le_bytes(bytes[4..12].try_into().map_err(invalid)?);
        let accepted_at_ms = u64::from_le_bytes(bytes[12..20].try_into().map_err(invalid)?);
        let delivery = DeliverTaskHandoff::decode(&bytes[HEADER_LEN..])
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Ok(Self {
            number,
            accepted_at_ms,
            delivery,
        })
    }

    /// The delivery's deadline, never later than [`MAX_VALIDITY_MS`] after its acceptance.
    fn deadline(&self) -> u64 {
        self.delivery
            .valid_until_ms
            .min(self.accepted_at_ms.saturating_add(MAX_VALIDITY_MS))
    }

    fn id(&self) -> io::Result<Id> {
        let invalid = io::ErrorKind::InvalidData;
        // A number at or past the node's id clamp would push the broker's counter past it on restore (#1194 review).
        if self.number == 0
            || self.number >= crate::mesh_server::MAX_FIRST_ID
            || self.delivery.receiver_run_id.len() != 16
        {
            return Err(invalid.into());
        }
        self.delivery
            .delivery_id
            .as_slice()
            .try_into()
            .map_err(|_| invalid.into())
    }
}

pub(crate) enum InsertError {
    Full {
        resource: &'static str,
        limit: usize,
    },
    Storage(io::Error),
}

pub(crate) struct Store {
    root: PathBuf,
    records: BTreeMap<Id, (Record, usize)>,
    bytes: usize,
}

fn filename(id: Id) -> String {
    use std::fmt::Write;
    let mut name = String::with_capacity(40);
    for b in id {
        // Infallible for String.
        let _ = write!(name, "{b:02x}");
    }
    name.push_str(".handoff");
    name
}

fn stale_temporary(name: &str) -> bool {
    let Some((id, pid)) = name.split_once(".handoff.tmp-") else {
        return false;
    };
    id.len() == 32
        && id.bytes().all(|b| b.is_ascii_hexdigit())
        && !pid.is_empty()
        && pid.bytes().all(|b| b.is_ascii_digit())
}

/// The inbox number in a record's header: `Ok(None)` when the file does not start like one (too
/// short or another format), `Err` when it could not be read.
fn header_number(path: &Path) -> io::Result<Option<u64>> {
    let mut header = [0; HEADER_LEN];
    match std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut header)) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    if !header.starts_with(FORMAT) {
        return Ok(None);
    }
    Ok(header[4..12].try_into().ok().map(u64::from_le_bytes))
}

/// A record file that could not be read (too many open files, no permission, a disk error): it is
/// left where it is and not loaded, since the next start may read it. Its size, when known, still
/// counts toward the byte limit while this store is open.
fn skipped(store: &mut Store, path: &Path, error: &io::Error) {
    eprintln!(
        "mesh: durable handoff record {} not loaded, left in place: {error}",
        path.display()
    );
    if let Ok(meta) = std::fs::metadata(path) {
        let len = usize::try_from(meta.len()).unwrap_or(usize::MAX);
        store.bytes = store.bytes.saturating_add(len).min(MAX_BYTES);
    }
}

/// Moves a record that is not loaded out of the store's way, keeping it for a person to look at.
/// The new name ends in `.rejected`: it neither loads as a record nor counts as a temporary.
fn set_aside(path: &Path, reason: &str) {
    let mut aside = path.as_os_str().to_owned();
    aside.push(".rejected");
    match std::fs::rename(path, &aside) {
        Ok(()) => eprintln!(
            "mesh: durable handoff record {} set aside ({reason})",
            path.display()
        ),
        Err(e) => eprintln!(
            "mesh: durable handoff record {} not loaded ({reason}); could not set it aside: {e}",
            path.display()
        ),
    }
}

impl Store {
    /// Loads what is on disk within the limits. A record that is not one this node wrote (its
    /// format, name or number) or does not fit the limits is set aside (renamed, see
    /// [`set_aside`]); one that cannot be read now is left in place ([`skipped`]); the rest stay
    /// available: one bad file never turns the durable inbox off. When more is on disk than the
    /// limits allow, the earliest accepted records (lowest inbox numbers) are kept.
    pub fn open(root: &Path) -> io::Result<Self> {
        agora_platform::ensure_private_dir(root)?;
        let mut store = Self {
            root: root.to_path_buf(),
            records: BTreeMap::new(),
            bytes: 0,
        };
        let mut found = Vec::new();
        for entry in std::fs::read_dir(root)? {
            let Ok(entry) = entry else {
                continue;
            };
            let path = entry.path();
            if entry.file_name().to_str().is_some_and(stale_temporary) {
                // Left by an interrupted write; never an accepted record.
                let _ = std::fs::remove_file(&path);
                continue;
            }
            if path.extension().and_then(|value| value.to_str()) != Some("handoff") {
                continue;
            }
            let is_file = match entry.file_type() {
                Ok(t) => t.is_file(),
                Err(e) => {
                    skipped(&mut store, &path, &e);
                    continue;
                }
            };
            if !is_file {
                set_aside(&path, "not a file");
                continue;
            }
            match header_number(&path) {
                Ok(Some(number)) => found.push((number, path)),
                Ok(None) => set_aside(&path, "format"),
                Err(e) => skipped(&mut store, &path, &e),
            }
        }
        found.sort_unstable();
        for (_, path) in found {
            if store.records.len() == MAX_RECORDS {
                set_aside(&path, "handoff_records");
                continue;
            }
            let remaining = MAX_BYTES - store.bytes;
            let mut bytes = Vec::new();
            let read = std::fs::File::open(&path)
                .and_then(|f| f.take(remaining as u64 + 1).read_to_end(&mut bytes));
            if let Err(e) = read {
                skipped(&mut store, &path, &e);
                continue;
            }
            if bytes.len() > remaining {
                set_aside(&path, "handoff_bytes");
                continue;
            }
            let loaded = Record::decode(&bytes).and_then(|record| Ok((record.id()?, record)));
            let Ok((id, record)) = loaded else {
                set_aside(&path, "format");
                continue;
            };
            if path.file_name().and_then(|n| n.to_str()) != Some(filename(id).as_str())
                || store.records.contains_key(&id)
            {
                set_aside(&path, "name");
                continue;
            }
            store.bytes += bytes.len();
            store.records.insert(id, (record, bytes.len()));
        }
        Ok(store)
    }

    pub fn get(&self, id: &Id) -> Option<&Record> {
        self.records.get(id).map(|(record, _)| record)
    }

    pub fn records(&self) -> impl Iterator<Item = &Record> {
        self.records.values().map(|(record, _)| record)
    }

    /// Whether one more record holding `delivery` fits the limits. A record's size does not
    /// depend on its number, so this is answered before a number is taken.
    pub fn room_for(&self, delivery: &DeliverTaskHandoff) -> Result<(), InsertError> {
        if self.records.len() >= MAX_RECORDS {
            return Err(InsertError::Full {
                resource: "handoff_records",
                limit: MAX_RECORDS,
            });
        }
        if HEADER_LEN + delivery.encoded_len() > MAX_BYTES - self.bytes {
            return Err(InsertError::Full {
                resource: "handoff_bytes",
                limit: MAX_BYTES,
            });
        }
        Ok(())
    }

    pub fn insert(&mut self, record: Record) -> Result<(), InsertError> {
        let id = record.id().map_err(InsertError::Storage)?;
        if self.records.contains_key(&id) {
            return Err(InsertError::Storage(io::ErrorKind::AlreadyExists.into()));
        }
        // A file under this id that `open` did not load (it could not be read then) stays for the next start: the retry is
        // refused for now (F1640), never written over it as a second entry. Only a proven absence lets the write go on.
        let path = self.root.join(filename(id));
        match std::fs::symlink_metadata(&path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Ok(_) => return Err(InsertError::Storage(io::ErrorKind::AlreadyExists.into())),
            Err(e) => return Err(InsertError::Storage(e)),
        }
        self.room_for(&record.delivery)?;
        let bytes = record.bytes().map_err(InsertError::Storage)?;
        agora_platform::write_private(&path, &bytes).map_err(InsertError::Storage)?;
        self.records.insert(id, (record, bytes.len()));
        self.bytes += bytes.len();
        Ok(())
    }

    /// A receipt remains until its deadline. Payloads for live runs remain available to the
    /// non-destructive MCP inbox, even after the deadline.
    pub fn prune(&mut self, now: u64, live: impl Fn(&DeliverTaskHandoff) -> bool) {
        self.records.retain(|id, (record, bytes)| {
            if record.deadline() > now || live(&record.delivery) {
                return true;
            }
            if std::fs::remove_file(self.root.join(filename(*id)))
                .is_err_and(|e| e.kind() != io::ErrorKind::NotFound)
            {
                return true;
            }
            self.bytes -= *bytes;
            false
        });
    }
}

#[cfg(test)]
impl Store {
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    pub(crate) fn len(&self) -> usize {
        self.records.len()
    }
}

/// Writes `record` where [`Store::open`] looks for it, without the store (and without a sync: a
/// test plants many). Returns the file's size.
#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(crate) fn plant(root: &Path, record: &Record) -> usize {
    std::fs::create_dir_all(root).unwrap();
    let bytes = record.bytes().unwrap();
    let id: Id = record.delivery.delivery_id.as_slice().try_into().unwrap();
    std::fs::write(root.join(filename(id)), &bytes).unwrap();
    bytes.len()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("agora-handoff-store-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn id_of(n: u64) -> Id {
        let mut id = [0xab; 16];
        id[..8].copy_from_slice(&n.to_le_bytes());
        id
    }

    fn record(number: u64, summary: String) -> Record {
        Record {
            number,
            accepted_at_ms: 10,
            delivery: DeliverTaskHandoff {
                delivery_id: id_of(number).to_vec(),
                receiver_run_id: vec![2; 16],
                agent_id: "receiver".into(),
                project_id: "shared".into(),
                task_id: "t-1".into(),
                summary,
                valid_until_ms: 1000,
                sender_node: "remote".into(),
                sender_agent_id: "sender".into(),
                ..Default::default()
            },
        }
    }

    fn names(root: &Path, extension: &str) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(root)
            .unwrap()
            .filter_map(|e| {
                let name = e.unwrap().file_name().into_string().unwrap();
                name.ends_with(extension).then_some(name)
            })
            .collect();
        names.sort();
        names
    }

    fn size_on_disk(root: &Path) -> usize {
        names(root, ".handoff")
            .iter()
            .map(|n| std::fs::metadata(root.join(n)).unwrap().len() as usize)
            .sum()
    }

    #[test]
    fn the_limits_are_the_accepted_ones() {
        // ADR-0109 amendment A.
        assert_eq!((MAX_RECORDS, MAX_BYTES), (1024, 32 * 1024 * 1024));
        assert_eq!(MAX_VALIDITY_MS, (24 + 1) * 3_600_000);
    }

    /// More records on disk than the limit (a node whose store was filled by hand, or by an older limit): the earliest accepted
    /// are loaded and the rest set aside, so the store is within its limits after `open` as after `insert`.
    #[test]
    fn the_record_limit_applies_on_reopen() {
        let dir = TempDir::new("reopen-records");
        let root = dir.0.join("handoffs");
        for n in 1..=MAX_RECORDS as u64 + 1 {
            plant(&root, &record(n, String::new()));
        }
        let store = Store::open(&root).unwrap();
        assert_eq!(store.len(), MAX_RECORDS);
        assert!(store.get(&id_of(MAX_RECORDS as u64 + 1)).is_none());
        assert!(store.get(&id_of(1)).is_some());
        assert_eq!(
            names(&root, ".rejected"),
            [format!(
                "{}.rejected",
                filename(id_of(MAX_RECORDS as u64 + 1))
            )]
        );
        assert_eq!(store.bytes(), size_on_disk(&root));
        drop(store);
        // Opening again finds the same records and nothing more to set aside.
        let store = Store::open(&root).unwrap();
        assert_eq!(store.len(), MAX_RECORDS);
        assert_eq!(names(&root, ".rejected").len(), 1);
    }

    #[test]
    fn the_byte_limit_applies_on_reopen() {
        let dir = TempDir::new("reopen-bytes");
        let root = dir.0.join("handoffs");
        let big = || "x".repeat(MAX_BYTES / 2);
        let first = plant(&root, &record(1, big()));
        plant(&root, &record(2, big()));
        plant(&root, &record(3, String::new()));
        let store = Store::open(&root).unwrap();
        // The first fits; the second would pass 32 MiB; the third, small, still fits after it.
        assert!(store.get(&id_of(1)).is_some());
        assert!(store.get(&id_of(2)).is_none());
        assert!(store.get(&id_of(3)).is_some());
        assert!(store.bytes() > first && store.bytes() <= MAX_BYTES);
        assert_eq!(store.bytes(), size_on_disk(&root));
        assert_eq!(
            names(&root, ".rejected"),
            [format!("{}.rejected", filename(id_of(2)))]
        );
    }

    /// Review of #1194: a record numbered at or past the node's id clamp would push the broker's counter past it when restored (an
    /// overflow, or no id left for any mesh call), so it is set aside like any record this node could not have written.
    #[test]
    fn a_record_numbered_past_the_id_clamp_is_set_aside() {
        let dir = TempDir::new("clamp");
        let root = dir.0.join("handoffs");
        let max = crate::mesh_server::MAX_FIRST_ID;
        plant(&root, &record(1, String::new()));
        for n in [max - 1, max, u64::MAX - 1] {
            let mut r = record(n, String::new());
            r.delivery.delivery_id = id_of(n % 1000 + 2).to_vec();
            plant(&root, &r);
        }
        let store = Store::open(&root).unwrap();
        assert_eq!(store.len(), 2);
        assert_eq!(store.records().map(|r| r.number).max(), Some(max - 1));
        assert_eq!(names(&root, ".rejected").len(), 2);
    }

    /// Review of #1194: a record that cannot be read now (here, no permission) is neither loaded nor set aside: it stays where it is
    /// for the next start, and its size still counts toward the byte limit.
    #[cfg(unix)]
    #[test]
    fn a_record_that_cannot_be_read_now_stays_in_place() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("eacces");
        let root = dir.0.join("handoffs");
        plant(&root, &record(1, "readable".into()));
        let size = plant(&root, &record(2, "locked".into()));
        let locked = root.join(filename(id_of(2)));
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::File::open(&locked).is_ok() {
            return; // running with privileges that read it anyway
        }
        let store = Store::open(&root).unwrap();
        assert_eq!(store.len(), 1);
        assert!(store.get(&id_of(2)).is_none());
        assert_eq!(store.bytes(), size_on_disk(&root));
        assert!(store.bytes() >= size);
        assert!(names(&root, ".rejected").is_empty());
        drop(store);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o600)).unwrap();
        let store = Store::open(&root).unwrap();
        assert_eq!(store.get(&id_of(2)).unwrap().delivery.summary, "locked");
    }

    /// F1640: a retry of a delivery whose record could not be read at start (left in place for the next start) is refused for now,
    /// never written over that record under a new number: one entry per delivery id, the original one.
    #[cfg(unix)]
    #[test]
    fn a_retry_never_replaces_a_record_left_in_place() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("retry-in-place");
        let root = dir.0.join("handoffs");
        plant(&root, &record(2, "locked".into()));
        let locked = root.join(filename(id_of(2)));
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::File::open(&locked).is_ok() {
            return; // running with privileges that read it anyway
        }
        let mut store = Store::open(&root).unwrap();
        assert!(store.insert(record(3, String::new())).is_ok());
        let mut retry = record(9, "locked".into());
        retry.delivery.delivery_id = id_of(2).to_vec();
        assert!(matches!(
            store.insert(retry),
            Err(InsertError::Storage(ref e)) if e.kind() == io::ErrorKind::AlreadyExists
        ));
        assert!(store.get(&id_of(2)).is_none());
        drop(store);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o600)).unwrap();
        let store = Store::open(&root).unwrap();
        assert_eq!(store.get(&id_of(2)).unwrap().number, 2);
        assert_eq!(store.len(), 2);
    }

    /// Review of #1175 (N5): one file that is not a record this node wrote does not stop the store from opening (which would turn
    /// the durable inbox off). It is set aside, and every good record is still loaded.
    #[test]
    fn a_bad_record_is_set_aside_and_the_rest_load() {
        let dir = TempDir::new("bad");
        let root = dir.0.join("handoffs");
        let good = plant(&root, &record(1, "kept".into()));
        // Garbage, a short file, a record under another record's name, an empty record id, and a folder.
        std::fs::write(root.join(filename(id_of(7))), b"AHO1 not a record").unwrap();
        std::fs::write(root.join(filename(id_of(8))), b"AH").unwrap();
        let moved = record(9, String::new());
        std::fs::write(root.join(filename(id_of(10))), moved.bytes().unwrap()).unwrap();
        let mut no_id = record(11, String::new()).bytes().unwrap();
        no_id[4..12].copy_from_slice(&0u64.to_le_bytes());
        std::fs::write(root.join(filename(id_of(11))), no_id).unwrap();
        std::fs::create_dir(root.join(filename(id_of(12)))).unwrap();
        // An interrupted write is removed; other files are left alone.
        std::fs::write(
            root.join(format!("{}.tmp-123", filename(id_of(13)))),
            b"partial",
        )
        .unwrap();
        std::fs::write(root.join("notes.txt"), b"left").unwrap();

        let store = Store::open(&root).unwrap();
        assert_eq!(store.len(), 1);
        assert_eq!(store.get(&id_of(1)).unwrap().delivery.summary, "kept");
        assert_eq!(store.bytes(), good);
        assert_eq!(names(&root, ".handoff"), [filename(id_of(1))]);
        assert_eq!(names(&root, ".rejected").len(), 5);
        assert!(names(&root, "-123").is_empty());
        assert!(root.join("notes.txt").exists());
    }

    /// Review of #1175 (B2, N3): an ended run's record keeps its receipt until its deadline and is reclaimed at it; a live run keeps
    /// its record past the deadline; and no record outlives `MAX_VALIDITY_MS` after its acceptance, whatever deadline it names.
    #[test]
    fn prune_keeps_receipts_until_the_deadline_and_never_past_the_cap() {
        let dir = TempDir::new("prune");
        let root = dir.0.join("handoffs");
        let mut store = Store::open(&root).unwrap();
        let mut far = record(3, String::new());
        far.delivery.valid_until_ms = u64::MAX;
        for r in [record(1, String::new()), record(2, String::new()), far] {
            store.insert(r).ok().unwrap();
        }
        let live = |d: &DeliverTaskHandoff| d.delivery_id == id_of(2);
        store.prune(999, live);
        assert_eq!(store.len(), 3);
        store.prune(1000, live);
        assert!(store.get(&id_of(1)).is_none());
        assert!(store.get(&id_of(2)).is_some(), "a live run keeps its entry");
        assert!(store.get(&id_of(3)).is_some());
        assert!(!root.join(filename(id_of(1))).exists());
        assert_eq!(store.bytes(), size_on_disk(&root));
        let cap = 10 + MAX_VALIDITY_MS;
        store.prune(cap - 1, live);
        assert!(store.get(&id_of(3)).is_some());
        store.prune(cap, live);
        assert!(store.get(&id_of(3)).is_none());
        assert_eq!(names(&root, ".handoff"), [filename(id_of(2))]);
        assert_eq!(store.bytes(), size_on_disk(&root));
    }
}
