//! Measurement only (architecture review R4): hub cold start against history size. Run with
//! `AGORA_MEASURE_DB=<copy of a hub.db> cargo test --release -p agora-hub --test startup_measure -- --ignored --nocapture`.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use agora_hub::{
    hub::Hub,
    projection::View,
    store::{EventStore, SqliteStore},
};
use agora_protocol::v1::Event;
use prost::Message;
use std::collections::BTreeMap;
use std::time::Instant;

fn all(store: &SqliteStore) -> Vec<agora_hub::store::StoredEvent> {
    let (mut out, mut at) = (Vec::new(), 0);
    loop {
        let page = store.events_after(at, 1000).unwrap();
        let Some(last) = page.last() else { return out };
        at = last.pos;
        out.extend(page);
    }
}

fn size(path: &std::path::Path) -> u64 {
    ["", "-wal"]
        .iter()
        .map(|s| {
            std::fs::metadata(format!("{}{s}", path.display()))
                .map(|m| m.len())
                .unwrap_or(0)
        })
        .sum()
}

#[test]
#[ignore]
fn hub_cold_start_against_history_size() {
    let src =
        std::path::PathBuf::from(std::env::var("AGORA_MEASURE_DB").expect("AGORA_MEASURE_DB"));
    let real = SqliteStore::open(&src).unwrap();
    let events = all(&real);
    let mut kinds: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for e in &events {
        let ev = Event::decode(e.payload.as_slice()).unwrap();
        let k = format!("{:?}", ev.body.as_ref().unwrap())
            .split('(')
            .next()
            .unwrap()
            .to_string();
        let c = kinds.entry(k).or_default();
        c.0 += 1;
        c.1 += e.payload.len();
    }
    let mut by: Vec<_> = kinds.into_iter().collect();
    by.sort_by_key(|a| std::cmp::Reverse(a.1 .0));
    println!(
        "MEASURE sample: {} events, {} payload bytes",
        events.len(),
        events.iter().map(|e| e.payload.len()).sum::<usize>()
    );
    for (k, (n, b)) in by.iter().take(12) {
        println!("MEASURE   {k:<24} {n:>7} events {:>9} bytes", b);
    }
    let max_seq: BTreeMap<&str, u64> = events.iter().fold(BTreeMap::new(), |mut m, e| {
        let v = m.entry(e.producer.as_str()).or_insert(0);
        *v = (*v).max(e.seq);
        m
    });
    let dir = std::env::temp_dir().join(format!("agora-r4-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for copies in [1u64, 10, 50, 100] {
        let file = dir.join(format!("hub-{copies}.db"));
        {
            let store = SqliteStore::open(&file).unwrap();
            for k in 0..copies {
                for e in &events {
                    let mut ev = Event::decode(e.payload.as_slice()).unwrap();
                    let seq = e.seq + k * max_seq[e.producer.as_str()];
                    ev.seq = seq;
                    store
                        .append(&e.producer, seq, e.at_ms, &ev.encode_to_vec())
                        .unwrap();
                }
            }
        }
        let n = events.len() as u64 * copies;
        let t = Instant::now();
        let (view, _) = View::rebuild(&SqliteStore::open(&file).unwrap()).unwrap();
        let rebuild = t.elapsed();
        let t = Instant::now();
        let _hub = Hub::open(SqliteStore::open(&file).unwrap()).unwrap();
        let open = t.elapsed();
        println!(
            "MEASURE {n:>9} events  db {:>6.1} MiB  rebuild {:>8.1} ms  Hub::open {:>8.1} ms  ({} agents, {} nodes in view)",
            size(&file) as f64 / 1048576.0,
            rebuild.as_secs_f64() * 1000.0,
            open.as_secs_f64() * 1000.0,
            view.agents.len(),
            view.nodes.len()
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore]
fn usage_report_rate_and_coalescing() {
    use agora_protocol::v1::event::Body;
    let src =
        std::path::PathBuf::from(std::env::var("AGORA_MEASURE_DB").expect("AGORA_MEASURE_DB"));
    let events = all(&SqliteStore::open(&src).unwrap());
    let usage: Vec<(String, String, u64, u64, u64)> = events
        .iter()
        .filter_map(|e| {
            let ev = Event::decode(e.payload.as_slice()).ok()?;
            match ev.body? {
                Body::AgentUsageReported(u) => Some((
                    format!("{}/{}", ev.producer_id, u.agent_id),
                    u.model,
                    ev.at_ms,
                    u.input_tokens,
                    u.output_tokens,
                )),
                _ => None,
            }
        })
        .collect();
    let (first, last) = (usage.first().unwrap().2, usage.last().unwrap().2);
    let hours = (last - first) as f64 / 3_600_000.0;
    let agents: std::collections::BTreeSet<_> = usage.iter().map(|u| u.0.clone()).collect();
    let zero = usage.iter().filter(|u| u.3 == 0 && u.4 == 0).count();
    println!("MEASURE {} usage events over {hours:.1} h from {} agents ({:.0}/h); {} carry no tokens at all", usage.len(), agents.len(), usage.len() as f64 / hours, zero);
    let mut per: BTreeMap<&str, usize> = BTreeMap::new();
    for u in &usage {
        *per.entry(u.0.as_str()).or_default() += 1;
    }
    let mut top: Vec<_> = per.into_iter().collect();
    top.sort_by_key(|a| std::cmp::Reverse(a.1));
    for (a, n) in top.iter().take(5) {
        println!("MEASURE   {a:<28} {n}");
    }
    for window in [5_000u64, 30_000, 60_000] {
        // One event per (agent, model) per window: a sum of the deltas, sent when the window closes.
        let mut open: BTreeMap<(String, String), u64> = BTreeMap::new();
        let mut sent = 0usize;
        for u in &usage {
            let k = (u.0.clone(), u.1.clone());
            match open.get(&k) {
                Some(start) if u.2 < start + window => {}
                _ => {
                    sent += 1;
                    open.insert(k, u.2);
                }
            }
        }
        println!(
            "MEASURE window {:>3} s: {sent} events ({:.1}% of now)",
            window / 1000,
            sent as f64 * 100.0 / usage.len() as f64
        );
    }
}
