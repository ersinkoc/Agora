//! Chat transcript cost (ADR-0089, ADR-0104 C1): what saving a structured agent's conversation to the node's disk adds, against the
//! budgets in `docs/design/performance.md`. Four figures:
//!
//! - the chat path: one entry added to an agent's ring on a board that keeps memory only, and on one that saves (the only extra work
//!   is marking the entry's id), with the ring full so every entry also evicts one;
//! - the writer's time under the board's lock to take one batch (the clones of the changed entries), for a second's worth of a busy
//!   agent;
//! - the writer's time to encode, redact and append that batch (off the lock and off every agent's path);
//! - seeding at take-up: reading the end of a full file (at most 4 MiB of it) and filling the ring;
//! - #1147, the live path: one piece of the agent's words added and sent to a follower, redacted, in a turn, with no value held, one
//!   of 52 bytes, one of 8 KiB, 64 of 8,000 bytes, and the 52-byte value itself streamed in 4-byte pieces (each held, then sent).

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
    /// Review of #1103, round 4: writing one 16 KiB tool output dense with edges (the bridge's `[redacted] ` marks) with an 8 KiB value
    /// held (p50, ms): linear in the text, whatever the number of edges.
    #[serde(default)]
    pub write_dense_8k_p50_ms: f64,
    /// #1147: mean time to add one 200-byte piece of the agent's words and send it to a follower, in a turn, with no value held, a
    /// 52-byte one, an 8 KiB one and 64 of 8,000 bytes; and with the 52-byte value streamed in 4-byte pieces among the words (ns).
    #[serde(default)]
    pub live_none_ns: f64,
    #[serde(default)]
    pub live_52_ns: f64,
    #[serde(default)]
    pub live_8k_ns: f64,
    #[serde(default)]
    pub live_64x8000_ns: f64,
    #[serde(default)]
    pub live_split_52_ns: f64,
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
        settled(secrets.dir())?;
        let st = std::sync::Arc::new(Store::new(
            &home,
            Limits::default(),
            Some(agora_node::credentials::SecretStore::in_data(&home)),
        ));
        let board = ChatBoard::saving(std::sync::Arc::clone(&st));
        let agent = AgentId::new("agent-s");
        // In a turn: the newest words are held back, and each take joins the carry, the words written and those held.
        board.set_working(&agent, true);
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
    let write_dense_8k_p50_ms = {
        let home = dir.join("dense");
        std::fs::create_dir_all(&home)?;
        let value = format!("Q{}", "abcdefghij".repeat(800));
        let secret = agora_node::credentials::Secret::new(&value)
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
        let secrets = agora_node::credentials::SecretStore::in_data(&home);
        secrets
            .set("BENCH", &secret)
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::Other))?;
        settled(secrets.dir())?;
        let st = std::sync::Arc::new(Store::new(
            &home,
            Limits::default(),
            Some(agora_node::credentials::SecretStore::in_data(&home)),
        ));
        let board = ChatBoard::saving(std::sync::Arc::clone(&st));
        let agent = AgentId::new("agent-d");
        let mut times = Vec::new();
        for i in 0..runs {
            let item = agora_adapters::host::ToolItem {
                call_id: format!("c{i}"),
                tool: Some("sh".into()),
                status: "completed".into(),
                output: Some("[redacted] ".repeat(1480)),
                ..Default::default()
            };
            board.push_item(&agent, &ChatItem::Tool(item), 1);
            let batches = board.take_unsaved();
            let start = Instant::now();
            for b in &batches {
                st.write(b)?;
            }
            times.push(start.elapsed());
        }
        percentile(&mut times, 0.5).as_secs_f64() * 1e3
    };

    let live = |tag: &str, values: Vec<String>, split: bool| -> std::io::Result<f64> {
        let home = dir.join(tag);
        std::fs::create_dir_all(&home)?;
        let secrets = agora_node::credentials::SecretStore::in_data(&home);
        for (i, v) in values.iter().enumerate() {
            let secret = agora_node::credentials::Secret::new(v)
                .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
            secrets
                .set(&format!("BENCH{i}"), &secret)
                .map_err(|_| std::io::Error::from(std::io::ErrorKind::Other))?;
        }
        if !values.is_empty() {
            settled(secrets.dir())?;
        }
        let st = std::sync::Arc::new(Store::new(
            &home,
            Limits::default(),
            Some(agora_node::credentials::SecretStore::in_data(&home)),
        ));
        let board = ChatBoard::saving(st);
        let agent = AgentId::new("agent-live");
        board.set_working(&agent, true);
        let (_, sub) = board.follow(&agent);
        let pieces: Vec<String> = if split {
            // Words, then the value in 4-byte pieces, then words: every piece of the value waits until the next one.
            let v = values.first().cloned().unwrap_or_default();
            let mut p = vec![text.clone()];
            p.extend(
                v.as_bytes()
                    .chunks(4)
                    .map(|c| String::from_utf8_lossy(c).into_owned()),
            );
            p
        } else {
            vec![text.clone()]
        };
        let n = 20_000usize;
        let items: Vec<ChatItem> = (0..n)
            .map(|i| ChatItem::Text(pieces[i % pieces.len()].clone()))
            .collect();
        let mut spent = Duration::ZERO;
        for chunk in items.chunks(256) {
            let start = Instant::now();
            for item in chunk {
                board.push_item(&agent, item, 1);
            }
            spent += start.elapsed();
            while sub.recv_timeout(Duration::ZERO).is_ok() {}
            let _ = board.take_unsaved();
        }
        Ok(spent.as_nanos() as f64 / n as f64)
    };
    let live_none_ns = live("live-none", Vec::new(), false)?;
    let live_52_ns = live("live-52", vec!["s".repeat(52)], false)?;
    let live_8k_ns = live("live-8k", vec!["k".repeat(8192)], false)?;
    let live_64x8000_ns = live(
        "live-64",
        (0..64)
            .map(|i| {
                format!(
                    "{i:02}{}",
                    "abcdefghijklmnopqrstuvwxyz0123456789".repeat(222)
                )
            })
            .collect(),
        false,
    )?;
    let live_split_52_ns = live(
        "live-split",
        vec!["QWERTYUIOPASDFGHJKLZXCVBNM-0123456789-qwertyuiopasdf".into()],
        true,
    )?;

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
        write_dense_8k_p50_ms,
        live_none_ns,
        live_52_ns,
        live_8k_ns,
        live_64x8000_ns,
        live_split_52_ns,
        seed_file_bytes,
        seed_p50_ms: ms(percentile(&mut seed, 0.5)),
        seed_max_ms: ms(percentile(&mut seed, 1.0)),
    })
}

/// The secrets folder as a node sees it once a value has been there a while: its times a minute old, so the store reads its values
/// once (it reads them at every write while a time is under 2 s old).
fn settled(dir: &std::path::Path) -> std::io::Result<()> {
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
    for entry in std::fs::read_dir(dir)? {
        std::fs::File::options()
            .write(true)
            .open(entry?.path())?
            .set_modified(old)?;
    }
    std::fs::File::open(dir)?.set_modified(old)
}
