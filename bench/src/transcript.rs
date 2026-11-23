//! Chat transcript cost (ADR-0089, ADR-0104 C1): what saving a structured agent's conversation to the node's disk adds, against the
//! budgets in `docs/design/performance.md`. Four figures:
//!
//! - the chat path: one entry added to an agent's ring on a board that keeps memory only, and on one that saves (the only extra work
//!   is marking the entry's id), with the ring full so every entry also evicts one;
//! - the writer's time under the board's lock to take one batch (the clones of the changed entries), for a second's worth of a busy
//!   agent;
//! - the writer's time to encode, redact and append that batch (off the lock and off every agent's path);
//! - seeding at take-up: reading the end of a full file (at most 4 MiB of it) and filling the ring.

use std::time::{Duration, Instant};

use agora_adapters::host::ChatItem;
use agora_core::AgentId;
use agora_node::chat::ChatBoard;
use agora_node::transcript::{Limits, Store};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscriptReport {
    pub commit: String,
    /// Mean time to add one 200-byte text entry to a full ring, memory only and saving.
    pub push_memory_ns: f64,
    pub push_saving_ns: f64,
    /// Review of #1103 (N9): the same with 16 KiB entries, 300 between two batches: the 256 KiB ring holds 16 of them, so nearly every
    /// entry leaves it before its batch and the saving board copies it aside (`keep_unsaved`, at most 1 MiB kept).
    #[serde(default)]
    pub push_large_memory_ns: f64,
    #[serde(default)]
    pub push_large_saving_ns: f64,
    /// Entries in one batch, and the time under the board's lock to take it (p50, max).
    pub batch_entries: usize,
    pub take_p50_us: f64,
    pub take_max_us: f64,
    /// Encoding, redacting and appending that batch (p50, max).
    pub write_p50_ms: f64,
    pub write_max_ms: f64,
    /// Review of #1103 (N1): the same batch, all one stream of the agent's text, as the writer takes it (holding back the newest
    /// pieces and joining each run's context) when the node holds a secret value of 52 bytes and one of 8 KiB: under the lock, then
    /// redacting and appending (p50).
    #[serde(default)]
    pub take_52_p50_us: f64,
    #[serde(default)]
    pub write_52_p50_ms: f64,
    #[serde(default)]
    pub take_8k_p50_us: f64,
    #[serde(default)]
    pub write_8k_p50_ms: f64,
    /// The file seeding reads, and the time to read its end and fill the ring (p50, max).
    pub seed_file_bytes: u64,
    pub seed_p50_ms: f64,
    pub seed_max_ms: f64,
}

fn percentile(samples: &mut [Duration], p: f64) -> Duration {
    samples.sort();
    let i = ((samples.len() as f64 - 1.0) * p).round() as usize;
    samples.get(i).copied().unwrap_or_default()
}

/// Mean time to add one entry of `size` bytes, `per_batch` between two batches taken as the writer takes them (not timed).
fn push_batched_ns(board: &ChatBoard, n: usize, size: usize, per_batch: usize) -> f64 {
    let agent = AgentId::new("bench-batched");
    let text = "z".repeat(size);
    for _ in 0..agora_node::chat::MAX_ENTRIES {
        board.push_item(&agent, &ChatItem::Text(text.clone()), 1);
    }
    let _ = board.take_unsaved();
    let items: Vec<ChatItem> = (0..n).map(|_| ChatItem::Text(text.clone())).collect();
    let mut spent = Duration::ZERO;
    for chunk in items.chunks(per_batch) {
        let start = Instant::now();
        for item in chunk {
            board.push_item(&agent, item, 1);
        }
        spent += start.elapsed();
        let _ = board.take_unsaved();
    }
    spent.as_nanos() as f64 / n as f64
}

fn push_ns(board: &ChatBoard, n: usize) -> f64 {
    let agent = AgentId::new("bench");
    let text = "x".repeat(200);
    // Fill the ring first, so every entry measured also evicts one.
    for _ in 0..agora_node::chat::MAX_ENTRIES {
        board.push_item(&agent, &ChatItem::Text(text.clone()), 1);
    }
    let items: Vec<ChatItem> = (0..n).map(|_| ChatItem::Text(text.clone())).collect();
    let start = Instant::now();
    for item in &items {
        board.push_item(&agent, item, 1);
    }
    start.elapsed().as_nanos() as f64 / n as f64
}

pub fn run(commit: &str, runs: usize) -> std::io::Result<TranscriptReport> {
    let dir = std::env::temp_dir().join(format!("agora-bench-transcript-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let result = measure(&dir, commit, runs);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn measure(dir: &std::path::Path, commit: &str, runs: usize) -> std::io::Result<TranscriptReport> {
    let n = 200_000;
    let push_memory_ns = push_ns(&ChatBoard::new(), n);
    let store = std::sync::Arc::new(Store::new(dir, Limits::default(), None));
    // The saving board's marks are taken now and then, as the writer does, so they stay what one second holds.
    let saving = ChatBoard::saving(std::sync::Arc::clone(&store));
    let push_saving_ns = {
        let agent = AgentId::new("bench");
        let text = "x".repeat(200);
        for _ in 0..agora_node::chat::MAX_ENTRIES {
            saving.push_item(&agent, &ChatItem::Text(text.clone()), 1);
        }
        let _ = saving.take_unsaved();
        let items: Vec<ChatItem> = (0..n).map(|_| ChatItem::Text(text.clone())).collect();
        // Only the pushes are timed; the batches taken between them are the writer's, measured below.
        let mut spent = Duration::ZERO;
        for chunk in items.chunks(300) {
            let start = Instant::now();
            for item in chunk {
                saving.push_item(&agent, item, 1);
            }
            spent += start.elapsed();
            let _ = saving.take_unsaved();
        }
        spent.as_nanos() as f64 / n as f64
    };
    let _ = saving.take_unsaved();
    let large = 16 * 1024;
    let push_large_memory_ns = push_batched_ns(&ChatBoard::new(), 30_000, large, 300);
    let push_large_saving_ns = push_batched_ns(
        &ChatBoard::saving(std::sync::Arc::clone(&store)),
        30_000,
        large,
        300,
    );

    // A busy agent's second: 300 entries of 200 bytes (about 64 KiB, the batch's size bound).
    let batch_entries = 300;
    let agent = AgentId::new("agent-1");
    let text = "y".repeat(200);
    let (mut take, mut write) = (Vec::new(), Vec::new());
    for _ in 0..runs {
        for _ in 0..batch_entries {
            saving.push_item(&agent, &ChatItem::Text(text.clone()), 1);
        }
        let start = Instant::now();
        let batches = saving.take_unsaved();
        take.push(start.elapsed());
        let start = Instant::now();
        for b in &batches {
            store.write(b)?;
        }
        write.push(start.elapsed());
    }

    // The writer's timed take with a secret value held: one stream of text, the newest pieces held back.
    let with_secret = |tag: &str, value: String| -> std::io::Result<(f64, f64)> {
        let home = dir.join(tag);
        std::fs::create_dir_all(&home)?;
        let secrets = agora_node::credentials::SecretStore::in_data(&home);
        let secret = agora_node::credentials::Secret::new(&value)
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
        secrets
            .set("BENCH", &secret)
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::Other))?;
        let st = std::sync::Arc::new(Store::new(
            &home,
            Limits::default(),
            Some(agora_node::credentials::SecretStore::in_data(&home)),
        ));
        let board = ChatBoard::saving(std::sync::Arc::clone(&st));
        let agent = AgentId::new("agent-s");
        let (mut take, mut write) = (Vec::new(), Vec::new());
        let mut now = 1_000_000u64;
        for _ in 0..runs {
            for _ in 0..batch_entries {
                now += 1;
                board.push_item(&agent, &ChatItem::Text(text.clone()), now);
            }
            let start = Instant::now();
            let batches = board.take_due(now);
            take.push(start.elapsed());
            let start = Instant::now();
            for b in &batches {
                st.write(b)?;
            }
            write.push(start.elapsed());
        }
        Ok((
            percentile(&mut take, 0.5).as_secs_f64() * 1e6,
            percentile(&mut write, 0.5).as_secs_f64() * 1e3,
        ))
    };
    let (take_52_p50_us, write_52_p50_ms) = with_secret("s52", "s".repeat(52))?;
    let (take_8k_p50_us, write_8k_p50_ms) = with_secret("s8k", "k".repeat(8192))?;

    // A full file to seed from: 32 MiB of entries.
    let full = AgentId::new("agent-full");
    while std::fs::metadata(store.path(full.as_str()).unwrap_or_default()).map_or(0, |m| m.len())
        < Limits::default().max_bytes - 128 * 1024
    {
        for _ in 0..batch_entries {
            saving.push_item(&full, &ChatItem::Text(text.clone()), 1);
        }
        for b in saving.take_unsaved() {
            store.write(&b)?;
        }
    }
    let seed_file_bytes = std::fs::metadata(store.path(full.as_str()).unwrap_or_default())?.len();
    let mut seed = Vec::new();
    for _ in 0..runs {
        let board = ChatBoard::saving(std::sync::Arc::clone(&store));
        let start = Instant::now();
        board.seed_from_disk(&full, 1);
        seed.push(start.elapsed());
    }
    let us = |d: Duration| d.as_secs_f64() * 1e6;
    let ms = |d: Duration| d.as_secs_f64() * 1e3;
    Ok(TranscriptReport {
        commit: commit.to_string(),
        push_memory_ns,
        push_saving_ns,
        push_large_memory_ns,
        push_large_saving_ns,
        batch_entries,
        take_p50_us: us(percentile(&mut take, 0.5)),
        take_max_us: us(percentile(&mut take, 1.0)),
        write_p50_ms: ms(percentile(&mut write, 0.5)),
        write_max_ms: ms(percentile(&mut write, 1.0)),
        take_52_p50_us,
        write_52_p50_ms,
        take_8k_p50_us,
        write_8k_p50_ms,
        seed_file_bytes,
        seed_p50_ms: ms(percentile(&mut seed, 0.5)),
        seed_max_ms: ms(percentile(&mut seed, 1.0)),
    })
}
