//! Suggest a cost-tiered crew from the models the user can actually reach.
//!
//! The economics (`docs/cost.md`): the builder burns most of the tokens, so it
//! should be the cheapest capable model; the auditor and architect see little
//! input and decide quality, so they should be strong. The auditor must also be
//! a different model from the lead and the builder, and ideally from a
//! different vendor, so its sign-off is a second opinion.
//!
//! Price is the only quality signal available before `ryter bench` has run, and
//! it is a weak one. Every suggestion says so.

use std::collections::{BTreeMap, HashSet};

use crate::config::RoleModel;
use crate::crew::same_model;
use crate::llm::ModelInfo;

/// Every model on every connection that has a key (local ones need none),
/// priced from the price book where the catalog did not say. Connections that
/// fail to list are reported, not fatal.
pub async fn reachable_models(cfg: &crate::config::Config) -> (Vec<ModelInfo>, Vec<String>) {
    let book = crate::spend::PriceBook::from_config(cfg);
    let mut all = Vec::new();
    let mut failed = Vec::new();
    for (name, conn) in &cfg.connections {
        let Ok(key) = crate::config::resolve_secret(cfg, &crate::ids::ConnectionId::new(name))
        else {
            continue;
        };
        let provider = crate::llm::http_provider(conn, key);
        match crate::llm::Provider::list_models(&provider).await {
            Ok(models) => {
                for mut m in models {
                    if m.input_per_million.is_none() {
                        if let Some(r) = book.rates(&m.id) {
                            m.input_per_million = Some(r.input_per_million);
                            m.output_per_million = Some(r.output_per_million);
                        }
                    }
                    m.connection = Some(name.clone());
                    all.push(m);
                }
            }
            Err(e) => failed.push(format!("{name}: {e}")),
        }
    }
    (all, failed)
}

/// Above this blended price ($/M tokens) a model is a premium outlier, not a
/// default. docs/cost.md prices the auditor and architect at a mainstream
/// flagship; a "pro" tier several times that makes every audit cost several
/// times the model. Assign one by hand if you want it.
const STRONG_CEILING: f64 = 15.0;
/// Any crew role needs room for a task, a diff, and its tool output.
const MIN_WINDOW: u64 = 64_000;
/// Only models released within this long of the newest one in the catalog are
/// ranked. Old flagships keep their price long after they stop being strong:
/// on a live catalog, 2023's gpt-4 was the "strongest" model by price alone.
const RECENT_SECS: u64 = 18 * 30 * 24 * 3600;
/// A cloud builder cheaper than this fraction of the strongest model is
/// unlikely to land tasks; price cannot say, so do not default below it.
/// `ryter bench` is how to justify going lower.
const BUILDER_FLOOR: f64 = 1.0 / 30.0;

/// Ids that are not chat models a crew role could use.
const NOT_CHAT: &[&str] = &[
    "embed",
    "whisper",
    "tts",
    "dall-e",
    "image",
    "moderation",
    "audio",
    "transcribe",
    "realtime",
    "rerank",
    "guard",
    "search-preview",
];

/// One role's pick.
#[derive(Debug, Clone, PartialEq)]
pub struct Pick {
    /// Connection name.
    pub connection: String,
    /// Model id.
    pub model: String,
    /// Blended $/M tokens (0 for a local model).
    pub blended: f64,
    /// Runs on this machine.
    pub local: bool,
}

impl Pick {
    /// `qwen3-coder (local, $0)`, `grok-4.3 on spacexai (~$1.50/M)`.
    pub fn label(&self) -> String {
        if self.local {
            format!("{} on {} (local, $0)", self.model, self.connection)
        } else {
            format!(
                "{} on {} (~${:.2}/M)",
                self.model, self.connection, self.blended
            )
        }
    }
}

/// A suggested crew and why.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tiering {
    /// Cheapest capable model.
    pub builder: Option<Pick>,
    /// Strongest model from another vendor than the lead and builder.
    pub auditor: Option<Pick>,
    /// Strongest model.
    pub architect: Option<Pick>,
    /// Caveats the user should read.
    pub notes: Vec<String>,
}

impl Tiering {
    /// Rows for `/crew` / `crew.toml`.
    pub fn as_specialists(&self) -> BTreeMap<String, RoleModel> {
        let mut m = BTreeMap::new();
        for (role, pick) in [
            ("builder", &self.builder),
            ("auditor", &self.auditor),
            ("architect", &self.architect),
        ] {
            if let Some(p) = pick {
                m.insert(
                    role.to_string(),
                    RoleModel {
                        connection: Some(p.connection.clone()),
                        model: Some(p.model.clone()),
                    },
                );
            }
        }
        m
    }

    /// Human-readable summary.
    pub fn render(&self) -> String {
        let row = |role: &str, p: &Option<Pick>| {
            format!(
                "{role:<10}{}\n",
                p.as_ref()
                    .map(Pick::label)
                    .unwrap_or_else(|| "(no suitable model)".into())
            )
        };
        let mut s = String::new();
        s.push_str(&row("builder", &self.builder));
        s.push_str(&row("auditor", &self.auditor));
        s.push_str(&row("architect", &self.architect));
        for n in &self.notes {
            s.push_str("· ");
            s.push_str(n);
            s.push('\n');
        }
        s
    }
}

/// The vendor behind a model, so the auditor can come from a different one.
/// OpenRouter ids carry it (`anthropic/claude-…`); direct ids are recognised
/// by name; anything else is its own family (its connection).
pub fn family(connection: &str, model: &str) -> String {
    if let Some((vendor, _)) = model.split_once('/') {
        return vendor.to_ascii_lowercase();
    }
    let m = model.to_ascii_lowercase();
    let known = [
        ("grok", "x-ai"),
        ("claude", "anthropic"),
        ("gpt", "openai"),
        ("o1", "openai"),
        ("o3", "openai"),
        ("o4", "openai"),
        ("gemini", "google"),
        ("gemma", "google"),
        ("llama", "meta"),
        ("qwen", "qwen"),
        ("deepseek", "deepseek"),
        ("mistral", "mistralai"),
        ("codestral", "mistralai"),
        ("devstral", "mistralai"),
        ("glm", "z-ai"),
        ("kimi", "moonshotai"),
    ];
    known
        .iter()
        .find(|(p, _)| m.starts_with(p))
        .map(|(_, v)| (*v).to_string())
        .unwrap_or_else(|| connection.to_string())
}

struct Candidate {
    pick: Pick,
    family: String,
}

/// Why a model can never fill a crew role, if it cannot.
fn ineligible(m: &ModelInfo, newest: Option<u64>, local: bool) -> bool {
    let id = m.id.to_ascii_lowercase();
    NOT_CHAT.iter().any(|x| id.contains(x))
        // Cloud `:variant` ids are routing tweaks of a model that is also
        // listed plainly (`:batch` does not even stream). Local tags like
        // `qwen3-coder:30b` are the model's name.
        || (!local && id.contains(':'))
        // Router meta-models pick a model per request, at a variable price.
        || id.starts_with("openrouter/")

        || m.tools == Some(false)
        || m.context_length.is_some_and(|w| w < MIN_WINDOW)
        || [m.input_per_million, m.output_per_million]
            .iter()
            .any(|p| p.is_some_and(|p| p < 0.0))
        || matches!((m.created, newest), (Some(c), Some(n)) if n.saturating_sub(c) > RECENT_SECS)
}

/// Suggest a builder, auditor, and architect for a crew led by
/// `lead_model` on `lead_connection`, from every model the user can reach.
pub fn suggest(
    lead_connection: &str,
    lead_model: &str,
    models: &[ModelInfo],
    local: &HashSet<String>,
) -> Tiering {
    let mut t = Tiering::default();
    let mut unpriced = 0usize;
    let mut cands: Vec<Candidate> = Vec::new();
    let newest = models.iter().filter_map(|m| m.created).max();
    let mut dropped = 0usize;
    for m in models {
        let conn = m
            .connection
            .clone()
            .unwrap_or_else(|| lead_connection.to_string());
        if ineligible(m, newest, local.contains(&conn)) {
            dropped += 1;
            continue;
        }
        let is_local = local.contains(&conn);
        let blended = if is_local {
            Some(0.0)
        } else {
            match (m.input_per_million, m.output_per_million) {
                (Some(i), Some(o)) => Some(0.8 * i + 0.2 * o),
                _ => None,
            }
        };
        let Some(blended) = blended else {
            unpriced += 1;
            continue;
        };
        cands.push(Candidate {
            family: family(&conn, &m.id),
            pick: Pick {
                connection: conn,
                model: m.id.clone(),
                blended,
                local: is_local,
            },
        });
    }
    let _ = dropped;
    if unpriced > 0 {
        t.notes.push(format!(
            "{unpriced} model(s) skipped: no known price. Add them under [pricing] to include them."
        ));
    }

    let strong = |c: &&Candidate| !c.pick.local && c.pick.blended <= STRONG_CEILING;
    // Architect first: the strongest model sets the builder's price band.
    t.architect = cands
        .iter()
        .filter(strong)
        .max_by(|a, b| a.pick.blended.total_cmp(&b.pick.blended))
        .map(|c| c.pick.clone());
    let top = t.architect.as_ref().map(|a| a.blended).unwrap_or(0.0);

    // Builder: a local model if there is one (free); otherwise the lead's own
    // model when it sits in the builder band — the user chose it — else the
    // cheapest model in the band. The band: at most a quarter of the strongest
    // model, at least BUILDER_FLOOR of it.
    let in_band = |c: &&Candidate| {
        top == 0.0 || (c.pick.blended >= top * BUILDER_FLOOR && c.pick.blended <= top / 4.0)
    };
    let builder = cands
        .iter()
        .filter(|c| c.pick.local)
        .min_by(|a, b| a.pick.blended.total_cmp(&b.pick.blended))
        // The floor keeps unknown bargain-bin models out of the default; it
        // does not overrule a model the user already chose to run.
        .or_else(|| {
            cands
                .iter()
                .filter(|c| top == 0.0 || c.pick.blended <= top / 4.0)
                .find(|c| same_model(&c.pick.model, lead_model))
        })
        .or_else(|| {
            cands
                .iter()
                .filter(|c| !c.pick.local)
                .filter(in_band)
                .min_by(|a, b| a.pick.blended.total_cmp(&b.pick.blended))
        });
    // Nothing is a quarter the price of the strongest model (one provider's
    // catalog, typically): the cheapest above the floor is still the best
    // builder, but tiering saves little and the user should know.
    let builder = builder.or_else(|| {
        let b = cands
            .iter()
            .filter(|c| c.pick.blended >= top * BUILDER_FLOOR)
            .min_by(|a, b| a.pick.blended.total_cmp(&b.pick.blended));
        if b.is_some() {
            t.notes.push(
                "No model you can reach is much cheaper than your strongest, so tiering saves \
                 little. A cheaper provider, or a local model, is where the savings are."
                    .into(),
            );
        }
        b
    });
    t.builder = builder.map(|c| c.pick.clone());
    let lead_family = family(lead_connection, lead_model);
    let builder_family = builder.map(|b| b.family.clone()).unwrap_or_default();
    let builder_model = t
        .builder
        .as_ref()
        .map(|b| b.model.clone())
        .unwrap_or_default();
    let independent = |c: &&Candidate| {
        !same_model(&c.pick.model, lead_model) && !same_model(&c.pick.model, &builder_model)
    };

    // Auditor: strongest independent model, preferring another vendor.
    let other_vendor = cands
        .iter()
        .filter(strong)
        .filter(independent)
        .filter(|c| c.family != lead_family && c.family != builder_family)
        .max_by(|a, b| a.pick.blended.total_cmp(&b.pick.blended));
    t.auditor = match other_vendor {
        Some(c) => Some(c.pick.clone()),
        None => {
            let same_vendor = cands
                .iter()
                .filter(strong)
                .filter(independent)
                .max_by(|a, b| a.pick.blended.total_cmp(&b.pick.blended));
            if same_vendor.is_some() {
                t.notes.push(
                    "The auditor is from the same vendor as the lead or builder: a different \
                     model, but a less independent opinion. Connect a second provider for a \
                     stronger sign-off."
                        .into(),
                );
            }
            same_vendor.map(|c| c.pick.clone())
        }
    };
    if t.auditor.is_none() {
        t.notes.push(
            "No model qualifies as an auditor: it must differ from the lead and the builder. \
             Connect a second provider."
                .into(),
        );
    }

    if let Some(b) = &t.builder {
        if b.local {
            t.notes.push(
                "The builder is a local model: free, but its tool use varies a lot between \
                 models. Run `ryter bench` before trusting it with real work."
                    .into(),
            );
        }
    }
    if let (Some(lead), Some(b)) = (
        cands.iter().find(|c| same_model(&c.pick.model, lead_model)),
        &t.builder,
    ) {
        // Only worth saying when the lead is genuinely expensive.
        if lead.pick.blended >= 3.0 && lead.pick.blended > 4.0 * b.blended {
            t.notes.push(format!(
                "Your lead ({lead_model}, ~${:.2}/M) mostly converses and writes tasks; a \
                 mid-priced model would do, and it runs on every turn.",
                lead.pick.blended
            ));
        }
    }
    t.notes.push(
        "Picked by price, the only signal before a benchmark. `ryter bench` measures what \
         actually lands."
            .into(),
    );
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(conn: &str, id: &str, i: f64, o: f64) -> ModelInfo {
        ModelInfo {
            id: id.into(),
            context_length: Some(200_000),
            input_per_million: Some(i),
            output_per_million: Some(o),
            connection: Some(conn.into()),
            created: None,
            tools: None,
        }
    }

    fn unpriced(conn: &str, id: &str) -> ModelInfo {
        ModelInfo {
            id: id.into(),
            context_length: None,
            input_per_million: None,
            output_per_million: None,
            connection: Some(conn.into()),
            created: None,
            tools: None,
        }
    }

    fn catalog() -> Vec<ModelInfo> {
        vec![
            m("spacexai", "grok-4.6", 2.0, 6.0),
            m("spacexai", "grok-4.3", 1.25, 2.5),
            m("openrouter", "anthropic/claude-sonnet-4.6", 3.0, 15.0),
            m("openrouter", "deepseek/deepseek-v4", 0.3, 1.2),
            m("openrouter", "openai/text-embedding-3", 0.02, 0.0),
            m("openrouter", "vendor/ultra-pro", 150.0, 600.0),
        ]
    }

    #[test]
    fn cheap_builder_strong_independent_auditor() {
        let t = suggest("spacexai", "grok-4.6", &catalog(), &HashSet::new());
        assert_eq!(t.builder.unwrap().model, "deepseek/deepseek-v4");
        let a = t.auditor.unwrap();
        assert_eq!(
            a.model, "anthropic/claude-sonnet-4.6",
            "strongest from another vendor"
        );
        // The ultra-premium outlier is never a default.
        assert_eq!(t.architect.unwrap().model, "anthropic/claude-sonnet-4.6");
    }

    fn aged(mut m: ModelInfo, created: u64, window: u64, tools: bool) -> ModelInfo {
        m.created = Some(created);
        m.context_length = Some(window);
        m.tools = Some(tools);
        m
    }

    /// Cases from a live OpenRouter catalog that broke price-only ranking.
    #[test]
    fn a_live_catalogs_traps_are_avoided() {
        let now = 1_789_000_000;
        let models = vec![
            aged(
                m("openrouter", "deepseek/deepseek-v4.1-flash", 0.3, 1.2),
                now,
                1_048_576,
                true,
            ),
            aged(
                m("openrouter", "anthropic/claude-sonnet-4.6", 3.0, 15.0),
                now - 200 * 86_400,
                1_000_000,
                true,
            ),
            // Router meta-model priced at -1 per token: not "the cheapest".
            aged(
                m("openrouter", "openrouter/auto-beta", -1e6, -1e6),
                now,
                2_000_000,
                true,
            ),
            // Expensive because it is old, not because it is strong.
            aged(
                m("openrouter", "openai/gpt-4", 30.0, 60.0),
                now - 3 * 365 * 86_400,
                8_191,
                true,
            ),
            // Cheap but cannot call tools.
            aged(
                m("openrouter", "vendor/no-tools", 0.01, 0.01),
                now,
                128_000,
                false,
            ),
            // Free tier: rate-limited below what a build loop needs.
            aged(
                m("openrouter", "vendor/tiny:free", 0.0, 0.0),
                now,
                128_000,
                true,
            ),
            // Batch API variant: does not stream.
            aged(
                m("openrouter", "anthropic/claude-opus-4.1:batch", 7.5, 37.5),
                now,
                200_000,
                true,
            ),
            // Cheaper than any plausible builder: below the floor.
            aged(
                m("openrouter", "vendor/ling-flash", 0.02, 0.05),
                now,
                128_000,
                true,
            ),
            // Premium tier: every audit would cost several times the model.
            aged(
                m("openrouter", "openai/gpt-5-pro", 15.0, 120.0),
                now,
                400_000,
                true,
            ),
        ];
        let t = suggest(
            "openrouter",
            "deepseek/deepseek-v4.1-flash",
            &models,
            &HashSet::new(),
        );
        assert_eq!(
            t.builder.as_ref().unwrap().model,
            "deepseek/deepseek-v4.1-flash"
        );
        assert_eq!(
            t.auditor.as_ref().unwrap().model,
            "anthropic/claude-sonnet-4.6"
        );
        assert_eq!(
            t.architect.as_ref().unwrap().model,
            "anthropic/claude-sonnet-4.6"
        );
        let text = t.render();
        for trap in [
            "auto-beta",
            "gpt-4\n",
            "no-tools",
            ":free",
            "gpt-5-pro",
            ":batch",
            "ling-flash",
        ] {
            assert!(!text.contains(trap), "{trap} was picked:\n{text}");
        }
        // A $0.48/M lead is not "expensive".
        assert!(!text.contains("mostly converses"), "{text}");
    }

    /// The user chose their lead; when it is priced like a builder, it is the
    /// builder — not a cheaper stranger.
    #[test]
    fn a_builder_priced_lead_is_preferred_as_builder() {
        let now = 1_789_000_000;
        let models = vec![
            // Live price: below the floor, but the user runs it as the lead.
            aged(
                m("openrouter", "deepseek/deepseek-v4.1-flash", 0.15, 0.6),
                now,
                1_048_576,
                true,
            ),
            aged(
                m("openrouter", "inception/mercury-2", 0.25, 0.75),
                now,
                128_000,
                true,
            ),
            aged(
                m("openrouter", "openai/gpt-5.5", 5.0, 30.0),
                now,
                400_000,
                true,
            ),
        ];
        let t = suggest(
            "openrouter",
            "deepseek/deepseek-v4.1-flash",
            &models,
            &HashSet::new(),
        );
        assert_eq!(t.builder.unwrap().model, "deepseek/deepseek-v4.1-flash");
    }

    #[test]
    fn embeddings_are_never_picked() {
        let t = suggest("spacexai", "grok-4.6", &catalog(), &HashSet::new());
        assert!(!t.render().contains("embedding"));
    }

    /// The auditor must pass the same rule the gate enforces.
    #[test]
    fn the_auditor_passes_the_independence_rule() {
        let t = suggest("spacexai", "grok-4.6", &catalog(), &HashSet::new());
        let a = t.auditor.unwrap();
        let b = t.builder.unwrap();
        assert!(!same_model(&a.model, "grok-4.6"));
        assert!(!same_model(&a.model, &b.model));
    }

    #[test]
    fn a_local_model_becomes_the_free_builder() {
        let mut models = catalog();
        models.push(unpriced("ollama", "qwen3-coder:30b"));
        let local: HashSet<String> = ["ollama".to_string()].into();
        let t = suggest("spacexai", "grok-4.6", &models, &local);
        let b = t.builder.clone().unwrap();
        assert!(b.local && b.model == "qwen3-coder:30b");
        assert!(t.render().contains("local, $0"));
        assert!(t.notes.iter().any(|n| n.contains("ryter bench")));
    }

    /// One provider only: the auditor is a different model of the same vendor,
    /// and the user is told why that is weaker.
    #[test]
    fn a_single_vendor_crew_says_its_auditor_is_weaker() {
        let models = vec![
            m("spacexai", "grok-4.6", 2.0, 6.0),
            m("spacexai", "grok-4.5", 2.0, 6.0),
            m("spacexai", "grok-4.3", 1.25, 2.5),
        ];
        let t = suggest("spacexai", "grok-4.6", &models, &HashSet::new());
        assert_eq!(t.builder.as_ref().unwrap().model, "grok-4.3");
        assert_eq!(t.auditor.as_ref().unwrap().model, "grok-4.5");
        assert!(t.notes.iter().any(|n| n.contains("same vendor")));
    }

    #[test]
    fn no_second_model_means_no_auditor_and_a_clear_note() {
        let models = vec![m("spacexai", "grok-4.6", 2.0, 6.0)];
        let t = suggest("spacexai", "grok-4.6", &models, &HashSet::new());
        assert!(t.auditor.is_none());
        assert!(
            t.notes
                .iter()
                .any(|n| n.contains("Connect a second provider"))
        );
    }

    #[test]
    fn unpriced_models_are_counted_not_guessed() {
        let mut models = catalog();
        models.push(unpriced("anthropic", "claude-opus-4.6"));
        let t = suggest("spacexai", "grok-4.6", &models, &HashSet::new());
        assert!(t.notes.iter().any(|n| n.starts_with("1 model(s) skipped")));
    }

    #[test]
    fn families_are_recognised_across_routes() {
        assert_eq!(
            family("openrouter", "anthropic/claude-sonnet-4.6"),
            "anthropic"
        );
        assert_eq!(family("anthropic", "claude-sonnet-4-6"), "anthropic");
        assert_eq!(family("spacexai", "grok-4.6"), "x-ai");
        assert_eq!(family("box", "my-finetune"), "box");
    }

    #[test]
    fn the_suggestion_becomes_crew_rows() {
        let t = suggest("spacexai", "grok-4.6", &catalog(), &HashSet::new());
        let rows = t.as_specialists();
        assert_eq!(
            rows["builder"].model.as_deref(),
            Some("deepseek/deepseek-v4")
        );
        assert_eq!(rows["auditor"].connection.as_deref(), Some("openrouter"));
    }
}
