//! A pulled price catalog: USD per million tokens, cache reads and writes
//! included, for every model the community keeps at models.dev. It prices
//! a model the shipped tables never heard of, and it is fetched at most once
//! a day, on a thread of its own, into `prices.json` under the Ryter home.
//! Nothing here runs on a request's path: [`crate::spend::PriceBook::load`]
//! reads the file.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::spend::Rates;
use crate::{Error, Result};

/// Where the catalog comes from.
pub const SOURCE: &str = "https://models.dev/api.json";
/// Refusing a body past this: the catalog is a few megabytes.
const MAX_BYTES: u64 = 64 * 1024 * 1024;
/// A pull a day.
const EVERY_SECS: u64 = 24 * 60 * 60;
const TIMEOUT: Duration = Duration::from_secs(60);

/// The file as written: the source's rows trimmed to their prices.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Catalog {
    /// Unix seconds of the pull.
    pub fetched: u64,
    /// Where it came from.
    #[serde(default)]
    pub source: String,
    /// Provider id → model id → prices.
    #[serde(default)]
    pub providers: BTreeMap<String, BTreeMap<String, Row>>,
}

/// One model's prices, USD per million tokens.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub input: Option<f64>,
    pub output: Option<f64>,
    pub cache_read: Option<f64>,
    pub cache_write: Option<f64>,
    /// Higher rates for a prompt over `size` tokens.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tiers: Vec<Tier>,
}

/// The rates a prompt over `size` tokens pays.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Tier {
    pub size: u64,
    pub input: Option<f64>,
    pub output: Option<f64>,
    pub cache_read: Option<f64>,
    pub cache_write: Option<f64>,
}

fn priced(v: Option<f64>) -> Option<f64> {
    v.filter(|v| v.is_finite() && *v >= 0.0)
}

impl Row {
    /// The row as rates, when it has an input and an output price. A cache
    /// read the row does not price bills as input; a write it does not
    /// price bills as input too.
    pub fn rates(&self) -> Option<Rates> {
        let input = priced(self.input)?;
        let output = priced(self.output)?;
        let mut rates = Rates::per_million(input, output);
        if let Some(read) = priced(self.cache_read) {
            rates.cached_per_million = read;
        }
        rates.cache_write_per_million = priced(self.cache_write);
        if let Some(t) = self
            .tiers
            .iter()
            .filter(|t| t.size > 0)
            .min_by_key(|t| t.size)
        {
            rates.long_threshold = Some(t.size + 1);
            rates.long_input_per_million = priced(t.input);
            rates.long_cached_per_million = priced(t.cache_read);
            rates.long_output_per_million = priced(t.output);
            rates.long_cache_write_per_million = priced(t.cache_write);
        }
        Some(rates)
    }
}

/// `prices.json` under the Ryter home.
pub fn cache_path(home: &Path) -> PathBuf {
    home.join("prices.json")
}

/// The catalog on disk, if any.
pub fn load(home: &Path) -> Option<Catalog> {
    let text = std::fs::read_to_string(cache_path(home)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Whether a pull is due: never pulled, or a day since the last.
pub fn due(home: &Path, now: u64) -> bool {
    load(home).is_none_or(|c| now.saturating_sub(c.fetched) >= EVERY_SECS)
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The source's JSON trimmed to a [`Catalog`]. models.dev lists
/// `provider → models → model → cost {input, output, cache_read,
/// cache_write, tiers[{tier: {size}, …}]}`; a model without an input and an
/// output price is left out.
pub fn trim(raw: &str, fetched: u64) -> Result<Catalog> {
    let v: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| Error::Provider(format!("price catalog: {e}")))?;
    let Some(providers) = v.as_object() else {
        return Err(Error::Provider("price catalog: not an object".into()));
    };
    let num = |v: &serde_json::Value, k: &str| v.get(k).and_then(serde_json::Value::as_f64);
    let mut catalog = Catalog {
        fetched,
        source: SOURCE.into(),
        providers: BTreeMap::new(),
    };
    for (provider, p) in providers {
        let Some(models) = p.get("models").and_then(serde_json::Value::as_object) else {
            continue;
        };
        let mut rows = BTreeMap::new();
        for (id, m) in models {
            let Some(cost) = m.get("cost") else { continue };
            let (Some(input), Some(output)) = (num(cost, "input"), num(cost, "output")) else {
                continue;
            };
            let tiers = cost
                .get("tiers")
                .and_then(serde_json::Value::as_array)
                .map(|ts| {
                    ts.iter()
                        .filter_map(|t| {
                            let size = t.get("tier")?.get("size")?.as_u64()?;
                            Some(Tier {
                                size,
                                input: num(t, "input"),
                                output: num(t, "output"),
                                cache_read: num(t, "cache_read"),
                                cache_write: num(t, "cache_write"),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            rows.insert(
                id.clone(),
                Row {
                    input: Some(input),
                    output: Some(output),
                    cache_read: num(cost, "cache_read"),
                    cache_write: num(cost, "cache_write"),
                    tiers,
                },
            );
        }
        if !rows.is_empty() {
            catalog.providers.insert(provider.clone(), rows);
        }
    }
    Ok(catalog)
}

/// Write `catalog` as `prices.json`, whole or not at all.
pub fn save(home: &Path, catalog: &Catalog) -> Result<()> {
    std::fs::create_dir_all(home).map_err(|e| Error::Io(e.to_string()))?;
    let path = cache_path(home);
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string(catalog).map_err(|e| Error::Io(e.to_string()))?;
    std::fs::write(&tmp, text).map_err(|e| Error::Io(e.to_string()))?;
    std::fs::rename(&tmp, &path).map_err(|e| Error::Io(e.to_string()))
}

/// Pull `url` and save it. Returns how many models were priced.
pub fn refresh_from(home: &Path, url: &str) -> Result<usize> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(format!("ryter/{}", crate::update::Version::current()))
        .connect_timeout(Duration::from_secs(10))
        .timeout(TIMEOUT)
        .build()
        .map_err(|e| Error::Io(e.to_string()))?;
    let resp = client
        .get(url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|e| Error::Io(format!("{url}: {e}")))?;
    let mut body = Vec::new();
    resp.take(MAX_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|e| Error::Io(format!("{url}: {e}")))?;
    if body.len() as u64 > MAX_BYTES {
        return Err(Error::Io(format!("{url} is larger than expected")));
    }
    let raw = String::from_utf8(body).map_err(|e| Error::Provider(format!("{url}: {e}")))?;
    let catalog = trim(&raw, now())?;
    let n = catalog.providers.values().map(BTreeMap::len).sum();
    save(home, &catalog)?;
    Ok(n)
}

/// Pull the catalog from [`SOURCE`].
pub fn refresh(home: &Path) -> Result<usize> {
    refresh_from(home, SOURCE)
}

/// The launch pull: when `enabled` and due, fetch; `Some(count)` when a new
/// catalog was written. A failed pull is quiet: the shipped tables stand.
pub fn on_launch(home: &Path, enabled: bool) -> Option<usize> {
    if !enabled || !due(home, now()) {
        return None;
    }
    refresh(home).ok()
}

/// `3h ago`, for the doctor.
pub fn age(fetched: u64) -> String {
    let secs = now().saturating_sub(fetched);
    if secs < 60 {
        "just now".into()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "anthropic": {"id": "anthropic", "models": {
        "claude-opus-5-5": {"cost": {"input": 4, "output": 20, "cache_read": 0.2, "cache_write": 5}},
        "claude-haiku-5-5": {"cost": {"input": 0.1, "output": 0.5, "cache_read": 0.01, "cache_write": 0.125,
           "tiers": [{"input": 0.5, "output": 2.5, "cache_read": 0.05, "cache_write": 0.625, "tier": {"type": "context", "size": 100000}}]}},
        "claude-free": {"name": "no cost block"}
      }},
      "xai": {"id": "xai", "models": {
        "grok-4.6": {"cost": {"input": 2, "output": 6, "cache_read": 0.5,
           "tiers": [{"input": 4, "output": 12, "cache_read": 1, "tier": {"type": "context", "size": 200000}}]}}
      }},
      "deepseek": {"id": "deepseek", "models": {"deepseek-chat": {"cost": {"input": 0.28, "output": 0.42, "cache_read": 0.028}}}},
      "other": {"id": "other", "models": {"deepseek-chat": {"cost": {"input": 0.5, "output": 0.9}}}}
    }"#;

    #[test]
    fn the_source_is_trimmed_to_prices_and_tiers() {
        let c = trim(SAMPLE, 7).unwrap();
        assert_eq!(c.fetched, 7);
        assert_eq!(c.providers.len(), 4);
        let a = &c.providers["anthropic"];
        assert!(
            !a.contains_key("claude-free"),
            "a model without prices is left out"
        );
        let r = a["claude-opus-5-5"].rates().unwrap();
        assert_eq!(
            (
                r.input_per_million,
                r.cached_per_million,
                r.cache_write_per_million,
                r.output_per_million
            ),
            (4.0, 0.2, Some(5.0), 20.0)
        );
        let h = a["claude-haiku-5-5"].rates().unwrap();
        assert_eq!(h.long_threshold, Some(100_001));
        assert_eq!(h.long_input_per_million, Some(0.5));
        assert_eq!(h.long_cache_write_per_million, Some(0.625));
        let g = c.providers["xai"]["grok-4.6"].rates().unwrap();
        assert_eq!(g.cache_write_per_million, None);
        assert_eq!(g.long_cached_per_million, Some(1.0));
    }

    #[test]
    fn a_catalog_round_trips_through_its_file_and_knows_its_age() {
        let dir = tempfile::tempdir().unwrap();
        assert!(due(dir.path(), 1_000_000), "never pulled is due");
        let c = trim(SAMPLE, 1_000_000).unwrap();
        save(dir.path(), &c).unwrap();
        let back = load(dir.path()).unwrap();
        assert_eq!(
            back.providers["anthropic"]["claude-opus-5-5"].cache_write,
            Some(5.0)
        );
        assert!(!due(dir.path(), 1_000_000 + EVERY_SECS - 1));
        assert!(due(dir.path(), 1_000_000 + EVERY_SECS));
        assert!(!cache_path(dir.path()).with_extension("json.tmp").exists());
    }

    #[test]
    fn a_bad_price_is_not_a_rate() {
        let row = Row {
            input: Some(-1.0),
            output: Some(1.0),
            ..Row::default()
        };
        assert!(row.rates().is_none());
        let row = Row {
            input: Some(1.0),
            output: Some(1.0),
            cache_read: Some(f64::NAN),
            ..Row::default()
        };
        assert_eq!(
            row.rates().unwrap().cached_per_million,
            1.0,
            "an unusable read rate bills as input"
        );
    }

    #[test]
    fn a_refresh_writes_what_the_server_gave() {
        let dir = tempfile::tempdir().unwrap();
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap();
        let body = SAMPLE.to_string();
        let t = std::thread::spawn(move || {
            let (mut s, _) = server.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = std::io::Read::read(&mut s, &mut buf);
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            std::io::Write::write_all(&mut s, head.as_bytes()).unwrap();
            std::io::Write::write_all(&mut s, body.as_bytes()).unwrap();
        });
        let n = refresh_from(dir.path(), &format!("http://{addr}/api.json")).unwrap();
        t.join().unwrap();
        assert_eq!(n, 5);
        let c = load(dir.path()).unwrap();
        assert_eq!(c.source, SOURCE);
        assert!(c.fetched > 0);
    }
}
