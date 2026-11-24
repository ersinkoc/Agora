//! The node's private ADR-0109 inbox records. Each file is one atomic acceptance, including
//! both the inbox payload and the information needed to answer a retry.

use std::collections::BTreeMap;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use agora_protocol::v1::DeliverTaskHandoff;
use prost::Message;

pub(crate) const MAX_RECORDS: usize = 1024;
pub(crate) const MAX_BYTES: usize = 32 * 1024 * 1024;
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

    fn id(&self) -> io::Result<Id> {
        let invalid = io::ErrorKind::InvalidData;
        if self.number == 0 || self.number == u64::MAX || self.delivery.receiver_run_id.len() != 16
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

impl Store {
    pub fn open(root: &Path) -> io::Result<Self> {
        agora_platform::ensure_private_dir(root)?;
        let mut store = Self {
            root: root.to_path_buf(),
            records: BTreeMap::new(),
            bytes: 0,
        };
        for entry in std::fs::read_dir(root)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_name().to_str().is_some_and(stale_temporary) {
                std::fs::remove_file(&path)?;
                continue;
            }
            if path.extension().and_then(|value| value.to_str()) != Some("handoff") {
                continue;
            }
            if !entry.file_type()?.is_file() || store.records.len() == MAX_RECORDS {
                return Err(io::ErrorKind::InvalidData.into());
            }
            let remaining = MAX_BYTES - store.bytes;
            let mut bytes = Vec::new();
            std::fs::File::open(&path)?
                .take(remaining as u64 + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() > remaining {
                return Err(io::ErrorKind::InvalidData.into());
            }
            let record = Record::decode(&bytes)?;
            let id = record.id()?;
            if entry.file_name() != filename(id).as_str()
                || store.records.insert(id, (record, bytes.len())).is_some()
            {
                return Err(io::ErrorKind::InvalidData.into());
            }
            store.bytes += bytes.len();
        }
        Ok(store)
    }

    pub fn get(&self, id: &Id) -> Option<&Record> {
        self.records.get(id).map(|(record, _)| record)
    }

    pub fn records(&self) -> impl Iterator<Item = &Record> {
        self.records.values().map(|(record, _)| record)
    }

    pub fn insert(&mut self, record: Record) -> Result<(), InsertError> {
        let id = record.id().map_err(InsertError::Storage)?;
        if self.records.contains_key(&id) {
            return Err(InsertError::Storage(io::ErrorKind::AlreadyExists.into()));
        }
        let bytes = record.bytes().map_err(InsertError::Storage)?;
        if self.records.len() == MAX_RECORDS {
            return Err(InsertError::Full {
                resource: "handoff_records",
                limit: MAX_RECORDS,
            });
        }
        if bytes.len() > MAX_BYTES - self.bytes {
            return Err(InsertError::Full {
                resource: "handoff_bytes",
                limit: MAX_BYTES,
            });
        }
        agora_platform::write_private(&self.root.join(filename(id)), &bytes)
            .map_err(InsertError::Storage)?;
        self.records.insert(id, (record, bytes.len()));
        self.bytes += bytes.len();
        Ok(())
    }

    /// A receipt remains until its deadline. Payloads for live runs remain available to the
    /// non-destructive MCP inbox, even after the deadline.
    pub fn prune(&mut self, now: u64, live: impl Fn(&DeliverTaskHandoff) -> bool) {
        self.records.retain(|id, (record, bytes)| {
            if record.delivery.valid_until_ms > now || live(&record.delivery) {
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
