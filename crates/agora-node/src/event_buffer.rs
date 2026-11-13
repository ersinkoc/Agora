//! Durable event buffer: the node's append-only on-disk log of events the hub has not acknowledged.
//!
//! Protocol rule (docs/specs/protocol.md §3): events are numbered 1, 2, 3, … per producer; the hub
//! acknowledges the highest contiguous number; the node keeps everything after the ack and replays it, in
//! order, after a reconnect. This is the persistent counterpart of `agora_core::EventLog`: same numbering
//! and replay rules, but it survives a node restart. Payloads are opaque bytes (encoded protocol events).
//!
//! Layout in the buffer directory:
//! - `events.log`: header (`AGORAEV1`, `first_seq` u64 LE) then records `len` u32 LE, `checksum` u32 LE
//!   (FNV-1a of the payload), payload. A record cut short by a crash or failing its checksum ends the log;
//!   the damaged tail is truncated on open, so a torn final write never poisons earlier events.
//! - `ack`: the acknowledged sequence number, u64 LE, replaced atomically (write + rename).
//!
//! Size is capped. When the log outgrows `max_bytes`, the oldest events are dropped down to 80% of the cap.
//! If any dropped event was not yet acknowledged, a later [`DiskEventLog::replay_after`] from the old ack
//! returns [`ReplayError::Stale`] and the consumer must resynchronize from a snapshot instead (the same rule
//! as the in-memory log).

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 8] = b"AGORAEV1";
const HEADER_LEN: u64 = 16;
const RECORD_HEAD: u64 = 8;
/// Refuse absurd records (a corrupted length must not make us allocate gigabytes).
const MAX_PAYLOAD: u32 = 16 * 1024 * 1024;
/// Do not rewrite the file to reclaim acknowledged events until at least this much is reclaimable.
const MIN_COMPACT: u64 = 64 * 1024;

#[derive(Debug)]
pub enum ReplayError {
    /// Events after the requested point were dropped to respect the size cap; resync from a snapshot.
    Stale {
        oldest_retained: u64,
    },
    Io(io::Error),
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stale { oldest_retained } => {
                write!(f, "events before {oldest_retained} were dropped")
            }
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ReplayError {}

impl From<io::Error> for ReplayError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Result of [`DiskEventLog::append`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Appended {
    pub seq: u64,
    /// Unacknowledged events dropped by the size cap while making room (0 in normal operation).
    pub evicted_unacked: u64,
}

fn fnv1a(data: &[u8]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in data {
        h ^= u32::from(*b);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

fn encode(payload: &[u8]) -> io::Result<Vec<u8>> {
    let len = u32::try_from(payload.len())
        .ok()
        .filter(|l| *l <= MAX_PAYLOAD)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "event too large"))?;
    let mut buf = Vec::with_capacity(payload.len() + RECORD_HEAD as usize);
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(&fnv1a(payload).to_le_bytes());
    buf.extend_from_slice(payload);
    Ok(buf)
}

/// Read one record; `None` at a clean end, a short record, or a bad checksum.
fn read_record(r: &mut impl Read) -> Option<Vec<u8>> {
    let mut head = [0u8; RECORD_HEAD as usize];
    r.read_exact(&mut head).ok()?;
    let len = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
    let sum = u32::from_le_bytes([head[4], head[5], head[6], head[7]]);
    if len > MAX_PAYLOAD {
        return None;
    }
    let mut payload = vec![0u8; len as usize];
    r.read_exact(&mut payload).ok()?;
    (fnv1a(&payload) == sum).then_some(payload)
}

pub struct DiskEventLog {
    dir: PathBuf,
    file: File,
    /// Start offset of each retained record; index `i` is sequence `first_seq + i`.
    offsets: Vec<u64>,
    first_seq: u64,
    acked: u64,
    /// Current file length.
    end: u64,
    max_bytes: u64,
}

impl DiskEventLog {
    /// Open (or create) the buffer in `dir`, recovering from a torn tail.
    pub fn open(dir: &Path, max_bytes: u64) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let log_path = dir.join("events.log");
        if !log_path.exists() {
            write_log(&log_path, 1, &[])?;
        }
        let mut file = OpenOptions::new().read(true).write(true).open(&log_path)?;
        let mut header = [0u8; HEADER_LEN as usize];
        file.read_exact(&mut header)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "event log header missing"))?;
        if &header[..8] != MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "not an event log",
            ));
        }
        let mut first = [0u8; 8];
        first.copy_from_slice(&header[8..]);
        let first_seq = u64::from_le_bytes(first).max(1);

        let mut offsets = Vec::new();
        let mut pos = HEADER_LEN;
        {
            let mut reader = io::BufReader::new(&file);
            while let Some(payload) = read_record(&mut reader) {
                offsets.push(pos);
                pos += RECORD_HEAD + payload.len() as u64;
            }
        }
        // Drop a damaged tail so appends continue from the last good record.
        if file.metadata()?.len() != pos {
            file.set_len(pos)?;
        }
        file.seek(SeekFrom::End(0))?;

        let next_seq = first_seq + offsets.len() as u64;
        let acked = fs::read(dir.join("ack"))
            .ok()
            .and_then(|b| <[u8; 8]>::try_from(b.as_slice()).ok())
            .map_or(0, u64::from_le_bytes)
            .min(next_seq - 1);
        Ok(Self {
            dir: dir.to_path_buf(),
            file,
            offsets,
            first_seq,
            acked,
            end: pos,
            max_bytes,
        })
    }

    /// The number the next appended event will get.
    pub fn next_seq(&self) -> u64 {
        self.first_seq + self.offsets.len() as u64
    }

    /// Lowest sequence number still retained (equals `next_seq` when nothing is retained).
    pub fn oldest_retained(&self) -> u64 {
        self.first_seq
    }

    pub fn acked(&self) -> u64 {
        self.acked
    }

    /// Number of retained events (acknowledged ones are reclaimed lazily).
    pub fn len(&self) -> usize {
        self.offsets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    /// Append an event. It reaches the OS before this returns (it survives a process crash); call
    /// [`sync`](Self::sync) at batch boundaries to also survive power loss.
    pub fn append(&mut self, payload: &[u8]) -> io::Result<Appended> {
        let buf = encode(payload)?;
        self.file.write_all(&buf)?;
        self.offsets.push(self.end);
        self.end += buf.len() as u64;
        let seq = self.next_seq() - 1;
        let evicted_unacked = if self.end > self.max_bytes {
            self.evict()?
        } else {
            0
        };
        Ok(Appended {
            seq,
            evicted_unacked,
        })
    }

    /// Flush buffered data to stable storage.
    pub fn sync(&mut self) -> io::Result<()> {
        self.file.sync_data()
    }

    /// The consumer acknowledged everything up to and including `seq`. Acks never move backwards and are
    /// clamped to what was produced. The ack is durable before this returns.
    pub fn ack(&mut self, seq: u64) -> io::Result<()> {
        let seq = seq.min(self.next_seq() - 1);
        if seq <= self.acked {
            return Ok(());
        }
        self.acked = seq;
        let tmp = self.dir.join("ack.tmp");
        {
            let mut f = File::create(&tmp)?;
            f.write_all(&seq.to_le_bytes())?;
            f.sync_data()?;
        }
        fs::rename(&tmp, self.dir.join("ack"))?;
        self.compact()
    }

    /// Starts numbering at `next` and forgets what is retained: for a node whose consumer is already past everything it has
    /// (the consumer acknowledged those numbers for an earlier life of this producer, and this buffer started over). A no-op when
    /// `next` is not ahead of the next number. Everything before `next` counts as acknowledged afterwards.
    pub fn advance_to(&mut self, next: u64) -> io::Result<()> {
        if next <= self.next_seq() {
            return Ok(());
        }
        let path = self.dir.join("events.log");
        write_log(&path, next, &[])?;
        let mut file = OpenOptions::new().read(true).write(true).open(&path)?;
        file.seek(SeekFrom::End(0))?;
        self.file = file;
        self.offsets.clear();
        self.first_seq = next;
        self.end = HEADER_LEN;
        self.ack(next - 1)
    }

    /// The first retained event not yet acknowledged for which `pick` says yes, read from disk one record at a time (a start-up
    /// check must not hold the whole backlog in memory).
    pub fn find_unacked(&self, mut pick: impl FnMut(&[u8]) -> bool) -> io::Result<Option<Vec<u8>>> {
        let idx =
            usize::try_from((self.acked + 1).saturating_sub(self.first_seq)).unwrap_or(usize::MAX);
        let Some(start) = self.offsets.get(idx) else {
            return Ok(None);
        };
        let mut f = File::open(self.dir.join("events.log"))?;
        f.seek(SeekFrom::Start(*start))?;
        let mut reader = io::BufReader::new(f.take(self.end - start));
        while let Some(p) = read_record(&mut reader) {
            if pick(&p) {
                return Ok(Some(p));
            }
        }
        Ok(None)
    }

    /// Moves this log out of the way, never deleting it: `events.log` becomes `events.<label>.<unix_secs>.log` in the same folder
    /// (and the ack file `events.<label>.<unix_secs>.ack`), with `-2`, `-3`, ... added when that name is taken. `label` is reduced
    /// to `[A-Za-z0-9._-]` (others become `_`, at most 64 characters). Returns the new path of the log; opening the folder
    /// afterwards starts a fresh, empty log numbered from 1.
    pub fn set_aside(self, label: &str, unix_secs: u64) -> io::Result<PathBuf> {
        let Self { dir, file, .. } = self;
        drop(file);
        let label: String = label
            .chars()
            .take(64)
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let label = if label.is_empty() { "_".into() } else { label };
        let mut stem = format!("events.{label}.{unix_secs}");
        let mut n = 1;
        while dir.join(format!("{stem}.log")).exists() || dir.join(format!("{stem}.ack")).exists() {
            n += 1;
            stem = format!("events.{label}.{unix_secs}-{n}");
        }
        let log = dir.join(format!("{stem}.log"));
        fs::rename(dir.join("events.log"), &log)?;
        let ack = dir.join("ack");
        if ack.exists() {
            fs::rename(ack, dir.join(format!("{stem}.ack")))?;
        }
        Ok(log)
    }

    /// Events after `after`, in order, for replay following a reconnect (`after` is the consumer's ack).
    pub fn replay_after(&self, after: u64) -> Result<Vec<Vec<u8>>, ReplayError> {
        self.replay_after_max(after, usize::MAX)
    }

    /// Like [`replay_after`](Self::replay_after), at most `max` events, reading only those from disk (one uplink batch
    /// must not cost a read of the whole backlog).
    pub fn replay_after_max(&self, after: u64, max: usize) -> Result<Vec<Vec<u8>>, ReplayError> {
        if after.saturating_add(1) < self.first_seq {
            return Err(ReplayError::Stale {
                oldest_retained: self.first_seq,
            });
        }
        let idx = usize::try_from(after + 1 - self.first_seq).unwrap_or(usize::MAX);
        let Some(start) = self.offsets.get(idx) else {
            return Ok(Vec::new());
        };
        let stop = self
            .offsets
            .get(idx.saturating_add(max))
            .copied()
            .unwrap_or(self.end);
        let mut f = File::open(self.dir.join("events.log"))?;
        f.seek(SeekFrom::Start(*start))?;
        let mut reader = io::BufReader::new(f.take(stop - start));
        let mut out = Vec::with_capacity((self.offsets.len() - idx).min(max));
        while let Some(p) = read_record(&mut reader) {
            out.push(p);
        }
        Ok(out)
    }

    /// Reclaim acknowledged events once enough of the file is dead weight.
    fn compact(&mut self) -> io::Result<()> {
        // The size cap may already have dropped events past the ack: then nothing retained is acknowledged.
        let dead = usize::try_from((self.acked + 1).saturating_sub(self.first_seq))
            .unwrap_or(0)
            .min(self.offsets.len());
        if dead == 0 {
            return Ok(());
        }
        let dead_bytes = self.offsets.get(dead).copied().unwrap_or(self.end) - HEADER_LEN;
        if dead_bytes >= MIN_COMPACT.max((self.end - HEADER_LEN) / 2) {
            self.rewrite_from(dead)?;
        }
        Ok(())
    }

    /// Size cap: drop the oldest events down to 80% of the cap (always keeping the newest). Returns how
    /// many dropped events had not been acknowledged.
    fn evict(&mut self) -> io::Result<u64> {
        let target = self.max_bytes / 10 * 8;
        let keep_from = self
            .offsets
            .iter()
            .position(|o| self.end - o <= target)
            .unwrap_or(self.offsets.len() - 1);
        if keep_from == 0 {
            return Ok(0);
        }
        // Events before `first_seq` were dropped (and counted) by an earlier eviction.
        let first_dropped_unacked = (self.acked + 1).max(self.first_seq);
        let last_dropped = self.first_seq + keep_from as u64 - 1;
        let evicted = if last_dropped >= first_dropped_unacked {
            last_dropped - first_dropped_unacked + 1
        } else {
            0
        };
        self.rewrite_from(keep_from)?;
        Ok(evicted)
    }

    /// Rewrite the log without its first `drop` records (atomically, via a temporary file).
    fn rewrite_from(&mut self, drop: usize) -> io::Result<()> {
        let Some(&start) = self.offsets.get(drop) else {
            // Dropping everything: start an empty log at the next sequence number.
            let next = self.next_seq();
            write_log(&self.dir.join("events.log"), next, &[])?;
            self.reopen(next, Vec::new(), HEADER_LEN)?;
            return Ok(());
        };
        let mut tail = vec![0u8; usize::try_from(self.end - start).unwrap_or(0)];
        let mut f = File::open(self.dir.join("events.log"))?;
        f.seek(SeekFrom::Start(start))?;
        f.read_exact(&mut tail)?;
        let new_first = self.first_seq + drop as u64;
        write_log(&self.dir.join("events.log"), new_first, &tail)?;
        let shift = start - HEADER_LEN;
        let offsets = self.offsets[drop..].iter().map(|o| o - shift).collect();
        self.reopen(new_first, offsets, HEADER_LEN + tail.len() as u64)
    }

    fn reopen(&mut self, first_seq: u64, offsets: Vec<u64>, end: u64) -> io::Result<()> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(self.dir.join("events.log"))?;
        file.seek(SeekFrom::End(0))?;
        self.file = file;
        self.first_seq = first_seq;
        self.offsets = offsets;
        self.end = end;
        Ok(())
    }
}

/// Write a complete log file atomically: temporary file, flush to disk, rename over the target.
fn write_log(path: &Path, first_seq: u64, records: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut f = File::create(&tmp)?;
        f.write_all(MAGIC)?;
        f.write_all(&first_seq.to_le_bytes())?;
        f.write_all(records)?;
        f.sync_data()?;
    }
    fs::rename(&tmp, path)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn dir() -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!(
            "agora-evbuf-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&d);
        d
    }

    fn ev(i: u32) -> Vec<u8> {
        format!("event-{i}").into_bytes()
    }

    fn evs(range: std::ops::RangeInclusive<u32>) -> Vec<Vec<u8>> {
        range.map(ev).collect()
    }

    #[test]
    fn numbers_events_from_one_and_replays_in_order() {
        let mut log = DiskEventLog::open(&dir(), 1 << 20).unwrap();
        assert_eq!(log.next_seq(), 1);
        for i in 1..=5 {
            assert_eq!(
                log.append(&ev(i)).unwrap(),
                Appended {
                    seq: u64::from(i),
                    evicted_unacked: 0
                }
            );
        }
        assert_eq!(log.replay_after(0).unwrap(), evs(1..=5));
        assert_eq!(log.replay_after(3).unwrap(), evs(4..=5));
        assert!(log.replay_after(5).unwrap().is_empty());
        assert!(log.replay_after(99).unwrap().is_empty());
    }

    #[test]
    fn a_bounded_replay_reads_at_most_max_events_with_the_same_rules() {
        let mut log = DiskEventLog::open(&dir(), 1 << 20).unwrap();
        for i in 1..=5 {
            log.append(&ev(i)).unwrap();
        }
        assert_eq!(log.replay_after_max(0, 2).unwrap(), evs(1..=2));
        assert_eq!(log.replay_after_max(3, 2).unwrap(), evs(4..=5));
        assert_eq!(log.replay_after_max(3, 99).unwrap(), evs(4..=5));
        assert!(log.replay_after_max(0, 0).unwrap().is_empty());
        assert!(log.replay_after_max(5, 2).unwrap().is_empty());
    }

    #[test]
    fn restart_replays_exactly_the_events_after_the_last_ack() {
        let d = dir();
        {
            let mut log = DiskEventLog::open(&d, 1 << 20).unwrap();
            for i in 1..=10 {
                log.append(&ev(i)).unwrap();
            }
            log.ack(4).unwrap();
            // The process dies here without a clean shutdown: nothing is flushed explicitly.
        }
        let mut log = DiskEventLog::open(&d, 1 << 20).unwrap();
        assert_eq!(log.acked(), 4);
        assert_eq!(log.next_seq(), 11);
        assert_eq!(log.replay_after(log.acked()).unwrap(), evs(5..=10));
        // Numbering continues where it left off.
        assert_eq!(log.append(&ev(11)).unwrap().seq, 11);
        log.ack(11).unwrap();
        drop(log);
        let log = DiskEventLog::open(&d, 1 << 20).unwrap();
        assert!(log.replay_after(log.acked()).unwrap().is_empty());
        assert_eq!(log.next_seq(), 12);
    }

    #[test]
    fn acks_never_move_backwards_and_are_clamped() {
        let mut log = DiskEventLog::open(&dir(), 1 << 20).unwrap();
        for i in 1..=3 {
            log.append(&ev(i)).unwrap();
        }
        log.ack(2).unwrap();
        log.ack(1).unwrap();
        assert_eq!(log.acked(), 2);
        log.ack(1000).unwrap();
        assert_eq!(log.acked(), 3);
    }

    #[test]
    fn find_unacked_looks_only_past_the_ack() {
        let mut log = DiskEventLog::open(&dir(), 1 << 20).unwrap();
        for i in 1..=4 {
            log.append(&ev(i)).unwrap();
        }
        log.ack(2).unwrap();
        assert_eq!(log.find_unacked(|p| p == ev(1)).unwrap(), None);
        assert_eq!(log.find_unacked(|p| p == ev(3)).unwrap(), Some(ev(3)));
        log.ack(4).unwrap();
        assert_eq!(log.find_unacked(|_| true).unwrap(), None);
    }

    #[test]
    fn a_log_set_aside_is_renamed_never_deleted_and_a_fresh_one_starts_at_1() {
        let d = dir();
        let mut log = DiskEventLog::open(&d, 1 << 20).unwrap();
        for i in 1..=3 {
            log.append(&ev(i)).unwrap();
        }
        log.ack(1).unwrap();
        let kept = log.set_aside("lo/cal", 1700).unwrap();
        assert_eq!(kept, d.join("events.lo_cal.1700.log"));
        assert!(d.join("events.lo_cal.1700.ack").exists());
        assert!(!d.join("ack").exists());
        let old = DiskEventLog::open(&d, 1 << 20).unwrap();
        assert_eq!((old.next_seq(), old.acked()), (1, 0));
        // The same name in the same second does not overwrite the first.
        assert_eq!(
            old.set_aside("lo/cal", 1700).unwrap(),
            d.join("events.lo_cal.1700-2.log")
        );
        let bytes = fs::read(&kept).unwrap();
        assert_eq!(&bytes[..8], MAGIC);
    }

    #[test]
    fn a_torn_final_write_is_discarded_and_earlier_events_survive() {
        let d = dir();
        {
            let mut log = DiskEventLog::open(&d, 1 << 20).unwrap();
            for i in 1..=3 {
                log.append(&ev(i)).unwrap();
            }
        }
        let path = d.join("events.log");
        let len = fs::metadata(&path).unwrap().len();
        // Cut the last record in half, as a crash mid-write would.
        OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(len - 4)
            .unwrap();
        let mut log = DiskEventLog::open(&d, 1 << 20).unwrap();
        assert_eq!(log.replay_after(0).unwrap(), evs(1..=2));
        assert_eq!(log.append(&ev(30)).unwrap().seq, 3);
        assert_eq!(log.replay_after(0).unwrap(), vec![ev(1), ev(2), ev(30)]);
    }

    #[test]
    fn a_corrupted_record_ends_the_log_there() {
        let d = dir();
        {
            let mut log = DiskEventLog::open(&d, 1 << 20).unwrap();
            for i in 1..=3 {
                log.append(&ev(i)).unwrap();
            }
        }
        let path = d.join("events.log");
        let mut bytes = fs::read(&path).unwrap();
        // Flip a byte inside the second record's payload.
        let second =
            HEADER_LEN as usize + RECORD_HEAD as usize + ev(1).len() + RECORD_HEAD as usize;
        bytes[second] ^= 0xff;
        fs::write(&path, bytes).unwrap();
        let log = DiskEventLog::open(&d, 1 << 20).unwrap();
        assert_eq!(log.replay_after(0).unwrap(), evs(1..=1));
    }

    #[test]
    fn the_size_cap_drops_oldest_events_and_makes_old_resumes_stale() {
        let d = dir();
        let rec = encode(&ev(100)).unwrap().len() as u64;
        // Room for about ten records.
        let mut log = DiskEventLog::open(&d, rec * 10 + HEADER_LEN).unwrap();
        let mut evicted = 0;
        for i in 100..130 {
            evicted += log.append(&ev(i)).unwrap().evicted_unacked;
        }
        assert!(evicted > 0, "unacknowledged events were dropped");
        assert!(log.len() <= 10);
        assert_eq!(log.next_seq(), 31);
        assert!(log.oldest_retained() > 1);
        // The consumer is far behind: it must resync, not receive a gap silently.
        match log.replay_after(0) {
            Err(ReplayError::Stale { oldest_retained }) => {
                assert_eq!(oldest_retained, log.oldest_retained())
            }
            other => panic!("expected Stale, got {other:?}"),
        }
        // A consumer that is current is fine, and the newest events are intact.
        let tail = log.replay_after(log.oldest_retained() - 1).unwrap();
        assert_eq!(tail.last().unwrap(), &ev(129));
        assert_eq!(tail.len(), log.len());
        // The eviction itself survives a restart.
        let n = log.next_seq();
        let first = log.oldest_retained();
        drop(log);
        let log = DiskEventLog::open(&d, rec * 10 + HEADER_LEN).unwrap();
        assert_eq!((log.oldest_retained(), log.next_seq()), (first, n));
    }

    #[test]
    fn an_ack_older_than_what_the_size_cap_kept_drops_nothing_retained() {
        let d = dir();
        let cap = 200 * 1024;
        let mut log = DiskEventLog::open(&d, cap).unwrap();
        // Events 1..=10 are on their way to the hub when a burst overflows the cap and evicts past them.
        for _ in 0..410 {
            log.append(&[7u8; 1000]).unwrap();
        }
        let (len, oldest) = (log.len(), log.oldest_retained());
        assert!(oldest > 11 && len * 1008 >= MIN_COMPACT as usize);
        // Then the hub's ack for that earlier batch arrives.
        log.ack(10).unwrap();
        assert_eq!(
            (log.acked(), log.len(), log.oldest_retained()),
            (10, len, oldest)
        );
        assert_eq!(log.replay_after(oldest - 1).unwrap().len(), len);
        drop(log);
        let log = DiskEventLog::open(&d, cap).unwrap();
        assert_eq!(
            (log.acked(), log.len(), log.oldest_retained()),
            (10, len, oldest)
        );
    }

    #[test]
    fn each_unacknowledged_event_the_size_cap_drops_is_counted_once() {
        let mut log = DiskEventLog::open(&dir(), HEADER_LEN + 108 * 10).unwrap();
        let mut reported = 0;
        for _ in 0..40 {
            reported += log.append(&[1u8; 100]).unwrap().evicted_unacked;
        }
        // Nothing was acknowledged, so every event before the oldest retained one was dropped unacknowledged, once.
        assert!(log.oldest_retained() > 11);
        assert_eq!(reported, log.oldest_retained() - 1);
    }

    #[test]
    fn acknowledged_events_are_reclaimed_without_changing_numbering() {
        let d = dir();
        let mut log = DiskEventLog::open(&d, 64 << 20).unwrap();
        let big = vec![7u8; 1024];
        for _ in 0..200 {
            log.append(&big).unwrap();
        }
        let before = fs::metadata(d.join("events.log")).unwrap().len();
        log.ack(150).unwrap();
        let after = fs::metadata(d.join("events.log")).unwrap().len();
        assert!(after < before / 2, "{after} vs {before}");
        assert_eq!(log.next_seq(), 201);
        assert_eq!(log.replay_after(150).unwrap().len(), 50);
        let next = log.append(b"x").unwrap().seq;
        assert_eq!(next, 201);
        drop(log);
        let log = DiskEventLog::open(&d, 64 << 20).unwrap();
        assert_eq!(log.acked(), 150);
        assert_eq!(log.replay_after(150).unwrap().len(), 51);
    }

    #[test]
    fn rejects_foreign_files_and_oversized_events() {
        let d = dir();
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("events.log"), b"this is not an event log at all").unwrap();
        assert!(DiskEventLog::open(&d, 1 << 20).is_err());
        let mut log = DiskEventLog::open(&dir(), 1 << 20).unwrap();
        let huge = vec![0u8; MAX_PAYLOAD as usize + 1];
        assert!(log.append(&huge).is_err());
        assert_eq!(log.next_seq(), 1);
    }

    #[test]
    fn advancing_past_a_consumer_that_is_ahead_drops_the_old_events_and_numbers_on_from_there() {
        let dir = std::env::temp_dir().join(format!("agora-evbuf-adv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut log = DiskEventLog::open(&dir, 1 << 20).unwrap();
        log.append(b"a").unwrap();
        log.append(b"b").unwrap();
        assert_eq!(log.next_seq(), 3);
        log.advance_to(2).unwrap(); // not ahead: nothing changes
        assert_eq!((log.next_seq(), log.len()), (3, 2));
        log.advance_to(10).unwrap();
        assert_eq!(
            (
                log.next_seq(),
                log.oldest_retained(),
                log.acked(),
                log.len()
            ),
            (10, 10, 9, 0)
        );
        assert_eq!(log.append(b"c").unwrap().seq, 10);
        // It survives a restart.
        drop(log);
        let log = DiskEventLog::open(&dir, 1 << 20).unwrap();
        assert_eq!((log.next_seq(), log.acked(), log.len()), (11, 9, 1));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
