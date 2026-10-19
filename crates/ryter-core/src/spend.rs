//! Token → USD pricing. Unknown rates yield `None` (`$?.??`), never a fake `$0.00`.

use std::collections::HashMap;

use serde::Deserialize;

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
}

impl Rates {
    /// Catalog row with only the two rates the sidebar shows.
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
        }
    }

    fn effective(self, usage: Usage) -> (f64, f64, f64) {
        let long = self.long_threshold.is_some_and(|t| usage.input_tokens >= t);
        if long {
            (
                self.long_input_per_million
                    .unwrap_or(self.input_per_million),
                self.long_cached_per_million
                    .unwrap_or(self.cached_per_million),
                self.long_output_per_million
                    .unwrap_or(self.output_per_million),
            )
        } else {
            (
                self.input_per_million,
                self.cached_per_million,
                self.output_per_million,
            )
        }
    }

    /// USD for `usage`. `input_tokens` is the whole prompt: cache reads and
    /// writes are the parts of it billed at their own rates.
    pub fn cost(self, usage: Usage) -> f64 {
        let (input, cached, output) = self.effective(usage);
        let write = self.cache_write_per_million.unwrap_or(input);
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

/// Looks up rates: TOML override → catalog (OpenRouter) → the shipped tables.
#[derive(Debug, Clone, Default)]
pub struct PriceBook {
    overrides: HashMap<String, Option<Rates>>,
    catalog: HashMap<String, Rates>,
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

    /// Merge OpenRouter `GET /api/v1/models` (or a fixture) into the catalog.
    pub fn ingest_openrouter_models(&mut self, json: &str) -> crate::Result<usize> {
        let parsed: OpenRouterModels = serde_json::from_str(json)
            .map_err(|e| crate::Error::Provider(format!("openrouter models: {e}")))?;
        let mut n = 0;
        for m in parsed.data {
            let Some(pricing) = m.pricing else { continue };
            let prompt = parse_per_token(pricing.prompt.as_deref())?;
            let completion = parse_per_token(pricing.completion.as_deref())?;
            let Some(input) = prompt else { continue };
            let Some(output) = completion else { continue };
            let read = parse_per_token(pricing.input_cache_read.as_deref())?.unwrap_or(input);
            let write = parse_per_token(pricing.input_cache_write.as_deref())?;
            self.catalog.insert(
                m.id,
                Rates {
                    input_per_million: input * 1_000_000.0,
                    cached_per_million: read * 1_000_000.0,
                    output_per_million: output * 1_000_000.0,
                    cache_write_per_million: write.map(|w| w * 1_000_000.0),
                    long_threshold: None,
                    long_input_per_million: None,
                    long_cached_per_million: None,
                    long_output_per_million: None,
                },
            );
            n += 1;
        }
        Ok(n)
    }

    /// Rates for `model`, if known.
    pub fn rates(&self, model: &str) -> Option<Rates> {
        if let Some(r) = self.overrides.get(model) {
            return *r;
        }
        if let Some(r) = self.catalog.get(model) {
            return Some(*r);
        }
        shipped_rates(model)
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
    pub fn ingest_model_info(&mut self, models: &[crate::llm::ModelInfo]) {
        for m in models {
            let Some(input) = m.input_per_million else {
                continue;
            };
            let Some(output) = m.output_per_million else {
                continue;
            };
            self.catalog
                .entry(m.id.clone())
                .or_insert(Rates::per_million(input, output));
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
    })
}

fn parse_per_token(raw: Option<&str>) -> crate::Result<Option<f64>> {
    let Some(s) = raw else { return Ok(None) };
    if s.is_empty() {
        return Ok(None);
    }
    s.parse::<f64>()
        .map(Some)
        .map_err(|e| crate::Error::Provider(format!("pricing {s:?}: {e}")))
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

/// A row with a higher rate from `threshold` input tokens.
fn tiered(
    (input, cached, output): (f64, f64, f64),
    threshold: u64,
    (long_in, long_cached, long_out): (f64, f64, f64),
) -> Rates {
    Rates {
        input_per_million: input,
        cached_per_million: cached,
        output_per_million: output,
        cache_write_per_million: None,
        long_threshold: Some(threshold),
        long_input_per_million: Some(long_in),
        long_cached_per_million: Some(long_cached),
        long_output_per_million: Some(long_out),
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
    Some(tiered(short, 200_000, long))
}

fn claude_rates(model: &str) -> Option<Rates> {
    // https://docs.claude.com/en/docs/about-claude/pricing. A cache read is
    // a tenth of input and a write five quarters where the page gives no
    // rate of its own; where it gives none for the write, the write is
    // billed as input (`None`).
    let (input, cached, output, write) = match model {
        "claude-fable-5-1" | "claude-fable-5" | "claude-mythos-5-1" => (10.0, 0.25, 50.0, None),
        "claude-opus-5-5" => (4.0, 0.20, 20.0, None),
        "claude-opus-5" => (5.0, 0.50, 25.0, None),
        "claude-opus-4-8" | "claude-opus-4-7" | "claude-opus-4-6" => (5.0, 0.50, 25.0, Some(6.25)),
        "claude-sonnet-5-5" | "claude-sonnet-5" => (2.0, 0.20, 10.0, None),
        "claude-sonnet-4-6" => (3.0, 0.30, 15.0, Some(3.75)),
        "claude-haiku-4-5" => (1.0, 0.10, 5.0, Some(1.25)),
        // Five times the rate past 100k tokens of prompt.
        "claude-haiku-5-5" => {
            return Some(tiered((0.10, 0.01, 0.50), 100_000, (0.50, 0.05, 2.50)));
        }
        _ => return None,
    };
    Some(Rates {
        input_per_million: input,
        cached_per_million: cached,
        output_per_million: output,
        cache_write_per_million: write,
        long_threshold: None,
        long_input_per_million: None,
        long_cached_per_million: None,
        long_output_per_million: None,
    })
}

fn gpt_rates(model: &str) -> Option<Rates> {
    // https://developers.openai.com/api/docs/pricing — standard tier; a
    // prompt over 272k tokens is billed at the long-context rate.
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
    Some(tiered(short, 272_000 + 1, long))
}

#[derive(Deserialize)]
struct OpenRouterModels {
    #[serde(default)]
    data: Vec<OpenRouterModel>,
}

#[derive(Deserialize)]
struct OpenRouterModel {
    id: String,
    #[serde(default)]
    pricing: Option<OpenRouterPricing>,
}

#[derive(Deserialize)]
struct OpenRouterPricing {
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    completion: Option<String>,
    #[serde(default)]
    input_cache_read: Option<String>,
    #[serde(default)]
    input_cache_write: Option<String>,
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
        // A cache read on Opus 5.5 is $0.20 per million.
        let cached = Usage {
            input_tokens: 100_000,
            cached_tokens: 100_000,
            output_tokens: 0,
            ..Usage::default()
        };
        assert!((cost("claude-opus-5-5", cached) - 0.02).abs() < 1e-9);
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
        let n = book.ingest_openrouter_models(json).unwrap();
        assert!(n >= 1);
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
        book.ingest_openrouter_models(json).unwrap();
        let r = book.rates("anthropic/claude-sonnet-5").unwrap();
        assert!((r.cached_per_million - 0.3).abs() < 1e-9);
        assert!((r.cache_write_per_million.unwrap() - 3.75).abs() < 1e-9);
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
