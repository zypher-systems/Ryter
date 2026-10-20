//! Token → USD pricing. Unknown rates yield `None` (`$?.??`), never a fake `$0.00`.

use std::collections::HashMap;

use std::path::Path;

use crate::config::{Config, PriceOverride};

/// Token counts from a provider.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    /// Prompt / input tokens.
    pub input_tokens: u64,
    /// Completion / output tokens.
    pub output_tokens: u64,
    /// Cached input tokens (billed at the cached rate when known).
    pub cached_tokens: u64,
    /// Input tokens written to the provider's cache this call, billed at the
    /// cache-write rate (Anthropic: 1.25× input). Part of `input_tokens`.
    pub cache_write_tokens: u64,
}

impl Usage {
    /// Fold a later usage report into this one. Anthropic reports input at
    /// `message_start` and only output at `message_delta`; keeping just the
    /// last report priced every Anthropic round at zero input tokens.
    /// Counts are cumulative, so the larger of each wins.
    #[must_use]
    pub fn merge(self, later: Self) -> Self {
        Self {
            input_tokens: self.input_tokens.max(later.input_tokens),
            output_tokens: self.output_tokens.max(later.output_tokens),
            cached_tokens: self.cached_tokens.max(later.cached_tokens),
            cache_write_tokens: self.cache_write_tokens.max(later.cache_write_tokens),
        }
    }
}

/// USD per million tokens for one model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rates {
    /// Input.
    pub input_per_million: f64,
    /// Cached input. Falls back to input if unset.
    pub cached_per_million: f64,
    /// Output.
    pub output_per_million: f64,
    /// Input written to the cache. Falls back to input if unset.
    pub cache_write_per_million: Option<f64>,
    /// When prompt tokens reach this, use the long-context rates for the whole request.
    pub long_threshold: Option<u64>,
    /// Long-context input.
    pub long_input_per_million: Option<f64>,
    /// Long-context cached input.
    pub long_cached_per_million: Option<f64>,
    /// Long-context output.
    pub long_output_per_million: Option<f64>,
    /// Long-context cache write. Falls back to long-context input if unset.
    pub long_cache_write_per_million: Option<f64>,
}

impl Rates {
    /// Catalog row with only the two rates the sidebar shows. A cache read
    /// is billed as input: no discount is claimed that the row does not give.
    pub fn per_million(input: f64, output: f64) -> Self {
        Self {
            input_per_million: input,
            cached_per_million: input,
            output_per_million: output,
            cache_write_per_million: None,
            long_threshold: None,
            long_input_per_million: None,
            long_cached_per_million: None,
            long_output_per_million: None,
            long_cache_write_per_million: None,
        }
    }

    /// Whether the row says what a cache read costs, as against assuming
    /// the input rate.
    pub fn knows_cache(self) -> bool {
        self.cached_per_million != self.input_per_million || self.cache_write_per_million.is_some()
    }

    /// `(input, cached, write, output)` per million for this request.
    fn effective(self, usage: Usage) -> (f64, f64, f64, f64) {
        let long = self.long_threshold.is_some_and(|t| usage.input_tokens >= t);
        if long {
            let input = self
                .long_input_per_million
                .unwrap_or(self.input_per_million);
            (
                input,
                self.long_cached_per_million
                    .unwrap_or(self.cached_per_million),
                self.long_cache_write_per_million.unwrap_or(input),
                self.long_output_per_million
                    .unwrap_or(self.output_per_million),
            )
        } else {
            (
                self.input_per_million,
                self.cached_per_million,
                self.cache_write_per_million
                    .unwrap_or(self.input_per_million),
                self.output_per_million,
            )
        }
    }

    /// USD for `usage`. `input_tokens` is the whole prompt: cache reads and
    /// writes are the parts of it billed at their own rates.
    pub fn cost(self, usage: Usage) -> f64 {
        let (input, cached, write, output) = self.effective(usage);
        let plain = usage
            .input_tokens
            .saturating_sub(usage.cached_tokens)
            .saturating_sub(usage.cache_write_tokens);
        (plain as f64) * input / 1_000_000.0
            + (usage.cached_tokens as f64) * cached / 1_000_000.0
            + (usage.cache_write_tokens as f64) * write / 1_000_000.0
            + (usage.output_tokens as f64) * output / 1_000_000.0
    }
}

/// Looks up rates: TOML override → the connection's own model list → the
/// shipped tables → the pulled catalog (`prices.json`, see [`crate::prices`]).
/// The shipped row wins over the pulled one for the same model: it was read
/// by a person against the provider's page.
#[derive(Debug, Clone, Default)]
pub struct PriceBook {
    overrides: HashMap<String, Option<Rates>>,
    catalog: HashMap<String, Rates>,
    pulled: HashMap<String, Rates>,
}

/// The provider a built-in connection kind is listed under in the pulled
/// catalog. Other kinds take any provider's row for the exact id, when the
/// providers that have one agree on it.
pub fn catalog_provider(kind: &str) -> Option<&'static str> {
    match kind {
        "spacexsi" | "spacexai" => Some("xai"),
        "anthropic" => Some("anthropic"),
        "openai" => Some("openai"),
        "openrouter" => Some("openrouter"),
        _ => None,
    }
}

impl PriceBook {
    /// Empty book; still has the shipped tables on lookup.
    pub fn new() -> Self {
        Self::default()
    }

    /// Load TOML `[pricing]` overrides from config.
    pub fn from_config(cfg: &Config) -> Self {
        let mut book = Self::new();
        for (model, o) in &cfg.pricing {
            book.overrides.insert(model.clone(), override_rates(o));
        }
        book
    }

    /// The book for a run: the config's overrides plus the pulled catalog
    /// under `home`, when one has been fetched.
    pub fn load(cfg: &Config, home: &Path) -> Self {
        let mut book = Self::from_config(cfg);
        // `[spend] catalog = false` leaves a file pulled earlier unread too.
        if cfg.spend.catalog {
            if let Ok(text) = std::fs::read_to_string(crate::prices::cache_path(home)) {
                book.ingest_pulled(cfg, &text);
            }
        }
        book
    }

    /// Merge a pulled catalog (the file [`crate::prices::refresh`] writes).
    /// Each configured connection takes the rows of its provider; a kind
    /// with no provider of its own takes a row any providers agree on.
    /// Returns how many rows were taken.
    pub fn ingest_pulled(&mut self, cfg: &Config, json: &str) -> usize {
        let Ok(catalog) = serde_json::from_str::<crate::prices::Catalog>(json) else {
            return 0;
        };
        let mut n = 0;
        let kinds: Vec<&str> = cfg.connections.values().map(|c| c.kind.as_str()).collect();
        let named: Vec<&str> = kinds.iter().filter_map(|k| catalog_provider(k)).collect();
        let open = kinds.iter().any(|k| catalog_provider(k).is_none());
        for provider in &named {
            if let Some(models) = catalog.providers.get(*provider) {
                for (id, row) in models {
                    if let Some(rates) = row.rates() {
                        self.pulled.insert(id.clone(), rates);
                        n += 1;
                    }
                }
            }
        }
        if open {
            let mut seen: HashMap<&str, Option<Rates>> = HashMap::new();
            for (provider, models) in &catalog.providers {
                if named.contains(&provider.as_str()) {
                    continue;
                }
                for (id, row) in models {
                    let Some(rates) = row.rates() else { continue };
                    seen.entry(id)
                        .and_modify(|r| {
                            if *r != Some(rates) {
                                *r = None;
                            }
                        })
                        .or_insert(Some(rates));
                }
            }
            for (id, rates) in seen {
                if let Some(rates) = rates {
                    if !self.pulled.contains_key(id) {
                        self.pulled.insert(id.to_string(), rates);
                        n += 1;
                    }
                }
            }
        }
        n
    }

    /// Rates for `model`, if known.
    pub fn rates(&self, model: &str) -> Option<Rates> {
        if let Some(r) = self.overrides.get(model) {
            return *r;
        }
        if let Some(r) = self.catalog.get(model) {
            return Some(*r);
        }
        self.known(model)
    }

    /// What is known of `model` outside its connection's own list: the
    /// shipped tables, else the pulled catalog.
    fn known(&self, model: &str) -> Option<Rates> {
        shipped_rates(model).or_else(|| self.pulled.get(model).copied())
    }

    /// USD cost, or `None` when the model has no rates (`$?.??`).
    ///
    /// Zero tokens with known rates is `$0.00`. Unknown rates never become `$0.00`.
    pub fn cost(&self, model: &str, usage: Usage) -> Option<f64> {
        self.rates(model)
            .map(|r| r.cost(usage))
            .filter(|v| v.is_finite() && *v >= 0.0)
    }

    /// `$2/M input / $6/M output`, or `$?.??` when unknown.
    pub fn format_model_rates(&self, model: &str) -> String {
        format_rates(self.rates(model))
    }

    /// Fill missing catalog rows from a live `/models` list.
    ///
    /// A list that prices its cache reads (OpenRouter) is taken as it is. One
    /// that gives only input and output keeps the cache rates and tiers the
    /// book already knows for that model; it never turns a known cache
    /// discount back into the input rate.
    pub fn ingest_model_info(&mut self, models: &[crate::llm::ModelInfo]) {
        for m in models {
            let Some(input) = m.input_per_million else {
                continue;
            };
            let Some(output) = m.output_per_million else {
                continue;
            };
            if self.catalog.contains_key(&m.id) {
                continue;
            }
            let row = match m.cached_per_million {
                Some(cached) => Rates {
                    cached_per_million: cached,
                    cache_write_per_million: m.cache_write_per_million,
                    ..Rates::per_million(input, output)
                },
                None => match self.known(&m.id) {
                    Some(known) if known.knows_cache() => known,
                    _ => Rates::per_million(input, output),
                },
            };
            self.catalog.insert(m.id.clone(), row);
        }
    }
}

/// Sidebar price line. Unknown is `$?.??/M`, never a fake `$0`.
pub fn format_rates(rates: Option<Rates>) -> String {
    match rates {
        Some(r) => format!(
            "${}/M input / ${}/M output",
            trim_rate(r.input_per_million),
            trim_rate(r.output_per_million)
        ),
        None => "$?.??/M input / $?.??/M output".into(),
    }
}

/// Compact picker label: `$2/$6`.
pub fn format_rates_short(input: f64, output: f64) -> String {
    format!("${}/${}", trim_rate(input), trim_rate(output))
}

fn trim_rate(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else {
        let s = format!("{v:.2}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn override_rates(o: &PriceOverride) -> Option<Rates> {
    // An override is a complete base-rate row, not permission to make an
    // unspecified direction free. Explicit zero remains valid for local models.
    let input = o.input_per_million?;
    let output = o.output_per_million?;
    if [
        Some(input),
        Some(output),
        o.cached_per_million,
        o.cache_write_per_million,
    ]
    .into_iter()
    .flatten()
    .any(|v| !v.is_finite() || v < 0.0)
    {
        return None;
    }
    Some(Rates {
        input_per_million: input,
        cached_per_million: o.cached_per_million.unwrap_or(input),
        output_per_million: output,
        cache_write_per_million: o.cache_write_per_million,
        long_threshold: None,
        long_input_per_million: None,
        long_cached_per_million: None,
        long_output_per_million: None,
        long_cache_write_per_million: None,
    })
}

/// Format a priced total for the status line. Unknown is `$?.??`, never `$0.00` unless the value is actually zero.
pub fn format_usd(total: Option<f64>) -> String {
    match total {
        // An empty f64 sum is -0.0, and a difference of totals can land a
        // hair below zero; either printed as "$-0.00".
        Some(v) if v.abs() < 0.005 => "$0.00".into(),
        Some(v) => format!("${v:.2}"),
        None => "$?.??".into(),
    }
}

/// Rates shipped for the built-in connections' models, from the providers'
/// price pages on 2026-10-19. Refresh at release.
fn shipped_rates(model: &str) -> Option<Rates> {
    grok_rates(model)
        .or_else(|| claude_rates(model))
        .or_else(|| gpt_rates(model))
}

/// A row with a higher rate from `threshold` input tokens. `write` is the
/// cache write as a multiple of input (`None`: writes are not charged, or
/// not reported, and bill as input).
fn tiered(
    (input, cached, output): (f64, f64, f64),
    threshold: u64,
    (long_in, long_cached, long_out): (f64, f64, f64),
    write: Option<f64>,
) -> Rates {
    Rates {
        input_per_million: input,
        cached_per_million: cached,
        output_per_million: output,
        cache_write_per_million: write.map(|w| w * input),
        long_threshold: Some(threshold),
        long_input_per_million: Some(long_in),
        long_cached_per_million: Some(long_cached),
        long_output_per_million: Some(long_out),
        long_cache_write_per_million: write.map(|w| w * long_in),
    }
}

/// A dated id (`claude-haiku-4-5-20251001`) prices as its model.
fn undated(model: &str) -> &str {
    match model.rsplit_once('-') {
        Some((base, date)) if date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit()) => base,
        _ => model,
    }
}

fn grok_rates(model: &str) -> Option<Rates> {
    // https://docs.x.ai/developers/models — a prompt of 200k tokens or more
    // is billed at the higher rate throughout.
    let (short, long) = match model {
        "grok-4.7" | "grok-4.7-latest" | "grok-4.6" | "grok-4.6-latest" => {
            ((2.00, 0.50, 6.00), (4.00, 1.00, 12.00))
        }
        "grok-4.5" | "grok-4.5-latest" | "grok-build-latest" => {
            ((2.00, 0.30, 6.00), (4.00, 0.60, 12.00))
        }
        "grok-4.3" | "grok-4.3-latest" => ((1.25, 0.20, 2.50), (2.50, 0.40, 5.00)),
        "grok-build-0.1" => ((1.00, 0.20, 2.00), (2.00, 0.40, 4.00)),
        _ => return None,
    };
    // SpaceXSI does not charge for writing the cache.
    Some(tiered(short, 200_000, long, None))
}

fn claude_rates(model: &str) -> Option<Rates> {
    // https://platform.claude.com/docs/en/about-claude/pricing, read
    // 2026-10-19. Ryter writes five-minute cache entries, which cost five
    // quarters of input on every model; the whole 1M window is at the
    // standard rate, except Haiku 5.5 past 100k tokens of prompt.
    let (input, cached, output) = match undated(model) {
        "claude-fable-5-1" | "claude-mythos-5-1" => (10.0, 0.25, 50.0),
        "claude-fable-5" | "claude-mythos-5" => (10.0, 1.0, 50.0),
        "claude-opus-5-5" => (4.0, 0.20, 20.0),
        "claude-opus-5" | "claude-opus-4-8" | "claude-opus-4-7" | "claude-opus-4-6"
        | "claude-opus-4-5" => (5.0, 0.50, 25.0),
        "claude-opus-4-1" | "claude-opus-4" => (15.0, 1.50, 75.0),
        "claude-sonnet-5-5" => (2.0, 0.10, 10.0),
        "claude-sonnet-5" => (2.0, 0.20, 10.0),
        "claude-sonnet-4-6" | "claude-sonnet-4-5" | "claude-sonnet-4" => (3.0, 0.30, 15.0),
        "claude-haiku-4-5" => (1.0, 0.10, 5.0),
        "claude-haiku-3-5" => (0.80, 0.08, 4.0),
        "claude-haiku-5-5" => {
            return Some(tiered(
                (0.10, 0.01, 0.50),
                100_000 + 1,
                (0.50, 0.05, 2.50),
                Some(1.25),
            ));
        }
        _ => return None,
    };
    Some(Rates {
        input_per_million: input,
        cached_per_million: cached,
        output_per_million: output,
        cache_write_per_million: Some(input * 1.25),
        long_threshold: None,
        long_input_per_million: None,
        long_cached_per_million: None,
        long_output_per_million: None,
        long_cache_write_per_million: None,
    })
}

fn gpt_rates(model: &str) -> Option<Rates> {
    // https://developers.openai.com/api/docs/pricing, read 2026-10-19:
    // standard tier; a prompt over 272k tokens is billed at the long-context
    // rate; a cache write is five quarters of input from GPT-5.6 on.
    let (short, long) = match model {
        "gpt-6-astra" => ((10.0, 1.0, 50.0), (20.0, 2.0, 75.0)),
        "gpt-6.1-sol" => ((2.0, 0.10, 10.0), (4.0, 0.20, 15.0)),
        "gpt-6-sol" => ((2.0, 0.20, 10.0), (4.0, 0.40, 15.0)),
        "gpt-6-luna" => ((0.10, 0.01, 0.50), (0.20, 0.02, 0.75)),
        "gpt-5.6-sol" => ((4.0, 0.40, 20.0), (8.0, 0.80, 30.0)),
        "gpt-5.6-terra" => ((2.0, 0.20, 12.0), (4.0, 0.40, 18.0)),
        "gpt-5.6-luna" => ((0.20, 0.02, 1.20), (0.40, 0.04, 1.80)),
        _ => return None,
    };
    Some(tiered(short, 272_000 + 1, long, Some(1.25)))
}

#[cfg(test)]
mod tests {

    #[test]
    fn zero_never_prints_negative() {
        let empty: f64 = Vec::<f64>::new().into_iter().sum();
        assert_eq!(format_usd(Some(empty)), "$0.00");
        assert_eq!(format_usd(Some(1.25 - 1.25000000001)), "$0.00");
        assert_eq!(
            format_usd(Some(-2.0)),
            "$-2.00",
            "a real negative still shows"
        );
    }

    use super::*;

    #[test]
    fn grok_46_short_context() {
        let book = PriceBook::new();
        let cost = book
            .cost(
                "grok-4.6",
                Usage {
                    input_tokens: 50_000,
                    output_tokens: 10_000,
                    cached_tokens: 0,
                    cache_write_tokens: 0,
                },
            )
            .unwrap();
        let expected = 50_000.0 * 2.0 / 1e6 + 10_000.0 * 6.0 / 1e6;
        assert!((cost - expected).abs() < 1e-9, "{cost}");
    }

    /// The shipped tables price the built-ins' models: Grok, Claude and
    /// GPT, each with its provider's long-context tier where it has one.
    #[test]
    fn the_shipped_tables_price_claude_and_gpt_too() {
        let book = PriceBook::new();
        // A tenth of a million each way, under every long-context line.
        let small = Usage {
            input_tokens: 100_000,
            output_tokens: 100_000,
            ..Usage::default()
        };
        let cost = |model: &str, usage: Usage| book.cost(model, usage).unwrap();
        assert!((cost("grok-4.7", small) - 0.8).abs() < 1e-9);
        assert!((cost("claude-opus-5-5", small) - 2.4).abs() < 1e-9);
        assert!((cost("claude-sonnet-5-5", small) - 1.2).abs() < 1e-9);
        assert!((cost("gpt-6.1-sol", small) - 1.2).abs() < 1e-9);
        // Over 272k tokens of prompt OpenAI doubles the input rate.
        let long = Usage {
            input_tokens: 300_000,
            output_tokens: 0,
            ..Usage::default()
        };
        assert!(
            (cost("gpt-6.1-sol", long) - 1.2).abs() < 1e-9,
            "{}",
            cost("gpt-6.1-sol", long)
        );
        let under = Usage {
            input_tokens: 272_000,
            ..long
        };
        assert!((cost("gpt-6.1-sol", under) - 0.544).abs() < 1e-9);
        // A cache read on Opus 5.5 is $0.20 per million; a write $5.
        let cached = Usage {
            input_tokens: 100_000,
            cached_tokens: 100_000,
            output_tokens: 0,
            ..Usage::default()
        };
        assert!((cost("claude-opus-5-5", cached) - 0.02).abs() < 1e-9);
        let written = Usage {
            input_tokens: 100_000,
            cache_write_tokens: 100_000,
            output_tokens: 0,
            ..Usage::default()
        };
        assert!((cost("claude-opus-5-5", written) - 0.5).abs() < 1e-9);
        // The page's own numbers, read 2026-10-19: Fable 5 reads at $1,
        // Sonnet 5.5 at $0.10, Haiku 5.5 past 100k at $0.05.
        assert!((cost("claude-fable-5", cached) - 0.1).abs() < 1e-9);
        assert!((cost("claude-sonnet-5-5", cached) - 0.01).abs() < 1e-9);
        let haiku_long = Usage {
            input_tokens: 100_001,
            cached_tokens: 100_001,
            ..Usage::default()
        };
        assert!((cost("claude-haiku-5-5", haiku_long) - 0.00500005).abs() < 1e-9);
        assert!(
            (cost("claude-haiku-5-5-20260301", cached) - 0.001).abs() < 1e-9,
            "a dated id"
        );
        // OpenAI writes at five quarters, from GPT-5.6 on; doubled over 272k.
        assert!((cost("gpt-6.1-sol", written) - 0.25).abs() < 1e-9);
        let long_written = Usage {
            input_tokens: 300_000,
            cache_write_tokens: 300_000,
            ..Usage::default()
        };
        assert!((cost("gpt-6.1-sol", long_written) - 1.5).abs() < 1e-9);
        // SpaceXSI reports no writes; a write would bill as input.
        assert_eq!(
            book.rates("grok-4.7").unwrap().cache_write_per_million,
            None
        );
        // A model the tables don't know is still unpriced, never free.
        assert_eq!(book.cost("claude-opus-9", small), None);
        assert_eq!(book.cost("gpt-7", small), None);
    }

    #[test]
    fn grok_46_long_context_uses_higher_rates_for_all_tokens() {
        let book = PriceBook::new();
        let cost = book
            .cost(
                "grok-4.6",
                Usage {
                    input_tokens: 200_000,
                    output_tokens: 1_000,
                    cached_tokens: 0,
                    cache_write_tokens: 0,
                },
            )
            .unwrap();
        let expected = 200_000.0 * 4.0 / 1e6 + 1_000.0 * 12.0 / 1e6;
        assert!((cost - expected).abs() < 1e-9, "{cost} vs {expected}");
    }

    #[test]
    fn unknown_model_is_none_not_zero() {
        let book = PriceBook::new();
        assert_eq!(
            book.cost(
                "mystery-model",
                Usage {
                    input_tokens: 100,
                    output_tokens: 100,
                    cached_tokens: 0,
                    cache_write_tokens: 0,
                },
            ),
            None
        );
        assert_eq!(format_usd(None), "$?.??");
        assert_eq!(format_usd(Some(0.0)), "$0.00");
        assert_eq!(format_usd(Some(0.42)), "$0.42");
        assert_eq!(
            book.format_model_rates("grok-4.6"),
            "$2/M input / $6/M output"
        );
        assert_eq!(format_rates(None), "$?.??/M input / $?.??/M output");
        assert_eq!(format_rates_short(2.0, 6.0), "$2/$6");
        assert_eq!(format_rates_short(3.0, 15.0), "$3/$15");
    }

    #[test]
    fn toml_override_wins() {
        let mut cfg = Config::default();
        cfg.pricing.insert(
            "grok-4.6".into(),
            PriceOverride {
                input_per_million: Some(10.0),
                cached_per_million: None,
                output_per_million: Some(20.0),
                cache_write_per_million: None,
            },
        );
        let book = PriceBook::from_config(&cfg);
        let cost = book
            .cost(
                "grok-4.6",
                Usage {
                    input_tokens: 1_000_000,
                    output_tokens: 1_000_000,
                    cached_tokens: 0,
                    cache_write_tokens: 0,
                },
            )
            .unwrap();
        assert!((cost - 30.0).abs() < 1e-9);
    }

    #[test]
    fn openrouter_catalog_fixture() {
        let json = include_str!("../fixtures/openrouter_models.json");
        let mut book = PriceBook::new();
        let models = crate::llm::parse_models_json(json).unwrap();
        book.ingest_model_info(&models);
        assert!(!models.is_empty());
        // prompt 0.000003 / token = $3 / M; completion 0.000015 = $15 / M
        let cost = book
            .cost(
                "anthropic/claude-sonnet-4.6",
                Usage {
                    input_tokens: 1_000_000,
                    output_tokens: 1_000_000,
                    cached_tokens: 0,
                    cache_write_tokens: 0,
                },
            )
            .unwrap();
        assert!((cost - 18.0).abs() < 1e-6, "{cost}");
    }

    /// Anthropic bills a prompt in three parts: plain input, cache reads
    /// (0.1×), and cache writes (1.25×). Before, writes were not counted and
    /// reads were subtracted from a count that already left them out.
    #[test]
    fn cache_reads_and_writes_are_priced_at_their_own_rates() {
        let mut cfg = Config::default();
        cfg.pricing.insert(
            "claude-sonnet-5".into(),
            PriceOverride {
                input_per_million: Some(3.0),
                cached_per_million: Some(0.3),
                output_per_million: Some(15.0),
                cache_write_per_million: Some(3.75),
            },
        );
        let book = PriceBook::from_config(&cfg);
        let cost = book
            .cost(
                "claude-sonnet-5",
                Usage {
                    input_tokens: 1_000_000 + 2_000_000 + 400_000,
                    output_tokens: 100_000,
                    cached_tokens: 2_000_000,
                    cache_write_tokens: 400_000,
                },
            )
            .unwrap();
        let expected = 3.0 + 2.0 * 0.3 + 0.4 * 3.75 + 0.1 * 15.0;
        assert!((cost - expected).abs() < 1e-9, "{cost} vs {expected}");
    }

    #[test]
    fn openrouter_cache_rates_are_read() {
        let json = r#"{"data":[{"id":"anthropic/claude-sonnet-5","pricing":{
            "prompt":"0.000003","completion":"0.000015",
            "input_cache_read":"0.0000003","input_cache_write":"0.00000375"}}]}"#;
        let mut book = PriceBook::new();
        book.ingest_model_info(&crate::llm::parse_models_json(json).unwrap());
        let r = book.rates("anthropic/claude-sonnet-5").unwrap();
        assert!((r.cached_per_million - 0.3).abs() < 1e-9);
        assert!((r.cache_write_per_million.unwrap() - 3.75).abs() < 1e-9);
    }

    fn row(id: &str, input: f64, output: f64) -> crate::llm::ModelInfo {
        crate::llm::ModelInfo {
            input_per_million: Some(input),
            output_per_million: Some(output),
            ..crate::llm::ModelInfo::named(id, None)
        }
    }

    /// The user's own audit turn on 0.31.0: Opus 5.5 read 26,478 of 27,691
    /// prompt tokens from the cache and Ryter said $0.16, every token at
    /// the input rate, because the model list (input and output only) had
    /// been laid over the shipped row and its cached rate. The list fills
    /// what the book does not know; it never undoes a cache discount.
    #[test]
    fn a_list_without_cache_prices_keeps_the_known_cache_rate() {
        let usage = Usage {
            input_tokens: 27_691,
            output_tokens: 2_642,
            cached_tokens: 26_478,
            cache_write_tokens: 0,
        };
        let mut book = PriceBook::new();
        book.ingest_model_info(&[row("claude-opus-5-5", 4.0, 20.0)]);
        let cost = book.cost("claude-opus-5-5", usage).unwrap();
        let expected = 1_213.0 * 4.0 / 1e6 + 26_478.0 * 0.20 / 1e6 + 2_642.0 * 20.0 / 1e6;
        assert!(
            (cost - expected).abs() < 1e-9,
            "{cost} vs {expected}, not 0.163604"
        );
        let r = book.rates("claude-opus-5-5").unwrap();
        assert_eq!(
            r.cache_write_per_million,
            Some(5.0),
            "the write rate rides too"
        );

        // A model nobody prices the cache of: no discount is invented.
        book.ingest_model_info(&[row("vendor/new-model", 1.0, 2.0)]);
        let r = book.rates("vendor/new-model").unwrap();
        assert_eq!(
            (r.cached_per_million, r.cache_write_per_million),
            (1.0, None)
        );
    }

    const PULLED: &str = r#"{"fetched": 1, "source": "test", "providers": {
        "anthropic": {"claude-opus-9": {"input": 1.0, "output": 5.0, "cache_read": 0.1, "cache_write": 1.25}},
        "xai": {"grok-9": {"input": 2.0, "output": 6.0, "cache_read": 0.5,
            "tiers": [{"size": 200000, "input": 4.0, "output": 12.0, "cache_read": 1.0}]}},
        "deepseek": {"deepseek-chat": {"input": 0.28, "output": 0.42, "cache_read": 0.028},
                     "shared-id": {"input": 1.0, "output": 1.0}},
        "other": {"shared-id": {"input": 2.0, "output": 2.0},
                  "claude-opus-9": {"input": 9.0, "output": 9.0}}
    }}"#;

    /// The pulled catalog prices what the shipped tables don't know, by
    /// the provider of each configured connection; a kind with no provider
    /// of its own takes any row the providers agree on.
    #[test]
    fn the_pulled_catalog_prices_by_the_connections_providers() {
        // The four built-ins: anthropic's and xai's rows, nobody else's.
        let mut cfg = Config::default();
        let mut book = PriceBook::from_config(&cfg);
        assert_eq!(
            book.ingest_pulled(&cfg, PULLED),
            2,
            "anthropic's and xai's rows"
        );
        let r = book.rates("claude-opus-9").unwrap();
        assert_eq!((r.input_per_million, r.cached_per_million), (1.0, 0.1));
        assert_eq!(book.rates("grok-9").unwrap().long_threshold, Some(200_001));
        assert!(book.rates("deepseek-chat").is_none(), "no open kind");
        assert!(book.rates("shared-id").is_none());

        cfg.connections.insert(
            "local".into(),
            crate::config::connection_template("openai_compat").unwrap(),
        );
        let mut book = PriceBook::from_config(&cfg);
        assert_eq!(
            book.ingest_pulled(&cfg, PULLED),
            3,
            "the built-ins' rows and deepseek-chat"
        );
        assert_eq!(
            book.rates("deepseek-chat").unwrap().cached_per_million,
            0.028
        );
        assert!(book.rates("shared-id").is_none(), "two providers disagree");
        assert_eq!(
            book.rates("claude-opus-9").unwrap().input_per_million,
            1.0,
            "the named provider's row, not other's"
        );

        // A list with only input and output defers to the pulled row.
        book.ingest_model_info(&[row("claude-opus-9", 1.0, 5.0)]);
        assert_eq!(book.rates("claude-opus-9").unwrap().cached_per_million, 0.1);
        // Garbage is no catalog.
        assert_eq!(PriceBook::new().ingest_pulled(&cfg, "nope"), 0);
    }

    #[test]
    fn load_reads_the_pulled_file_under_home() {
        let home = tempfile::tempdir().unwrap();
        let mut cfg = Config::default();
        cfg.connections.insert(
            "spacexsi".into(),
            crate::config::connection_template("spacexsi").unwrap(),
        );
        assert!(PriceBook::load(&cfg, home.path()).rates("grok-9").is_none());
        std::fs::write(crate::prices::cache_path(home.path()), PULLED).unwrap();
        let r = PriceBook::load(&cfg, home.path()).rates("grok-9").unwrap();
        assert_eq!(r.long_threshold, Some(200_001));
        assert_eq!(r.long_cached_per_million, Some(1.0));
        // Off: the file on disk is not read either.
        cfg.spend.catalog = false;
        assert!(PriceBook::load(&cfg, home.path()).rates("grok-9").is_none());
    }

    /// A model both sources know is priced by the shipped table, read by a
    /// person against the provider's page; a wrong or stale pulled row
    /// never overrides it, and a list without cache rates keeps it too.
    #[test]
    fn the_shipped_row_wins_over_the_pulled_one() {
        let pulled = r#"{"fetched": 1, "source": "test", "providers": {
            "anthropic": {"claude-opus-5-5": {"input": 99.0, "output": 199.0, "cache_read": 98.0},
                          "claude-opus-9": {"input": 1.0, "output": 5.0, "cache_read": 0.1}}}}"#;
        let cfg = Config::default();
        let mut book = PriceBook::from_config(&cfg);
        assert_eq!(book.ingest_pulled(&cfg, pulled), 2);
        let r = book.rates("claude-opus-5-5").unwrap();
        assert_eq!(
            (
                r.input_per_million,
                r.cached_per_million,
                r.output_per_million
            ),
            (4.0, 0.20, 20.0),
            "the shipped Opus 5.5 row, not the pulled one"
        );
        assert_eq!(book.rates("claude-opus-9").unwrap().input_per_million, 1.0);
        // A list row with input and output only keeps the shipped row.
        book.ingest_model_info(&[crate::llm::ModelInfo {
            id: "claude-opus-5-5".into(),
            context_length: None,
            input_per_million: Some(4.0),
            output_per_million: Some(20.0),
            cached_per_million: None,
            cache_write_per_million: None,
            connection: None,
            created: None,
            tools: None,
        }]);
        let r = book.rates("claude-opus-5-5").unwrap();
        assert_eq!(r.cached_per_million, 0.20);
        assert_eq!(r.cache_write_per_million, Some(5.0));
    }
}
#[test]
fn missing_or_invalid_override_rates_never_become_free() {
    for model in ["fixture-unknown", "grok-4.6"] {
        for (input, output) in [
            (Some(1.0), None),
            (None, Some(1.0)),
            (Some(-1.0), Some(1.0)),
            (Some(f64::NAN), Some(1.0)),
            (Some(1.0), Some(f64::INFINITY)),
        ] {
            let mut cfg = Config::default();
            cfg.pricing.insert(
                model.into(),
                PriceOverride {
                    input_per_million: input,
                    output_per_million: output,
                    ..PriceOverride::default()
                },
            );
            let book = PriceBook::from_config(&cfg);
            assert_eq!(
                book.cost(
                    model,
                    Usage {
                        output_tokens: 1_000,
                        ..Usage::default()
                    }
                ),
                None,
                "{model}: {input:?}/{output:?}"
            );
        }
    }
    let mut cfg = Config::default();
    cfg.pricing.insert(
        "free-fixture".into(),
        PriceOverride {
            input_per_million: Some(0.0),
            output_per_million: Some(0.0),
            ..PriceOverride::default()
        },
    );
    assert_eq!(
        PriceBook::from_config(&cfg).cost(
            "free-fixture",
            Usage {
                input_tokens: 100,
                output_tokens: 100,
                ..Usage::default()
            }
        ),
        Some(0.0)
    );
}
