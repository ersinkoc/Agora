//! Output rate coalescing: batch a terminal's output before it is fanned out to clients.
//!
//! A flooding terminal can produce thousands of tiny chunks per second. Sending each one as its own message
//! would cost more in framing and wakeups than the data is worth, so output is gathered and released when
//! either enough bytes have piled up or a short delay has passed. The delay is small (a frame at 60 fps), so
//! interactive typing still feels immediate: a lone echoed character leaves after at most `max_delay`.
//! Pure state, no threads or clocks of its own (the caller passes `now`), so it is deterministic to test.

use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct Coalescer {
    max_delay: Duration,
    max_bytes: usize,
    buf: Vec<u8>,
    first_at: Option<Instant>,
}

impl Coalescer {
    pub fn new(max_delay: Duration, max_bytes: usize) -> Self {
        Self {
            max_delay,
            max_bytes: max_bytes.max(1),
            buf: Vec::new(),
            first_at: None,
        }
    }

    /// Defaults: one frame at 60 fps, 64 KiB.
    pub fn with_defaults() -> Self {
        Self::new(Duration::from_millis(16), 64 * 1024)
    }

    /// Add output; returns a batch if the byte threshold was reached or the oldest pending byte is due.
    pub fn push(&mut self, now: Instant, bytes: &[u8]) -> Option<Vec<u8>> {
        if bytes.is_empty() {
            return self.poll(now);
        }
        self.first_at.get_or_insert(now);
        self.buf.extend_from_slice(bytes);
        if self.buf.len() >= self.max_bytes {
            return Some(self.take());
        }
        self.poll(now)
    }

    /// Release pending output if its delay has passed.
    pub fn poll(&mut self, now: Instant) -> Option<Vec<u8>> {
        let due = self
            .first_at
            .is_some_and(|t| now.duration_since(t) >= self.max_delay);
        due.then(|| self.take())
    }

    /// When the caller should next call [`poll`](Self::poll), if anything is pending.
    pub fn deadline(&self) -> Option<Instant> {
        self.first_at.map(|t| t + self.max_delay)
    }

    /// Take everything pending now (for example when the terminal closes).
    pub fn take(&mut self) -> Vec<u8> {
        self.first_at = None;
        std::mem::take(&mut self.buf)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn small_chunks_are_batched_until_the_delay() {
        let t0 = Instant::now();
        let mut c = Coalescer::new(ms(16), 1024);
        assert_eq!(c.push(t0, b"a"), None);
        assert_eq!(c.push(t0 + ms(5), b"b"), None);
        assert_eq!(c.deadline(), Some(t0 + ms(16)));
        assert_eq!(c.poll(t0 + ms(15)), None);
        assert_eq!(c.poll(t0 + ms(16)), Some(b"ab".to_vec()));
        assert_eq!(c.deadline(), None);
        assert_eq!(c.poll(t0 + ms(100)), None);
    }

    #[test]
    fn a_lone_keystroke_waits_at_most_one_delay() {
        let t0 = Instant::now();
        let mut c = Coalescer::new(ms(16), 1024);
        assert_eq!(c.push(t0, b"x"), None);
        assert_eq!(c.poll(c.deadline().unwrap()), Some(b"x".to_vec()));
    }

    #[test]
    fn the_byte_threshold_releases_immediately() {
        let t0 = Instant::now();
        let mut c = Coalescer::new(ms(1000), 8);
        assert_eq!(c.push(t0, b"1234"), None);
        assert_eq!(c.push(t0, b"5678"), Some(b"12345678".to_vec()));
        assert_eq!(c.deadline(), None);
    }

    #[test]
    fn a_flood_becomes_few_batches_and_loses_nothing() {
        let t0 = Instant::now();
        let mut c = Coalescer::new(ms(16), 4096);
        let (mut batches, mut total) = (0, 0);
        // 20 000 chunks of 50 bytes arriving one per 50 microseconds (about 1 s of 20 MB/s).
        for i in 0..20_000u64 {
            let now = t0 + Duration::from_micros(i * 50);
            if let Some(b) = c.push(now, &[b'x'; 50]) {
                batches += 1;
                total += b.len();
            }
        }
        total += c.take().len();
        assert_eq!(total, 20_000 * 50);
        assert!(batches < 500, "{batches} batches for 20000 chunks");
    }

    #[test]
    fn take_flushes_what_is_pending() {
        let t0 = Instant::now();
        let mut c = Coalescer::with_defaults();
        c.push(t0, b"bye");
        assert_eq!(c.take(), b"bye".to_vec());
        assert!(c.take().is_empty());
    }
}
