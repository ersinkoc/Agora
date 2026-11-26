//! The hub's price table on disk, and keeping it current (ADR-0021, ADR-0033).
//!
//! The table is the models.dev catalog: a copy refreshed into `<hub dir>/prices-catalog.json`, else the snapshot bundled with the
//! release. The owner's overrides in `<hub dir>/prices.json` (`{ "<model>": { input, output, cache_read, cache_write } }`, USD per
//! million tokens) sit on top and win. `agora hub prices set|unset` edits that file; `agora hub prices update` fetches the catalog
//! once.
//!
//! ADR-0021 has the hub fetch the catalog periodically; ADR-0033 keeps a hub that never talks to the internet that way unless the
//! owner asks. So the periodic fetch is off until the owner turns it on (`agora hub prices auto on`, which writes
//! `<hub dir>/prices-auto`), and then runs about once a day, at a time spread by chance, sooner again after a failure with a growing
//! wait. A fetch is one bounded HTTPS `GET` (`agora_hub::deliver::get`: no redirect followed, a size cap here and a time cap there),
//! and an answer is kept only when it is a catalog with prices, written beside the old file and renamed over it, so a bad answer or
//! an interrupted write leaves the table that was in use. An automatic fetch also refuses a catalog with fewer than half the prices
//! of the one it fetched before (a truncated or emptied answer; this guard is ours, not the ADR's).
//!
//! A running hub watches the three files and takes a change within [`POLL`], so neither an override nor a manual update needs a
//! restart.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use agora_hub::deliver::Url;
use agora_hub::hub::Hub;
use agora_hub::usage::Catalog;
use serde_json::{Map, Value};

/// Where the catalog comes from unless the owner names another address.
pub(crate) const CATALOG_URL: &str = "https://models.dev/api.json";
pub(crate) const CATALOG_FILE: &str = "prices-catalog.json";
pub(crate) const OVERRIDES_FILE: &str = "prices.json";
/// Present while the periodic fetch is on; its first line is the address (empty for [`CATALOG_URL`]).
pub(crate) const AUTO_FILE: &str = "prices-auto";
/// The last automatic fetch: when, and how it went (an error code, never text).
pub(crate) const STATUS_FILE: &str = "prices-status";
/// The largest catalog answer kept (the full models.dev catalog is a few MB).
pub(crate) const MAX_CATALOG_BYTES: usize = 64 * 1024 * 1024;
/// The most override entries, the largest override file read, the longest model id and the highest price (USD per million tokens).
pub(crate) const MAX_OVERRIDES: usize = 1024;
const MAX_OVERRIDES_BYTES: u64 = 1024 * 1024;
pub(crate) const MAX_MODEL_ID: usize = 128;
const MAX_PRICE_USD: f64 = 1_000_000.0;
/// How often a running hub looks at its price files.
pub(crate) const POLL: Duration = Duration::from_secs(5);

/// The price table as the hub uses it.
pub(crate) struct Loaded {
    pub catalog: Catalog,
    /// When the catalog was refreshed, ms since the epoch; 0 for the bundled snapshot.
    pub at_ms: u64,
    /// Override entries that could not be read.
    pub skipped: usize,
}

fn ms(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// The refreshed catalog if there is a usable one, else the bundled snapshot; the owner's overrides on top.
pub(crate) fn load(dir: &Path) -> Loaded {
    let file = dir.join(CATALOG_FILE);
    let refreshed = std::fs::read_to_string(&file)
        .ok()
        .and_then(|t| Catalog::from_models_dev(&t).ok())
        .filter(|c| !c.is_empty());
    let at_ms = refreshed
        .as_ref()
        .and_then(|_| std::fs::metadata(&file).ok())
        .and_then(|m| m.modified().ok())
        .map_or(0, ms);
    let mut catalog = refreshed.unwrap_or_else(|| (*Catalog::bundled()).clone());
    let skipped = read_overrides_text(dir)
        .and_then(|t| catalog.load_overrides(&t).ok())
        .unwrap_or(0);
    Loaded {
        catalog,
        at_ms,
        skipped,
    }
}

fn read_overrides_text(dir: &Path) -> Option<String> {
    let path = dir.join(OVERRIDES_FILE);
    let len = std::fs::metadata(&path).ok()?.len();
    if len > MAX_OVERRIDES_BYTES {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// Why a fetch kept nothing. Each has a stable code; the words are in `locales/`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FetchError {
    /// Not an `http://` or `https://` address.
    Address,
    /// No answer (the system's reason, for the person).
    Unreachable(String),
    /// An answer other than 200 (a redirect is one: it is not followed).
    Status(u16),
    /// Not a models.dev catalog with at least one price, or over [`MAX_CATALOG_BYTES`].
    NotCatalog,
    /// Fewer prices than `floor` (an automatic fetch only).
    Shrunk { got: usize, floor: usize },
    /// The answer could not be written to the hub's folder.
    Store(String),
}

impl FetchError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Address => "prices.address",
            Self::Unreachable(_) => "prices.unreachable",
            Self::Status(_) => "prices.status",
            Self::NotCatalog => "prices.not_catalog",
            Self::Shrunk { .. } => "prices.shrunk",
            Self::Store(_) => "prices.store",
        }
    }
}

/// One bounded `GET`: the status and the body (at most the size given).
pub(crate) type Getter = dyn Fn(&Url, usize) -> std::io::Result<(u16, Vec<u8>)> + Send + Sync;

/// Fetches the catalog at `url` and, when it is a catalog with at least `max(floor, 1)` prices, keeps it as the hub's
/// (`prices-catalog.json`, owner-only, replaced whole). Returns how many models have a price.
pub(crate) fn update(
    dir: &Path,
    url: &str,
    floor: usize,
    get: &Getter,
) -> Result<usize, FetchError> {
    let url = Url::parse(url).ok_or(FetchError::Address)?;
    let (status, body) = match get(&url, MAX_CATALOG_BYTES) {
        Ok(answer) => answer,
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
            return Err(FetchError::NotCatalog)
        }
        Err(e) => return Err(FetchError::Unreachable(e.to_string())),
    };
    if status != 200 {
        return Err(FetchError::Status(status));
    }
    let text = String::from_utf8(body).map_err(|_| FetchError::NotCatalog)?;
    let got = Catalog::from_models_dev(&text)
        .map_err(|_| FetchError::NotCatalog)?
        .len();
    if got == 0 {
        return Err(FetchError::NotCatalog);
    }
    if got < floor {
        return Err(FetchError::Shrunk { got, floor });
    }
    std::fs::create_dir_all(dir).map_err(|e| FetchError::Store(e.to_string()))?;
    agora_platform::write_private(&dir.join(CATALOG_FILE), text.as_bytes())
        .map_err(|e| FetchError::Store(e.to_string()))?;
    Ok(got)
}

// ----- the owner's overrides -----

/// Why an override was not changed.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum OverrideError {
    /// `prices.json` exists and is not a JSON object (it is left as it is).
    Unreadable,
    /// The model id is empty, too long, or has a space or a control character.
    BadModel,
    /// A price is not a number from 0 to [`MAX_PRICE_USD`].
    BadPrice,
    /// The file already holds [`MAX_OVERRIDES`] entries.
    TooMany,
    Store(String),
}

pub(crate) fn model_id_ok(model: &str) -> bool {
    !model.is_empty()
        && model.len() <= MAX_MODEL_ID
        && model.chars().all(|c| !c.is_control() && !c.is_whitespace())
}

/// A price as the owner typed it, in USD per million tokens.
pub(crate) fn parse_price(s: &str) -> Option<f64> {
    let v: f64 = s.trim().parse().ok()?;
    (v.is_finite() && (0.0..=MAX_PRICE_USD).contains(&v)).then_some(v)
}

fn read_overrides(dir: &Path) -> Result<Map<String, Value>, OverrideError> {
    let path = dir.join(OVERRIDES_FILE);
    match std::fs::metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(_) => return Err(OverrideError::Unreadable),
        Ok(m) if m.len() > MAX_OVERRIDES_BYTES => return Err(OverrideError::Unreadable),
        Ok(_) => {}
    }
    let text = std::fs::read_to_string(&path).map_err(|_| OverrideError::Unreadable)?;
    match serde_json::from_str(&text) {
        Ok(Value::Object(m)) => Ok(m),
        _ => Err(OverrideError::Unreadable),
    }
}

fn write_overrides(dir: &Path, m: &Map<String, Value>) -> Result<(), OverrideError> {
    let text = serde_json::to_string_pretty(m).map_err(|e| OverrideError::Store(e.to_string()))?;
    std::fs::create_dir_all(dir).map_err(|e| OverrideError::Store(e.to_string()))?;
    agora_platform::write_private(&dir.join(OVERRIDES_FILE), format!("{text}\n").as_bytes())
        .map_err(|e| OverrideError::Store(e.to_string()))
}

/// Sets the owner's price for `model` (`[input, output, cache_read, cache_write]`, the last two 0 when not given), replacing one
/// it had. Other entries are kept as they are, also ones the hub cannot read.
pub(crate) fn set_override(dir: &Path, model: &str, usd: [f64; 4]) -> Result<(), OverrideError> {
    if !model_id_ok(model) {
        return Err(OverrideError::BadModel);
    }
    if usd
        .iter()
        .any(|v| !v.is_finite() || !(0.0..=MAX_PRICE_USD).contains(v))
    {
        return Err(OverrideError::BadPrice);
    }
    let mut m = read_overrides(dir)?;
    if !m.contains_key(model) && m.len() >= MAX_OVERRIDES {
        return Err(OverrideError::TooMany);
    }
    let mut entry = Map::new();
    for (k, v) in ["input", "output", "cache_read", "cache_write"]
        .into_iter()
        .zip(usd)
    {
        let n = serde_json::Number::from_f64(v).ok_or(OverrideError::BadPrice)?;
        entry.insert(k.to_string(), Value::Number(n));
    }
    m.insert(model.to_string(), Value::Object(entry));
    write_overrides(dir, &m)
}

/// Removes the owner's price for `model`; `false` when there was none.
pub(crate) fn unset_override(dir: &Path, model: &str) -> Result<bool, OverrideError> {
    let mut m = read_overrides(dir)?;
    if m.remove(model).is_none() {
        return Ok(false);
    }
    write_overrides(dir, &m)?;
    Ok(true)
}

/// One override as the owner wrote it: the model, and its four prices (`None` when the entry cannot be read).
pub(crate) type OverrideEntry = (String, Option<[f64; 4]>);

/// The owner's overrides, in model order: the model and its four prices in USD per million tokens (`None` for an entry the hub
/// cannot read).
pub(crate) fn overrides(dir: &Path) -> Result<Vec<OverrideEntry>, OverrideError> {
    let m = read_overrides(dir)?;
    Ok(m.into_iter()
        .map(|(model, v)| {
            let num = |k: &str| {
                v.get(k)
                    .and_then(Value::as_f64)
                    .filter(|p| p.is_finite() && *p >= 0.0)
            };
            let prices = match (num("input"), num("output")) {
                (Some(i), Some(o)) => Some([
                    i,
                    o,
                    num("cache_read").unwrap_or(0.0),
                    num("cache_write").unwrap_or(0.0),
                ]),
                _ => None,
            };
            (model, prices)
        })
        .collect())
}

// ----- the periodic fetch -----

/// The address the periodic fetch uses, or `None` while it is off.
pub(crate) fn auto_url(dir: &Path) -> Option<String> {
    let path = dir.join(AUTO_FILE);
    if std::fs::metadata(&path).ok()?.len() > 4096 {
        return Some(String::new());
    }
    let text = std::fs::read_to_string(path).ok()?;
    let line = text.lines().next().unwrap_or("").trim();
    Some(if line.is_empty() {
        CATALOG_URL.to_string()
    } else {
        line.to_string()
    })
}

/// Turns the periodic fetch on (with the address to use, `None` for [`CATALOG_URL`]) or off.
pub(crate) fn set_auto(dir: &Path, on: Option<Option<&str>>) -> std::io::Result<()> {
    let path = dir.join(AUTO_FILE);
    match on {
        Some(url) => {
            std::fs::create_dir_all(dir)?;
            agora_platform::write_private(&path, format!("{}\n", url.unwrap_or("")).as_bytes())
        }
        None => match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
    }
}

/// When the next automatic fetch is due. Pure: `r` is a random number, so hubs started together do not all ask at once.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Schedule {
    /// Between two successful fetches, give or take a tenth.
    pub every: Duration,
    /// The wait after a first failure; it doubles with each failure after, up to `retry_max`, plus up to a quarter by chance.
    pub retry: Duration,
    pub retry_max: Duration,
    /// The longest wait at start when the catalog is missing or older than `every`.
    pub first_max: Duration,
}

impl Default for Schedule {
    fn default() -> Self {
        Self {
            every: Duration::from_secs(24 * 60 * 60),
            retry: Duration::from_secs(15 * 60),
            retry_max: Duration::from_secs(6 * 60 * 60),
            first_max: Duration::from_secs(5 * 60),
        }
    }
}

/// Up to `max`, by `r`.
fn spread(r: u64, max: Duration) -> Duration {
    let max_ms = u64::try_from(max.as_millis()).unwrap_or(u64::MAX);
    if max_ms == 0 {
        Duration::ZERO
    } else {
        Duration::from_millis(r % max_ms)
    }
}

impl Schedule {
    /// The first wait once the fetch is on: what is left of `every` since the catalog was last refreshed (`age`), else a short
    /// wait.
    pub(crate) fn first(&self, age: Option<Duration>, r: u64) -> Duration {
        match age {
            Some(a) if a < self.every => (self.every - a) + spread(r, self.every / 10),
            _ => spread(r, self.first_max),
        }
    }

    pub(crate) fn after_success(&self, r: u64) -> Duration {
        self.every - self.every / 10 + spread(r, self.every / 5)
    }

    /// After the `failures`-th failure in a row (1 for the first).
    pub(crate) fn after_failure(&self, failures: u32, r: u64) -> Duration {
        let doubled = self
            .retry
            .checked_mul(1u32 << failures.saturating_sub(1).min(16))
            .unwrap_or(self.retry_max);
        let base = doubled.min(self.retry_max);
        base + spread(r, base / 4)
    }
}

fn random() -> u64 {
    let mut b = [0u8; 8];
    if getrandom::getrandom(&mut b).is_err() {
        // No system randomness: the clock's nanoseconds spread hubs well enough for a schedule.
        return u64::from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.subsec_nanos()),
        );
    }
    u64::from_le_bytes(b)
}

/// What a file looks like from outside: its length and modification time, or nothing when it is missing.
type Stamp = Option<(u64, Option<SystemTime>)>;

fn stamp(p: &Path) -> Stamp {
    std::fs::metadata(p)
        .ok()
        .map(|m| (m.len(), m.modified().ok()))
}

/// The hub's side: takes changes of the price files, and fetches the catalog when the periodic fetch is on and due.
pub(crate) struct Feed {
    dir: PathBuf,
    hub: Arc<Hub>,
    schedule: Schedule,
    get: Arc<Getter>,
    seen: (Stamp, Stamp),
    url: Option<String>,
    due: Option<Instant>,
    failures: u32,
}

impl Feed {
    pub(crate) fn new(dir: PathBuf, hub: Arc<Hub>, schedule: Schedule, get: Arc<Getter>) -> Self {
        let seen = (
            stamp(&dir.join(CATALOG_FILE)),
            stamp(&dir.join(OVERRIDES_FILE)),
        );
        Self {
            dir,
            hub,
            schedule,
            get,
            seen,
            url: None,
            due: None,
            failures: 0,
        }
    }

    /// When the next automatic fetch is due (`None` while it is off).
    #[cfg(test)]
    pub(crate) fn due(&self) -> Option<Instant> {
        self.due
    }

    /// One look: the periodic fetch first (when on and due), then the files; a change is handed to the hub whole.
    pub(crate) fn tick(&mut self, now: Instant) {
        let url = auto_url(&self.dir);
        if url != self.url {
            self.due = url.as_ref().map(|_| {
                let age = std::fs::metadata(self.dir.join(CATALOG_FILE))
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| SystemTime::now().duration_since(t).ok());
                now + self.schedule.first(age, random())
            });
            self.url = url;
            self.failures = 0;
        }
        if let (Some(url), Some(due)) = (self.url.clone(), self.due) {
            if now >= due {
                // Half of a catalog fetched before; none while the bundled table is in use, so an owner's smaller mirror is
                // taken the first time as `update --url` takes it.
                let in_use = load(&self.dir);
                let floor = if in_use.at_ms == 0 {
                    0
                } else {
                    in_use.catalog.len() / 2
                };
                let result = update(&self.dir, &url, floor, &*self.get);
                let wait = match &result {
                    Ok(_) => {
                        self.failures = 0;
                        self.schedule.after_success(random())
                    }
                    Err(_) => {
                        self.failures = self.failures.saturating_add(1);
                        self.schedule.after_failure(self.failures, random())
                    }
                };
                self.due = Some(now + wait);
                let next_ms = ms(SystemTime::now() + wait);
                let _ = write_status(&self.dir, &result, next_ms);
            }
        }
        let seen = (
            stamp(&self.dir.join(CATALOG_FILE)),
            stamp(&self.dir.join(OVERRIDES_FILE)),
        );
        if seen != self.seen {
            self.seen = seen;
            let l = load(&self.dir);
            self.hub.set_prices(l.catalog, l.at_ms);
        }
    }
}

/// The last automatic fetch, for `agora hub prices show`: `{at_ms, ok, models | error, status, next_ms}`.
fn write_status(
    dir: &Path,
    result: &Result<usize, FetchError>,
    next_ms: u64,
) -> std::io::Result<()> {
    let mut v = serde_json::json!({ "at_ms": ms(SystemTime::now()), "next_ms": next_ms });
    match result {
        Ok(n) => {
            v["ok"] = true.into();
            v["models"] = (*n as u64).into();
        }
        Err(e) => {
            v["ok"] = false.into();
            v["error"] = e.code().into();
            if let FetchError::Status(s) = e {
                v["status"] = (*s).into();
            }
        }
    }
    agora_platform::write_private(&dir.join(STATUS_FILE), v.to_string().as_bytes())
}

/// The last automatic fetch as written by [`write_status`].
pub(crate) fn read_status(dir: &Path) -> Option<Value> {
    let path = dir.join(STATUS_FILE);
    if std::fs::metadata(&path).ok()?.len() > 4096 {
        return None;
    }
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Runs the hub's [`Feed`] on a thread of its own for as long as the hub runs.
pub(crate) fn spawn(dir: PathBuf, hub: Arc<Hub>) {
    let get: Arc<Getter> = Arc::new(|url: &Url, max: usize| agora_hub::deliver::get(url, max));
    let mut feed = Feed::new(dir, hub, Schedule::default(), get);
    let _ = std::thread::Builder::new()
        .name("agora-prices".into())
        .spawn(move || loop {
            feed.tick(Instant::now());
            std::thread::sleep(POLL);
        });
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    const ONE: &str = r#"{"p":{"models":{"m":{"cost":{"input":1,"output":2}}}}}"#;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("agora-pricefeed-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn catalog(n: usize) -> String {
        let models: Vec<String> = (0..n)
            .map(|i| format!(r#""m{i}":{{"cost":{{"input":1,"output":2}}}}"#))
            .collect();
        format!(r#"{{"p":{{"models":{{{}}}}}}}"#, models.join(","))
    }

    /// A stand-in for the network: answers each `GET` with the next of `answers` and records the addresses asked.
    fn server(
        answers: Vec<std::io::Result<(u16, String)>>,
    ) -> (Arc<Getter>, Arc<Mutex<Vec<String>>>) {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let left = Mutex::new(answers.into_iter());
        let seen = Arc::clone(&asked);
        let get: Arc<Getter> = Arc::new(move |u: &Url, _max: usize| {
            seen.lock()
                .unwrap()
                .push(format!("{}:{}{}", u.host, u.port, u.path));
            match left.lock().unwrap().next() {
                Some(Ok((s, b))) => Ok((s, b.into_bytes())),
                Some(Err(e)) => Err(e),
                None => Err(std::io::ErrorKind::ConnectionRefused.into()),
            }
        });
        (get, asked)
    }

    #[test]
    fn an_update_keeps_only_a_catalog_with_prices_and_a_bad_answer_changes_nothing() {
        let d = tmp("update");
        let (get, asked) = server(vec![
            Ok((301, String::new())),
            Ok((200, "not json".into())),
            Ok((200, r#"{"p":{"models":{}}}"#.into())),
            Err(std::io::ErrorKind::InvalidData.into()),
            Err(std::io::ErrorKind::TimedOut.into()),
            Ok((200, ONE.into())),
            Ok((200, catalog(1))),
        ]);
        let u = "https://cat.example/api.json";
        assert_eq!(
            update(&d, u, 0, &*get),
            Err(FetchError::Status(301)),
            "a redirect is not followed"
        );
        assert_eq!(update(&d, u, 0, &*get), Err(FetchError::NotCatalog));
        assert_eq!(update(&d, u, 0, &*get), Err(FetchError::NotCatalog));
        assert_eq!(
            update(&d, u, 0, &*get),
            Err(FetchError::NotCatalog),
            "over the size cap"
        );
        assert_eq!(
            update(&d, u, 0, &*get).unwrap_err().code(),
            "prices.unreachable"
        );
        assert!(
            !d.join(CATALOG_FILE).exists(),
            "nothing kept from a bad answer"
        );
        assert_eq!(update(&d, u, 0, &*get), Ok(1));
        assert_eq!(std::fs::read_to_string(d.join(CATALOG_FILE)).unwrap(), ONE);
        // A shrunk catalog is refused when a floor is given, and the kept one stays.
        assert_eq!(
            update(&d, u, 2, &*get),
            Err(FetchError::Shrunk { got: 1, floor: 2 })
        );
        assert_eq!(std::fs::read_to_string(d.join(CATALOG_FILE)).unwrap(), ONE);
        assert_eq!(update(&d, "ftp://x/y", 0, &*get), Err(FetchError::Address));
        assert_eq!(
            asked.lock().unwrap().len(),
            7,
            "a refused address is never asked"
        );
        assert_eq!(asked.lock().unwrap()[0], "cat.example:443/api.json");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(d.join(CATALOG_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0, "owner-only");
        }
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn overrides_are_set_replaced_and_removed_and_other_entries_are_kept() {
        let d = tmp("overrides");
        std::fs::write(d.join(OVERRIDES_FILE), r#"{"kept":{"input":"x"}}"#).unwrap();
        set_override(&d, "p/m", [2.5, 10.0, 0.25, 0.0]).unwrap();
        set_override(&d, "p/m", [3.0, 12.0, 0.0, 0.0]).unwrap();
        let all = overrides(&d).unwrap();
        assert_eq!(
            all,
            vec![
                ("kept".to_string(), None),
                ("p/m".to_string(), Some([3.0, 12.0, 0.0, 0.0]))
            ]
        );
        // The hub reads what the command wrote, and the override wins over the catalog.
        std::fs::write(d.join(CATALOG_FILE), ONE).unwrap();
        set_override(&d, "p/m", [5.0, 6.0, 0.0, 0.0]).unwrap();
        let l = load(&d);
        assert_eq!(l.skipped, 1, "the unreadable entry is counted, not guessed");
        assert_eq!(l.catalog.price("p/m").unwrap().input, 5_000_000);
        assert_eq!(
            l.catalog.price("m").unwrap().input,
            5_000_000,
            "also by the bare id"
        );
        assert!(unset_override(&d, "p/m").unwrap());
        assert!(!unset_override(&d, "p/m").unwrap());
        assert_eq!(load(&d).catalog.price("p/m").unwrap().input, 1_000_000);
        // Bounds.
        for bad in ["", "a b", "a\nb", &"x".repeat(MAX_MODEL_ID + 1)] {
            assert_eq!(
                set_override(&d, bad, [1.0; 4]),
                Err(OverrideError::BadModel),
                "{bad:?}"
            );
        }
        for p in [-1.0, f64::NAN, f64::INFINITY, MAX_PRICE_USD * 2.0] {
            assert_eq!(
                set_override(&d, "p/x", [p, 1.0, 0.0, 0.0]),
                Err(OverrideError::BadPrice)
            );
        }
        assert_eq!(parse_price("2.5"), Some(2.5));
        assert_eq!(parse_price("-1"), None);
        assert_eq!(parse_price("abc"), None);
        // A file that is not an object is never overwritten.
        std::fs::write(d.join(OVERRIDES_FILE), "[1]").unwrap();
        assert_eq!(
            set_override(&d, "p/m", [1.0; 4]),
            Err(OverrideError::Unreadable)
        );
        assert_eq!(
            std::fs::read_to_string(d.join(OVERRIDES_FILE)).unwrap(),
            "[1]"
        );
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_override_file_holds_a_bounded_number_of_entries() {
        let d = tmp("many");
        let mut m = Map::new();
        for i in 0..MAX_OVERRIDES {
            m.insert(
                format!("p/m{i}"),
                serde_json::json!({"input": 1, "output": 1}),
            );
        }
        write_overrides(&d, &m).unwrap();
        assert_eq!(
            set_override(&d, "p/new", [1.0; 4]),
            Err(OverrideError::TooMany)
        );
        // Replacing one that is there is still allowed.
        set_override(&d, "p/m0", [2.0; 4]).unwrap();
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_schedule_spreads_waits_and_backs_off_after_failures() {
        let s = Schedule::default();
        let day = s.every;
        for r in [0, 1, 12345, u64::MAX] {
            let ok = s.after_success(r);
            assert!(ok >= day - day / 10 && ok < day + day / 10, "{ok:?}");
            assert!(s.first(None, r) < s.first_max);
            // A catalog refreshed an hour ago waits for the rest of the day.
            let rest = s.first(Some(Duration::from_secs(3600)), r);
            assert!(
                rest >= day - Duration::from_secs(3600)
                    && rest < day - Duration::from_secs(3600) + day / 10
            );
            assert!(
                s.first(Some(day * 2), r) < s.first_max,
                "a stale catalog is fetched soon"
            );
        }
        let fails: Vec<Duration> = (1..=8).map(|n| s.after_failure(n, 0)).collect();
        assert_eq!(fails[0], s.retry);
        assert_eq!(fails[1], s.retry * 2);
        assert!(fails.windows(2).all(|w| w[0] <= w[1]));
        assert_eq!(*fails.last().unwrap(), s.retry_max);
        assert!(s.after_failure(u32::MAX, u64::MAX) < s.retry_max + s.retry_max / 4);
        // Chance changes the wait, within its bound.
        assert_ne!(s.after_success(1), s.after_success(1_000_000));
    }

    fn quick() -> Schedule {
        Schedule {
            every: Duration::from_secs(1000),
            retry: Duration::from_secs(10),
            retry_max: Duration::from_secs(40),
            first_max: Duration::from_millis(1),
        }
    }

    fn hub() -> Arc<Hub> {
        let store = agora_hub::store::SqliteStore::open_in_memory().unwrap();
        Arc::new(Hub::open(store).unwrap())
    }

    #[test]
    fn the_periodic_fetch_is_off_until_the_owner_turns_it_on_then_keeps_to_its_schedule() {
        let d = tmp("feed");
        let calls = Arc::new(AtomicUsize::new(0));
        let n = Arc::clone(&calls);
        let full = catalog(10);
        let get: Arc<Getter> = Arc::new(move |_: &Url, _| {
            let k = n.fetch_add(1, Ordering::SeqCst);
            // The first answer fails, the second is a catalog, the third a truncated one (1 of 10).
            match k {
                0 => Ok((503, Vec::new())),
                1 => Ok((200, full.clone().into_bytes())),
                _ => Ok((200, ONE.as_bytes().to_vec())),
            }
        });
        let h = hub();
        let mut f = Feed::new(d.clone(), Arc::clone(&h), quick(), get);
        let t0 = Instant::now();
        // Off: nothing is asked, however long the hub runs.
        for k in 0..5 {
            f.tick(t0 + Duration::from_secs(k * 10_000));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(f.due(), None);
        // On: the first fetch is soon (no catalog yet); it fails and is tried again after the retry wait, not before.
        set_auto(&d, Some(Some("http://127.0.0.1:9/api.json"))).unwrap();
        f.tick(t0);
        // The first wait is at most `first_max` (1 ms here), so the fetch is in this tick or at the time it names.
        let first = if calls.load(Ordering::SeqCst) == 0 {
            let at = f.due().unwrap();
            assert!(at <= t0 + Duration::from_millis(1));
            f.tick(at);
            at
        } else {
            t0
        };
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(read_status(&d).unwrap()["error"], "prices.status");
        let retry = f.due().unwrap();
        assert!(
            retry >= first + Duration::from_secs(10) && retry < first + Duration::from_secs(13)
        );
        f.tick(retry - Duration::from_millis(1));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        f.tick(retry);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(read_status(&d).unwrap()["ok"], true);
        // The hub took the new table at once.
        assert!(h.usage_report().prices_at_ms > 0);
        let next = f.due().unwrap();
        assert!(
            next >= retry + Duration::from_secs(900),
            "about a day (here 1000 s) later"
        );
        let kept = std::fs::read_to_string(d.join(CATALOG_FILE)).unwrap();
        // A truncated answer next time is refused, and the catalog fetched before stays.
        f.tick(next);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert_eq!(read_status(&d).unwrap()["error"], "prices.shrunk");
        assert_eq!(std::fs::read_to_string(d.join(CATALOG_FILE)).unwrap(), kept);
        let again = f.due().unwrap();
        assert!(
            again < next + Duration::from_secs(13),
            "a failure is tried again soon"
        );
        // Off again: no more fetches.
        set_auto(&d, None).unwrap();
        f.tick(again + Duration::from_secs(10_000));
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert_eq!(f.due(), None);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_running_hub_takes_a_new_override_without_a_restart() {
        let d = tmp("reload");
        let (get, asked) = server(vec![]);
        let h = hub();
        std::fs::write(d.join(CATALOG_FILE), ONE).unwrap();
        let mut f = Feed::new(d.clone(), Arc::clone(&h), quick(), get);
        // Negative control: a tick with nothing changed hands the hub nothing (it still has the bundled table).
        f.tick(Instant::now());
        assert_eq!(h.usage_report().prices_at_ms, 0);
        set_override(&d, "p/m", [9.0, 9.0, 0.0, 0.0]).unwrap();
        f.tick(Instant::now());
        assert!(
            h.usage_report().prices_at_ms > 0,
            "the refreshed catalog and the override are in use"
        );
        assert!(
            asked.lock().unwrap().is_empty(),
            "no fetch while the periodic fetch is off"
        );
        let _ = std::fs::remove_dir_all(d);
    }
}
