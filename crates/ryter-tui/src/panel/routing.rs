//! `/provider` → `r` — the routing rules for an OpenRouter connection
//! (`R-POP-81`): OpenRouter's `provider` object as a form, saved to
//! `~/.ryter/routing.toml` and sent with every request on the connection.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::text::{Line, Span};
use ryter_core::{RoutingConfig, RoutingMaxPrice, RoutingPercentiles, RoutingSort};

use super::widgets::{Field, Form, Kind};
use super::{Body, Outcome, Panel};
use crate::action::Action;
use crate::theme::Theme;
use crate::view::View;

/// The sort choices, OpenRouter's own load balancing first.
const SORTS: [&str; 4] = ["openrouter's choice", "price", "throughput", "latency"];
/// A setting OpenRouter has a default for: left to it, or set either way.
const TRI: [&str; 3] = ["default", "on", "off"];
/// `partition`: OpenRouter's default, or one of its two words.
const PARTITIONS: [&str; 3] = ["default", "model", "none"];
/// `data_collection`.
const COLLECTION: [&str; 3] = ["default", "allow", "deny"];

/// The routing form for one connection.
#[derive(Debug, Clone)]
pub struct Routing {
    /// The connection's name.
    connection: String,
    form: Form,
    /// Editing the focused text field via the composer.
    editing: bool,
    /// `Esc` with edits asks first (`R-POP-50`).
    confirm_discard: bool,
}

fn select(id: &'static str, label: &str, options: &[&str], idx: usize) -> Field {
    Field::new(
        id,
        label,
        Kind::Select {
            options: options.iter().map(|s| (*s).to_string()).collect(),
            idx,
        },
    )
    .origin(if idx == 0 { "default" } else { "set" })
}

fn text(id: &'static str, label: &str, value: String) -> Field {
    let origin = if value.is_empty() { "default" } else { "set" };
    Field::new(id, label, Kind::Text(value)).origin(origin)
}

/// `Some(true)` → `on`, `Some(false)` → `off`, `None` → `default`.
fn tri(flag: Option<bool>) -> usize {
    match flag {
        None => 0,
        Some(true) => 1,
        Some(false) => 2,
    }
}

fn flag(idx: usize) -> Option<bool> {
    match idx {
        1 => Some(true),
        2 => Some(false),
        _ => None,
    }
}

/// A number as the form shows it: `80`, not `80.00`.
fn num(v: f64) -> String {
    format!("{v}")
}

fn percentiles(p: &Option<RoutingPercentiles>) -> String {
    match p {
        None => String::new(),
        Some(RoutingPercentiles::Median(v)) => num(*v),
        Some(RoutingPercentiles::Percentiles { p50, p75, p90, p99 }) => {
            [("p50", p50), ("p75", p75), ("p90", p90), ("p99", p99)]
                .iter()
                .filter_map(|(k, v)| v.map(|v| format!("{k}={}", num(v))))
                .collect::<Vec<_>>()
                .join(" ")
        }
    }
}

/// Words of a host or quantization list: commas or spaces between them.
fn list(text: &str) -> Vec<String> {
    text.split(|c: char| c == ',' || c.is_whitespace())
        .map(str::trim)
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// A dollar figure or a count: a number, finite and not below zero.
fn number(text: &str) -> Result<Option<f64>, String> {
    let text = text.trim().trim_start_matches('$');
    if text.is_empty() {
        return Ok(None);
    }
    match text.parse::<f64>() {
        Ok(v) if v.is_finite() && v >= 0.0 => Ok(Some(v)),
        Ok(_) => Err("not below zero".into()),
        Err(_) => Err("a number, e.g. 0.5".into()),
    }
}

/// `80` for the median, or `p50=80 p90=40` for percentiles by name.
fn parse_percentiles(text: &str) -> Result<Option<RoutingPercentiles>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    if !text.contains('=') {
        return number(text).map(|v| v.map(RoutingPercentiles::Median));
    }
    let (mut p50, mut p75, mut p90, mut p99) = (None, None, None, None);
    for word in text.split(|c: char| c == ',' || c.is_whitespace()) {
        if word.is_empty() {
            continue;
        }
        let Some((k, v)) = word.split_once('=') else {
            return Err("a number, or p50=80 p90=40".into());
        };
        let v = number(v)?.ok_or_else(|| "a number, or p50=80 p90=40".to_string())?;
        match k.trim() {
            "p50" => p50 = Some(v),
            "p75" => p75 = Some(v),
            "p90" => p90 = Some(v),
            "p99" => p99 = Some(v),
            other => return Err(format!("{other} is not p50, p75, p90 or p99")),
        }
    }
    Ok(Some(RoutingPercentiles::Percentiles { p50, p75, p90, p99 }))
}

/// What a rule does, in a line under the focused field.
fn note_for(id: &str) -> &'static str {
    match id {
        "sort" => {
            "price: the cheapest host; throughput: the fastest; latency: the soonest to answer. A sort turns OpenRouter's load balancing off"
        }
        "partition" => {
            "with a sort: model sorts each model's hosts (OpenRouter's default), none sorts all hosts together"
        }
        "order" => {
            "hosts to try first, by OpenRouter's slug: baseten, wafer, or an endpoint's baseten/fp8/us"
        }
        "allow_fallbacks" => {
            "whether a host outside the order may serve when those fail; OpenRouter's default is on"
        }
        "only" => {
            "the only hosts allowed; a region is part of an endpoint's slug, such as baseten/fp8/us"
        }
        "ignore" => "hosts never used",
        "quantizations" => {
            "what a host may serve the model at: fp8, bf16, fp16, fp32, int8, int4, fp4, unknown. Empty allows any"
        }
        "require_parameters" => {
            "only hosts that take every parameter of the request; Ryter sends tools and a reasoning effort"
        }
        "data_collection" => {
            "deny keeps out hosts that keep or train on prompts; OpenRouter's default allows them"
        }
        "zdr" => "only zero-data-retention endpoints",
        "enforce_distillable_text" => "only models whose terms allow distilling their text",
        "min_throughput" => {
            "tokens per second a host should reach: 80, or p50=80 p90=40. A preference: served even when no host reaches it"
        }
        "max_latency" => {
            "seconds to the first token a host should stay under: 2, or p50=2 p90=5. A preference"
        }
        "max_prompt" | "max_completion" => {
            "dollars per million tokens. A bar: a request no host serves under it fails and says so"
        }
        "max_request" | "max_image" => "dollars; a bar, as above",
        "clear" => "every rule back to OpenRouter's choice; saved with ^s",
        _ => "",
    }
}

impl Routing {
    /// The form for `connection`, filled from its rules.
    pub fn new(connection: &str, routing: Option<&RoutingConfig>) -> Self {
        Self {
            connection: connection.to_string(),
            form: Form::new(Self::fields(routing.cloned().unwrap_or_default())),
            editing: false,
            confirm_discard: false,
        }
    }

    fn fields(r: RoutingConfig) -> Vec<Field> {
        let sort_idx = r
            .sort
            .as_ref()
            .and_then(|s| SORTS.iter().position(|w| *w == s.by()))
            .unwrap_or(0);
        let partition_idx = match &r.sort {
            Some(RoutingSort::Detailed {
                partition: Some(p), ..
            }) => PARTITIONS.iter().position(|w| w == p).unwrap_or(0),
            _ => 0,
        };
        let collection_idx = r
            .data_collection
            .as_deref()
            .and_then(|w| COLLECTION.iter().position(|c| *c == w))
            .unwrap_or(0);
        let price = r.max_price.clone().unwrap_or_default();
        let money = |v: Option<f64>| v.map(num).unwrap_or_default();
        vec![
            Field::new("g_order", "order of hosts", Kind::Header),
            select("sort", "sort hosts by", &SORTS, sort_idx),
            select("partition", "sort within", &PARTITIONS, partition_idx),
            text("order", "try first", r.order.join(", ")),
            select(
                "allow_fallbacks",
                "fallbacks past them",
                &TRI,
                tri(r.allow_fallbacks),
            ),
            text("only", "only these hosts", r.only.join(", ")),
            text("ignore", "never these hosts", r.ignore.join(", ")),
            Field::new("g_offer", "what a host must offer", Kind::Header),
            text("quantizations", "quantizations", r.quantizations.join(", ")),
            select(
                "require_parameters",
                "every parameter",
                &TRI,
                tri(r.require_parameters),
            ),
            select(
                "data_collection",
                "data collection",
                &COLLECTION,
                collection_idx,
            ),
            select("zdr", "zero data retention", &TRI, tri(r.zdr)),
            select(
                "enforce_distillable_text",
                "distillable text only",
                &TRI,
                tri(r.enforce_distillable_text),
            ),
            Field::new("g_speed", "speed, a preference", Kind::Header),
            text(
                "min_throughput",
                "min throughput, tok/s",
                percentiles(&r.preferred_min_throughput),
            ),
            text(
                "max_latency",
                "max latency, seconds",
                percentiles(&r.preferred_max_latency),
            ),
            Field::new("g_price", "price, a bar", Kind::Header),
            text("max_prompt", "max $ per M prompt", money(price.prompt)),
            text(
                "max_completion",
                "max $ per M completion",
                money(price.completion),
            ),
            text("max_request", "max $ per request", money(price.request)),
            text("max_image", "max $ per image", money(price.image)),
            Field::new("g_clear", "", Kind::Header),
            Field::new("clear", "clear every rule", Kind::Action),
        ]
    }

    /// The form as a table, or the first field that cannot be read, with
    /// what is wrong with it.
    fn to_config(&self) -> Result<RoutingConfig, (&'static str, String)> {
        let f = &self.form;
        let idx = |id: &str| match f.get(id).map(|x| &x.kind) {
            Some(Kind::Select { idx, .. }) => *idx,
            _ => 0,
        };
        let text = |id: &str| match f.get(id).map(|x| &x.kind) {
            Some(Kind::Text(s)) => s.clone(),
            _ => String::new(),
        };
        let sort = match (idx("sort"), idx("partition")) {
            (0, 0) => None,
            (0, _) => return Err(("partition", "needs a sort".into())),
            (s, 0) => Some(RoutingSort::By(SORTS[s].to_string())),
            (s, p) => Some(RoutingSort::Detailed {
                by: SORTS[s].to_string(),
                partition: Some(PARTITIONS[p].to_string()),
            }),
        };
        let number = |id: &'static str| number(&text(id)).map_err(|e| (id, e));
        let price = RoutingMaxPrice {
            prompt: number("max_prompt")?,
            completion: number("max_completion")?,
            request: number("max_request")?,
            image: number("max_image")?,
        };
        Ok(RoutingConfig {
            sort,
            quantizations: list(&text("quantizations")),
            order: list(&text("order")),
            allow_fallbacks: flag(idx("allow_fallbacks")),
            only: list(&text("only")),
            ignore: list(&text("ignore")),
            require_parameters: flag(idx("require_parameters")),
            data_collection: (idx("data_collection") > 0)
                .then(|| COLLECTION[idx("data_collection")].to_string()),
            zdr: flag(idx("zdr")),
            enforce_distillable_text: flag(idx("enforce_distillable_text")),
            preferred_min_throughput: parse_percentiles(&text("min_throughput"))
                .map_err(|e| ("min_throughput", e))?,
            preferred_max_latency: parse_percentiles(&text("max_latency"))
                .map_err(|e| ("max_latency", e))?,
            max_price: (price != RoutingMaxPrice::default()).then_some(price),
        })
    }

    /// Save: the table on the connection, or nothing when every rule is
    /// OpenRouter's. A field that cannot be read is marked and focused.
    fn save(&mut self) -> Option<Action> {
        match self.to_config() {
            Ok(routing) => {
                self.form.dirty = false;
                Some(Action::SetRouting {
                    connection: self.connection.clone(),
                    routing: (routing != RoutingConfig::default()).then(|| Box::new(routing)),
                })
            }
            Err((id, why)) => {
                if let Some(at) = self.form.fields.iter().position(|f| f.id == id) {
                    self.form.selected = at;
                    self.form.fields[at].error = Some(why);
                }
                None
            }
        }
    }

    /// The row the field at `index` is drawn on.
    fn row_of(&self, index: usize) -> usize {
        let before: usize = self
            .form
            .fields
            .iter()
            .take(index)
            .enumerate()
            .map(|(i, f)| match f.kind {
                Kind::Header => 1 + usize::from(i > 0),
                _ => 1 + usize::from(f.error.is_some()),
            })
            .sum();
        let own = self
            .form
            .fields
            .get(index)
            .is_some_and(|f| matches!(f.kind, Kind::Header) && index > 0);
        before + usize::from(own)
    }
}

/// `price` → `cheapest host`, for the connection's row and the chat line.
pub fn summary(r: &RoutingConfig) -> String {
    match r.sort.as_ref().map(RoutingSort::by) {
        Some("price") => "cheapest host".into(),
        Some("throughput") => "fastest host".into(),
        Some("latency") => "soonest host".into(),
        _ if !r.only.is_empty() => format!("only {}", r.only.join(", ")),
        _ if !r.order.is_empty() => format!("{} first", r.order.join(", ")),
        _ => "host rules".into(),
    }
}

impl Panel for Routing {
    fn kind(&self) -> &'static str {
        "routing"
    }

    fn title(&self, _view: &View) -> String {
        format!("routing · {}", self.connection)
    }

    fn status(&self, _view: &View) -> String {
        if self.form.dirty {
            "edited".into()
        } else {
            String::new()
        }
    }

    fn legend(&self, _view: &View) -> String {
        if self.confirm_discard {
            "⏎ save · n discard · esc back".into()
        } else if self.editing {
            "type · enter set · esc back".into()
        } else {
            "↑↓ field · ←→ change · enter edit · ^s save · esc".into()
        }
    }

    fn input(&self, _view: &View) -> Option<String> {
        self.editing.then(|| {
            self.form
                .current()
                .map(|f| f.label.clone())
                .unwrap_or_else(|| "value".into())
        })
    }

    fn prefill(&self, _view: &View) -> Option<String> {
        self.editing
            .then(|| self.form.current().map(Field::value_text))
            .flatten()
    }

    fn size(&self, _view: &View) -> (u16, u16) {
        (72, 26)
    }

    fn render(&self, _view: &View, width: u16, height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let mut rows = self.form.render(w, theme, true);
        // What the focused rule does, under it.
        let note = self.form.current().map(|f| note_for(f.id)).unwrap_or("");
        let mut sel_row = self.row_of(self.form.selected);
        if !note.is_empty() {
            let after =
                (sel_row + 1 + usize::from(self.form.current().is_some_and(|f| f.error.is_some())))
                    .min(rows.len());
            let wrapped = crate::chat::wrap::wrap_plain(note, w.saturating_sub(4));
            let n = wrapped.len();
            rows.splice(
                after..after,
                wrapped
                    .into_iter()
                    .map(|l| Line::from(Span::styled(format!("    {l}"), theme.panel_muted()))),
            );
            // Aim the window at the field and its note together.
            sel_row += n / 2;
        }
        let total = rows.len();
        let h = usize::from(height).max(1);
        let first = super::window(sel_row, total, h).min(self.row_of(self.form.selected));
        let mut lines: Vec<_> = rows.into_iter().skip(first).take(h).collect();
        if self.confirm_discard {
            lines = super::widgets::save_prompt(lines, w, h, theme);
        }
        Body {
            lines,
            scroll: (total > h).then_some((first, total)),
        }
    }

    fn key(&mut self, key: KeyEvent, view: &mut View) -> Outcome {
        if self.confirm_discard {
            return match key.code {
                KeyCode::Char('y' | 'Y') | KeyCode::Enter => {
                    self.confirm_discard = false;
                    match self.save() {
                        Some(a) => Outcome::CloseAct(a),
                        None => Outcome::Stay,
                    }
                }
                KeyCode::Char('n' | 'N') => Outcome::Close,
                KeyCode::Esc => {
                    self.confirm_discard = false;
                    Outcome::Stay
                }
                _ => Outcome::Stay,
            };
        }
        if self.editing {
            return match key.code {
                KeyCode::Esc => {
                    self.editing = false;
                    view.composer.clear();
                    Outcome::Stay
                }
                KeyCode::Enter => {
                    let text = view.composer.text().to_string();
                    if self.form.set_from_text(&text) {
                        self.editing = false;
                        view.composer.clear();
                        if let Some(f) = self.form.current_mut() {
                            f.origin = if text.trim().is_empty() {
                                "default".into()
                            } else {
                                "set".into()
                            };
                        }
                    }
                    Outcome::Stay
                }
                _ => {
                    super::edit_field(&mut view.composer, key);
                    Outcome::Stay
                }
            };
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => {
                if self.form.dirty {
                    self.confirm_discard = true;
                    Outcome::Stay
                } else {
                    Outcome::Close
                }
            }
            KeyCode::Char('s') if ctrl => match self.save() {
                Some(a) => Outcome::Act(a),
                None => Outcome::Stay,
            },
            KeyCode::Up | KeyCode::BackTab => {
                self.form.step(-1);
                Outcome::Stay
            }
            KeyCode::Down | KeyCode::Tab => {
                self.form.step(1);
                Outcome::Stay
            }
            KeyCode::Left => {
                self.adjust(-1);
                Outcome::Stay
            }
            KeyCode::Right | KeyCode::Char(' ') => {
                self.adjust(1);
                Outcome::Stay
            }
            KeyCode::Enter => {
                match self.form.current().map(|f| &f.kind) {
                    Some(Kind::Text(s)) => {
                        view.composer.set_text(s);
                        self.editing = true;
                    }
                    Some(Kind::Select { .. }) => self.adjust(1),
                    Some(Kind::Action) => {
                        let selected = self.form.selected;
                        self.form = Form::new(Self::fields(RoutingConfig::default()));
                        self.form.selected = selected;
                        self.form.dirty = true;
                    }
                    _ => {}
                }
                Outcome::Stay
            }
            _ => Outcome::Stay,
        }
    }

    fn box_clone(&self) -> Box<dyn Panel> {
        Box::new(self.clone())
    }
}

impl Routing {
    /// `←`/`→` on a choice, and its origin note with it.
    fn adjust(&mut self, delta: i32) {
        if self.form.adjust(delta) {
            if let Some(f) = self.form.current_mut() {
                if let Kind::Select { idx, .. } = &f.kind {
                    f.origin = if *idx == 0 {
                        "default".into()
                    } else {
                        "set".into()
                    };
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    fn ctrl_s() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)
    }

    fn body(p: &Routing, v: &View) -> String {
        p.render(v, 72, 40, Theme::truecolor_dark())
            .lines
            .iter()
            .map(|l| l.to_string() + "\n")
            .collect()
    }

    fn full() -> RoutingConfig {
        RoutingConfig {
            sort: Some(RoutingSort::Detailed {
                by: "price".into(),
                partition: Some("none".into()),
            }),
            quantizations: vec!["fp8".into(), "bf16".into()],
            order: vec!["baseten/fp8/us".into()],
            allow_fallbacks: Some(false),
            only: vec![],
            ignore: vec!["wafer".into()],
            require_parameters: Some(true),
            data_collection: Some("deny".into()),
            zdr: Some(true),
            enforce_distillable_text: None,
            preferred_min_throughput: Some(RoutingPercentiles::Median(80.0)),
            preferred_max_latency: Some(RoutingPercentiles::Percentiles {
                p50: Some(2.0),
                p75: None,
                p90: Some(5.0),
                p99: None,
            }),
            max_price: Some(RoutingMaxPrice {
                prompt: Some(0.5),
                completion: Some(2.0),
                request: None,
                image: None,
            }),
        }
    }

    /// Every field of OpenRouter's object round-trips through the form:
    /// what was set is shown, and saving gives the same table back.
    #[test]
    fn every_rule_round_trips_through_the_form() {
        let v = View::new("openrouter".into(), "m".into(), "~".into());
        let p = Routing::new("openrouter", Some(&full()));
        let text = body(&p, &v);
        for want in [
            "‹ price ›",
            "‹ none ›",
            "baseten/fp8/us",
            "fp8, bf16",
            "wafer",
            "‹ deny ›",
            "80",
            "p50=2 p90=5",
            "0.5",
        ] {
            assert!(text.contains(want), "{want}\n{text}");
        }
        assert_eq!(p.to_config(), Ok(full()));
        // Nothing set is no table at all, not an empty one.
        let mut p = Routing::new("openrouter", None);
        assert!(matches!(
            p.key(ctrl_s(), &mut v.clone()),
            Outcome::Act(Action::SetRouting { ref connection, routing: None }) if connection == "openrouter"
        ));
    }

    /// Typed values are read as OpenRouter needs them: lists on commas or
    /// spaces, a number or percentiles by name, a price not below zero. A
    /// value that cannot be read marks its field, keeps the save, and is
    /// focused; a partition without a sort is one such.
    #[test]
    fn a_value_that_cannot_be_read_is_marked_and_keeps_the_save() {
        let mut v = View::new("openrouter".into(), "m".into(), "~".into());
        let mut p = Routing::new("openrouter", None);
        let set = |p: &mut Routing, v: &mut View, id: &str, text: &str| {
            let at = p.form.fields.iter().position(|f| f.id == id).unwrap();
            p.form.selected = at;
            p.key(key(KeyCode::Enter), v);
            assert!(p.editing, "{id} is not a text field");
            v.composer.set_text(text);
            p.key(key(KeyCode::Enter), v);
            assert!(!p.editing, "{id}: {text:?} not taken");
        };
        set(&mut p, &mut v, "quantizations", "fp8 bf16, fp16");
        set(&mut p, &mut v, "max_latency", "p50=2, p90=5");
        set(&mut p, &mut v, "max_prompt", "$0.5");
        set(&mut p, &mut v, "max_completion", "-1");
        assert!(matches!(p.key(ctrl_s(), &mut v), Outcome::Stay));
        let f = p.form.current().unwrap();
        assert_eq!(f.id, "max_completion");
        assert_eq!(f.error.as_deref(), Some("not below zero"));
        assert!(body(&p, &v).contains("✕ not below zero"));
        set(&mut p, &mut v, "max_completion", "2");
        set(&mut p, &mut v, "min_throughput", "p40=9");
        assert!(matches!(p.key(ctrl_s(), &mut v), Outcome::Stay));
        assert_eq!(p.form.current().unwrap().id, "min_throughput");
        set(&mut p, &mut v, "min_throughput", "");
        // A partition with no sort has nothing to apply to.
        let at = p
            .form
            .fields
            .iter()
            .position(|f| f.id == "partition")
            .unwrap();
        p.form.selected = at;
        p.key(key(KeyCode::Right), &mut v);
        assert!(matches!(p.key(ctrl_s(), &mut v), Outcome::Stay));
        assert_eq!(p.form.current().unwrap().id, "partition");
        p.key(key(KeyCode::Left), &mut v);
        let Outcome::Act(Action::SetRouting { routing, .. }) = p.key(ctrl_s(), &mut v) else {
            panic!("not saved");
        };
        let r = *routing.unwrap();
        assert_eq!(r.quantizations, ["fp8", "bf16", "fp16"]);
        assert_eq!(
            r.preferred_max_latency,
            Some(RoutingPercentiles::Percentiles {
                p50: Some(2.0),
                p75: None,
                p90: Some(5.0),
                p99: None
            })
        );
        assert_eq!(
            r.max_price,
            Some(RoutingMaxPrice {
                prompt: Some(0.5),
                completion: Some(2.0),
                request: None,
                image: None
            })
        );
        assert_eq!(r.sort, None);
        assert!(!p.form.dirty);
    }

    /// `Esc` with edits asks; `n` drops them, Enter saves them. The clear
    /// row puts every rule back and counts as an edit. The note under the
    /// focused field says what it does.
    #[test]
    fn esc_with_edits_asks_and_clear_puts_every_rule_back() {
        let mut v = View::new("openrouter".into(), "m".into(), "~".into());
        let mut p = Routing::new("openrouter", Some(&full()));
        assert!(matches!(p.key(key(KeyCode::Esc), &mut v), Outcome::Close));
        assert!(body(&p, &v).contains("cheapest host") || body(&p, &v).contains("load balancing"));
        p.key(key(KeyCode::Right), &mut v);
        assert!(p.form.dirty);
        assert!(matches!(p.key(key(KeyCode::Esc), &mut v), Outcome::Stay));
        assert!(p.confirm_discard);
        assert!(matches!(
            p.key(key(KeyCode::Char('n')), &mut v),
            Outcome::Close
        ));
        let mut p = Routing::new("openrouter", Some(&full()));
        p.key(key(KeyCode::Right), &mut v);
        p.key(key(KeyCode::Esc), &mut v);
        assert!(matches!(
            p.key(key(KeyCode::Enter), &mut v),
            Outcome::CloseAct(Action::SetRouting {
                routing: Some(_),
                ..
            })
        ));
        // Clear: the last row.
        let mut p = Routing::new("openrouter", Some(&full()));
        p.form.step(-1);
        assert_eq!(p.form.current().unwrap().id, "clear");
        p.key(key(KeyCode::Enter), &mut v);
        assert!(p.form.dirty);
        assert_eq!(p.to_config(), Ok(RoutingConfig::default()));
        assert!(matches!(
            p.key(ctrl_s(), &mut v),
            Outcome::Act(Action::SetRouting { routing: None, .. })
        ));
        assert_eq!(summary(&full()), "cheapest host");
        assert_eq!(
            summary(&RoutingConfig {
                only: vec!["baseten".into()],
                ..Default::default()
            }),
            "only baseten"
        );
    }
}
