//! Crew builder — choose every seat with a recommendation beside it, set a
//! budget sized to the job, and test each model before saving. Opens on first
//! launch when no crew is configured, and from `/crew`.

use std::collections::{BTreeMap, HashMap, HashSet};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::text::Line;
use ryter_core::ModelInfo;
use ryter_core::estimate::{CrewRates, JobSize, Rates, estimate};
use ryter_core::tiering::{self, Pick, Tier};
use ryter_core::{RoleModel, format_usd};

use super::{Body, Notice, Outcome, Panel, widgets};
use crate::action::Action;
use crate::chat::wrap;
use crate::theme::Theme;
use crate::view::View;

/// Seats in the order the builder asks for them.
pub const SEATS: [&str; 4] = ["lead", "architect", "builder", "auditor"];

/// Why each seat matters, in words.
fn why(seat: &str) -> &'static str {
    match seat {
        "lead" => {
            "talks with you and turns requests into tasks, on every message. it reads more \
             than it writes, so a capable budget model is usually enough."
        }
        "architect" => {
            "designs larger changes, a few calls per request. its plan is what every builder \
             follows, so a strong model pays for itself here."
        }
        "builder" => {
            "writes the code, and burns most of the tokens. the cheapest model that can use \
             tools well saves the most; several run at once."
        }
        _ => {
            "reviews every change before it lands. it must be a different model from the lead \
             and the builder, ideally from another vendor, so its sign-off is a second opinion."
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// Pick a starting point.
    Start,
    /// Choose a seat (index into [`SEATS`]).
    Seat(usize),
    /// Job size and budget.
    Budget,
    /// Test and save.
    Review,
}

impl Step {
    fn number(self) -> usize {
        match self {
            Step::Start => 1,
            Step::Seat(i) => 2 + i,
            Step::Budget => 6,
            Step::Review => 7,
        }
    }
}

const STEPS: usize = 7;

/// Where a model's test stands.
#[derive(Debug, Clone, PartialEq)]
enum Probe {
    Running,
    Ok,
    Failed(String),
}

/// One chosen seat.
#[derive(Debug, Clone, PartialEq)]
struct Seat {
    connection: String,
    model: String,
}

impl Seat {
    fn key(&self) -> (String, String) {
        (self.connection.clone(), self.model.clone())
    }
}

/// The crew builder.
#[derive(Debug, Clone)]
pub struct CrewBuilder {
    step: Step,
    models: Vec<ModelInfo>,
    loading: bool,
    tier: Tier,
    draft: [Option<Seat>; 4],
    /// Selection within the current step's rows.
    sel: usize,
    size: JobSize,
    budget_on: bool,
    budget: f64,
    /// The user moved the budget, so a size change no longer resets it.
    budget_touched: bool,
    probes: HashMap<(String, String), Probe>,
    /// Opened by `/crew` with no crew yet: say what crew mode needs, and let
    /// `esc` mean "stay in solo mode".
    first_run: bool,
}

impl CrewBuilder {
    /// Start from the current crew (lead included).
    pub fn new(view: &View, first_run: bool) -> Self {
        let lead = Seat {
            connection: view.connection.clone(),
            model: view.model.clone(),
        };
        let seat = |role: &str| {
            view.specialists
                .get(role)
                .and_then(|r| {
                    Some(Seat {
                        connection: r.connection.clone()?,
                        model: r.model.clone()?,
                    })
                })
                .unwrap_or_else(|| lead.clone())
        };
        Self {
            step: Step::Start,
            models: Vec::new(),
            loading: true,
            tier: Tier::Schooner,
            draft: [
                Some(lead.clone()),
                Some(seat("architect")),
                Some(seat("builder")),
                Some(seat("auditor")),
            ],
            sel: 1,
            size: JobSize::Medium,
            // Off unless the user already has one: a budget is their call.
            // The suggested cap is shown either way, one toggle away.
            budget_on: view.budget_usd > 0.0,
            budget: if view.budget_usd > 0.0 {
                view.budget_usd
            } else {
                5.0
            },
            budget_touched: false,
            probes: HashMap::new(),
            first_run,
        }
    }

    fn local(view: &View) -> HashSet<String> {
        view.connections
            .iter()
            .filter(|c| c.kind == "local")
            .map(|c| c.name.clone())
            .collect()
    }

    fn seat(&self, i: usize) -> Option<&Seat> {
        self.draft[i].as_ref()
    }

    /// Recommendations for the current tier, given the seats chosen so far.
    fn recommendations(&self, view: &View) -> [Option<Pick>; 4] {
        let local = Self::local(view);
        let lead = self.seat(0).map(Seat::key).unwrap_or_default();
        let base = tiering::suggest_tier(self.tier, &lead.0, &lead.1, &self.models, &local);
        let lead_rec = tiering::recommend_lead(self.tier, &base);
        let builder = self
            .seat(2)
            .map(Seat::key)
            .or_else(|| {
                base.builder
                    .as_ref()
                    .map(|p| (p.connection.clone(), p.model.clone()))
            })
            .unwrap_or_default();
        // The auditor must be independent of the seats actually chosen.
        let t = tiering::suggest_for(
            self.tier,
            (&lead.0, &lead.1),
            (&builder.0, &builder.1),
            &self.models,
            &local,
        );
        [lead_rec, base.architect, base.builder, t.auditor]
    }

    /// Fill every seat from the tier's recommendations.
    fn fill_from_tier(&mut self, view: &View) {
        let local = Self::local(view);
        let lead_now = self.seat(0).map(Seat::key).unwrap_or_default();
        let base = tiering::suggest_tier(self.tier, &lead_now.0, &lead_now.1, &self.models, &local);
        let to_seat = |p: &Option<Pick>| {
            p.as_ref().map(|p| Seat {
                connection: p.connection.clone(),
                model: p.model.clone(),
            })
        };
        if let Some(l) = to_seat(&tiering::recommend_lead(self.tier, &base)) {
            self.draft[0] = Some(l);
        }
        self.draft[1] = to_seat(&base.architect).or(self.draft[1].take());
        self.draft[2] = to_seat(&base.builder).or(self.draft[2].take());
        let recs = self.recommendations(view);
        self.draft[3] = to_seat(&recs[3]).or(self.draft[3].take());
    }

    fn info(&self, seat: &Seat) -> Option<&ModelInfo> {
        self.models.iter().find(|m| {
            m.id == seat.model && m.connection.as_deref().is_none_or(|c| c == seat.connection)
        })
    }

    fn rates(&self, i: usize) -> Option<Rates> {
        let m = self.info(self.seat(i)?)?;
        Some(Rates {
            input: m.input_per_million?,
            output: m.output_per_million?,
        })
    }

    fn crew_rates(&self, view: &View) -> CrewRates {
        let local = Self::local(view);
        let r = |i: usize| {
            if self.seat(i).is_some_and(|s| local.contains(&s.connection)) {
                Some(Rates {
                    input: 0.0,
                    output: 0.0,
                })
            } else {
                self.rates(i)
            }
        };
        CrewRates {
            lead: r(0),
            architect: r(1),
            builder: r(2),
            auditor: r(3),
        }
    }

    /// Why the auditor can't take `m`, if it can't.
    fn auditor_conflict(&self, model: &str) -> Option<&'static str> {
        if self
            .seat(0)
            .is_some_and(|s| ryter_core::crew::same_model(&s.model, model))
        {
            Some("same as the lead")
        } else if self
            .seat(2)
            .is_some_and(|s| ryter_core::crew::same_model(&s.model, model))
        {
            Some("same as the builder")
        } else {
            None
        }
    }

    /// Catalog rows for a seat step, filtered by what's typed.
    fn seat_rows(&self, view: &View, i: usize) -> Vec<&ModelInfo> {
        let f = view.composer.text().trim().to_ascii_lowercase();
        let mut rows: Vec<&ModelInfo> = self
            .models
            .iter()
            .filter(|m| m.tools != Some(false))
            // A seat needs a model with a price and dependable tool use:
            // OpenRouter's routers (openrouter/auto, …, listed at a price of
            // -1) pick a model per request. They led the list as "cheapest".
            .filter(|m| !m.id.starts_with("openrouter/"))
            .filter(|m| {
                [m.input_per_million, m.output_per_million]
                    .iter()
                    .all(|p| p.is_none_or(|p| p >= 0.0))
            })
            .filter(|m| f.is_empty() || m.id.to_ascii_lowercase().contains(&f))
            .collect();
        let blended = |m: &ModelInfo| match (m.input_per_million, m.output_per_million) {
            (Some(i), Some(o)) => 0.8 * i + 0.2 * o,
            _ => f64::MAX,
        };
        // Budget seats cheapest first; strong seats strongest first.
        if SEATS[i] == "lead" || SEATS[i] == "builder" {
            rows.sort_by(|a, b| blended(a).total_cmp(&blended(b)));
        } else {
            rows.sort_by(|a, b| {
                let (x, y) = (blended(a), blended(b));
                let (x, y) = (
                    if x == f64::MAX { -1.0 } else { x },
                    if y == f64::MAX { -1.0 } else { y },
                );
                y.total_cmp(&x)
            });
        }
        rows
    }

    /// Everything that must be fixed before saving.
    fn problems(&self) -> Vec<String> {
        let mut v = Vec::new();
        for (i, s) in self.draft.iter().enumerate() {
            if s.is_none() {
                v.push(format!("choose a {}", SEATS[i]));
            }
        }
        if let Some(a) = self.seat(3) {
            if let Some(why) = self.auditor_conflict(&a.model) {
                v.push(format!(
                    "the auditor is the {}",
                    why.trim_start_matches("same as the ")
                ));
            }
        }
        for s in self.draft.iter().flatten() {
            if let Some(Probe::Failed(why)) = self.probes.get(&s.key()) {
                let p = format!("{}: {why}", s.model);
                // One model in two seats fails once, not twice.
                if !v.contains(&p) {
                    v.push(p);
                }
            }
        }
        v
    }

    fn all_tested(&self) -> bool {
        self.draft
            .iter()
            .flatten()
            .all(|s| self.probes.get(&s.key()) == Some(&Probe::Ok))
    }

    fn go(&mut self, step: Step, view: &mut View) {
        self.step = step;
        self.sel = 0;
        view.composer.clear();
        if step == Step::Budget && !self.budget_touched {
            self.budget = estimate(self.crew_rates(view), self.size).budget;
        }
    }

    fn back(&mut self, view: &mut View) -> Outcome {
        let prev = match self.step {
            Step::Start => return Outcome::Close,
            Step::Seat(0) => Step::Start,
            Step::Seat(i) => Step::Seat(i - 1),
            Step::Budget => Step::Seat(3),
            Step::Review => Step::Budget,
        };
        self.go(prev, view);
        Outcome::Stay
    }

    /// The per-task cap to save: never lower than what's set, raised when
    /// this crew's normal task or design would not fit under it.
    fn task_cap(&self, view: &View) -> f64 {
        let e = estimate(self.crew_rates(view), JobSize::Large);
        view.task_budget_usd
            .max(ryter_core::estimate::task_cap_for(&e))
    }

    fn save(&self, view: &View) -> Action {
        let seat = |i: usize| {
            self.seat(i).map(|s| RoleModel {
                connection: Some(s.connection.clone()),
                model: Some(s.model.clone()),
            })
        };
        let mut crew = BTreeMap::new();
        for (i, role) in SEATS.iter().enumerate().skip(1) {
            if let Some(r) = seat(i) {
                crew.insert((*role).to_string(), r);
            }
        }
        let lead = self.seat(0).map(Seat::key).unwrap_or_default();
        Action::SaveCrewSetup {
            lead_connection: lead.0,
            lead_model: lead.1,
            crew,
            budget: if self.budget_on { self.budget } else { 0.0 },
            task_cap: self.task_cap(view),
        }
    }

    fn price_label(m: Option<&ModelInfo>, local: bool) -> String {
        if local {
            return "local, $0".into();
        }
        match m.and_then(|m| Some((m.input_per_million?, m.output_per_million?))) {
            Some((i, o)) => ryter_core::format_rates_short(i, o),
            None => "price ?".into(),
        }
    }
}

impl Panel for CrewBuilder {
    fn kind(&self) -> &'static str {
        "crew-builder"
    }

    fn title(&self, _view: &View) -> String {
        "crew builder".into()
    }

    fn status(&self, _view: &View) -> String {
        format!("{} of {STEPS}", self.step.number())
    }

    fn legend(&self, _view: &View) -> String {
        match self.step {
            Step::Start if self.first_run => {
                "↑↓ move · enter choose · esc stay in solo mode".into()
            }
            Step::Start => "↑↓ move · enter choose · esc close".into(),
            Step::Seat(_) => {
                "type to filter · ↑↓ move · tab reasoning · enter choose · esc back".into()
            }
            Step::Budget => "↑↓ move · ←→ change · enter next · esc back".into(),
            Step::Review if self.all_tested() && self.problems().is_empty() => {
                "enter save · esc back".into()
            }
            Step::Review => "enter test each model (under 1¢) · esc back".into(),
        }
    }

    fn input(&self, _view: &View) -> Option<String> {
        matches!(self.step, Step::Seat(_)).then(|| "filter models".into())
    }

    fn size(&self, _view: &View) -> (u16, u16) {
        (84, 26)
    }

    fn render(&self, view: &View, width: u16, height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let h = usize::from(height).max(1);
        let mut lines: Vec<Line<'static>> = Vec::new();
        let local = Self::local(view);
        let prose = |lines: &mut Vec<Line<'static>>, text: &str| {
            for l in wrap::wrap_plain(text, w.saturating_sub(3)) {
                lines.push(widgets::note(&format!(" {l}"), theme));
            }
        };
        match self.step {
            Step::Start => {
                lines.push(widgets::step_line(1, STEPS, "starting point", theme));
                if self.first_run {
                    prose(
                        &mut lines,
                        "crew mode needs a crew: a lead you talk to, an architect who designs, \
                         builders who work in parallel, and an auditor who signs off. pick a \
                         starting point; you choose every seat next, with a recommendation \
                         beside each. when it's saved, you're in crew mode.",
                    );
                } else {
                    prose(
                        &mut lines,
                        "pick a starting point. you choose every seat next, with a \
                         recommendation beside each.",
                    );
                }
                lines.push(widgets::blank(theme));
                for (i, tier) in Tier::ALL.iter().enumerate() {
                    lines.push(widgets::list_row(
                        "▸",
                        tier.name(),
                        tier.cost(),
                        "",
                        self.sel == i,
                        w,
                        theme,
                        Some(theme.accent),
                    ));
                    for l in wrap::wrap_plain(tier.tagline(), w.saturating_sub(6)) {
                        lines.push(widgets::note(&format!("    {l}"), theme));
                    }
                }
                lines.push(widgets::list_row(
                    "◇",
                    "my current crew",
                    "start from what is set now",
                    "",
                    self.sel == Tier::ALL.len(),
                    w,
                    theme,
                    None,
                ));
                if self.loading {
                    lines.push(widgets::blank(theme));
                    lines.push(widgets::note("  reading the models you can reach…", theme));
                } else if self.models.is_empty() {
                    lines.push(widgets::blank(theme));
                    prose(
                        &mut lines,
                        "no models could be listed. add a key in /provider, then come back \
                         with /crew → crew builder.",
                    );
                }
            }
            Step::Seat(i) => {
                let role = SEATS[i];
                lines.push(widgets::step_line(self.step.number(), STEPS, role, theme));
                prose(&mut lines, why(role));
                lines.push(widgets::blank(theme));
                let rec = self.recommendations(view)[i].clone();
                let rows = self.seat_rows(view, i);
                let fixed = 1;
                let n = rows.len() + fixed;
                let list_h = h.saturating_sub(lines.len() + 1).max(3);
                let first = super::window(self.sel, n, list_h);
                let mut body: Vec<Line<'static>> = Vec::new();
                let rec_label = rec
                    .as_ref()
                    .map(|p| {
                        format!(
                            "{} · {}",
                            p.model,
                            p.label()
                                .rsplit(" (")
                                .next()
                                .unwrap_or("")
                                .trim_end_matches(')')
                        )
                    })
                    .unwrap_or_else(|| "(none qualifies)".into());
                let rec_status = match &rec {
                    Some(p) => format!("{} · {}", view.reasoning_label(&p.model), self.tier.name()),
                    None => self.tier.name().to_string(),
                };
                body.push(widgets::list_row(
                    "★",
                    "recommended",
                    &rec_label,
                    &rec_status,
                    self.sel == 0,
                    w,
                    theme,
                    Some(theme.success),
                ));
                for (k, m) in rows.iter().enumerate() {
                    let conn = m.connection.clone().unwrap_or_default();
                    let chosen = self
                        .seat(i)
                        .is_some_and(|s| s.model == m.id && s.connection == conn);
                    let conflict = (role == "auditor")
                        .then(|| self.auditor_conflict(&m.id))
                        .flatten();
                    let status = match (chosen, conflict) {
                        (_, Some(c)) => c.to_string(),
                        (true, None) => format!("{} · chosen", view.reasoning_label(&m.id)),
                        _ => format!("{} · {conn}", view.reasoning_label(&m.id)),
                    };
                    body.push(widgets::list_row(
                        if chosen { "●" } else { " " },
                        &m.id,
                        &Self::price_label(Some(m), local.contains(&conn)),
                        &status,
                        self.sel == k + fixed,
                        w,
                        theme,
                        conflict.map(|_| theme.dim),
                    ));
                }
                lines.extend(body.into_iter().skip(first).take(list_h));
            }
            Step::Budget => {
                lines.push(widgets::step_line(6, STEPS, "budget", theme));
                prose(
                    &mut lines,
                    "how big is the work you're starting? the estimate uses your crew's prices \
                     and token use measured on real runs. it is rough: a larger job than you \
                     think is the usual surprise.",
                );
                lines.push(widgets::blank(theme));
                let rates = self.crew_rates(view);
                for (k, size) in JobSize::ALL.iter().enumerate() {
                    let e = estimate(rates, *size);
                    lines.push(widgets::list_row(
                        if *size == self.size { "●" } else { "○" },
                        size.name(),
                        size.describe(),
                        &format!("~{}", format_usd(Some(e.total))),
                        self.sel == k,
                        w,
                        theme,
                        None,
                    ));
                }
                let e = estimate(rates, self.size);
                lines.push(widgets::blank(theme));
                lines.push(widgets::note(
                    &format!(
                        "  each builder task ~{} · each design ~{} · this job ~{}",
                        format_usd(Some(e.per_task)),
                        format_usd(Some(e.per_design)),
                        format_usd(Some(e.total))
                    ),
                    theme,
                ));
                if e.partial {
                    lines.push(widgets::colored(
                        "  a model's price is unknown, so the estimate leaves it out",
                        theme.warn,
                        theme,
                    ));
                }
                lines.push(widgets::blank(theme));
                let row = JobSize::ALL.len();
                lines.push(widgets::list_row(
                    if self.budget_on { "●" } else { "○" },
                    "session budget",
                    if self.budget_on {
                        "on · the crew stops at the cap and says what finished"
                    } else {
                        "off · nothing stops on cost; watch the spend card"
                    },
                    "",
                    self.sel == row,
                    w,
                    theme,
                    None,
                ));
                lines.push(widgets::list_row(
                    " ",
                    "cap",
                    &format!(
                        "‹ {} ›{}",
                        format_usd(Some(self.budget)),
                        if (self.budget - e.budget).abs() < 0.01 {
                            "  suggested"
                        } else {
                            ""
                        }
                    ),
                    &format!("suggested {}", format_usd(Some(e.budget))),
                    self.sel == row + 1,
                    w,
                    theme,
                    (!self.budget_on).then_some(theme.dim),
                ));
            }
            Step::Review => {
                lines.push(widgets::step_line(7, STEPS, "review", theme));
                lines.push(widgets::blank(theme));
                for (i, role) in SEATS.iter().enumerate() {
                    let (label, status, color) = match self.seat(i) {
                        Some(s) => {
                            let price =
                                Self::price_label(self.info(s), local.contains(&s.connection));
                            let (st, c) = match self.probes.get(&s.key()) {
                                Some(Probe::Ok) => ("✓ answered".to_string(), theme.success),
                                Some(Probe::Running) => ("testing…".to_string(), theme.dim),
                                Some(Probe::Failed(_)) => ("✗ failed".to_string(), theme.error),
                                None => ("not tested".to_string(), theme.dim),
                            };
                            (
                                format!(
                                    "{} on {} · {price} · reasoning {}",
                                    s.model,
                                    s.connection,
                                    view.reasoning_label(&s.model)
                                ),
                                st,
                                Some(c),
                            )
                        }
                        None => ("(not chosen)".into(), String::new(), None),
                    };
                    lines.push(widgets::list_row(
                        "●", role, &label, &status, false, w, theme, color,
                    ));
                }
                lines.push(widgets::blank(theme));
                let cap = self.task_cap(view);
                lines.push(widgets::note(
                    &format!(
                        "  each task stops at {}{}",
                        format_usd(Some(cap)),
                        if cap > view.task_budget_usd + 0.001 {
                            format!(
                                " (raised from {} so this crew's designs fit)",
                                format_usd(Some(view.task_budget_usd))
                            )
                        } else {
                            String::new()
                        }
                    ),
                    theme,
                ));
                lines.push(widgets::note(
                    &format!(
                        "  budget: {} · {} job, ~{} estimated",
                        if self.budget_on {
                            format_usd(Some(self.budget))
                        } else {
                            "off".into()
                        },
                        self.size.name(),
                        format_usd(Some(estimate(self.crew_rates(view), self.size).total))
                    ),
                    theme,
                ));
                let problems = self.problems();
                lines.push(widgets::blank(theme));
                if problems.is_empty() && self.all_tested() {
                    lines.push(widgets::colored(
                        "  every model answered. enter saves the crew, the lead, and the budget.",
                        theme.success,
                        theme,
                    ));
                } else if problems.is_empty() {
                    prose(
                        &mut lines,
                        "enter sends each model one tiny request with a tool, to catch what a \
                         catalog can't: a data policy (zero data retention) that leaves it no \
                         provider, no tool support, no access, or no credits.",
                    );
                } else {
                    for p in problems {
                        for (k, l) in wrap::wrap_plain(&p, w.saturating_sub(6)).iter().enumerate() {
                            let mark = if k == 0 { "✗" } else { " " };
                            lines.push(widgets::colored(
                                &format!("  {mark} {l}"),
                                theme.error,
                                theme,
                            ));
                        }
                    }
                    lines.push(widgets::note("  esc to go back and change a seat", theme));
                }
            }
        }
        lines.truncate(h);
        Body {
            lines,
            scroll: None,
        }
    }

    fn key(&mut self, key: KeyEvent, view: &mut View) -> Outcome {
        match self.step {
            Step::Start => {
                let n = Tier::ALL.len() + 1;
                match key.code {
                    KeyCode::Esc => Outcome::Close,
                    KeyCode::Up => {
                        self.sel = super::step(self.sel, -1, n);
                        Outcome::Stay
                    }
                    KeyCode::Down => {
                        self.sel = super::step(self.sel, 1, n);
                        Outcome::Stay
                    }
                    KeyCode::Enter if !self.loading => {
                        if let Some(&tier) = Tier::ALL.get(self.sel) {
                            self.tier = tier;
                            self.fill_from_tier(view);
                        }
                        self.go(Step::Seat(0), view);
                        Outcome::Stay
                    }
                    _ => Outcome::Stay,
                }
            }
            Step::Seat(i) => {
                let n = self.seat_rows(view, i).len() + 1;
                match key.code {
                    KeyCode::Esc => self.back(view),
                    KeyCode::Up => {
                        self.sel = super::step(self.sel, -1, n);
                        Outcome::Stay
                    }
                    KeyCode::Down => {
                        self.sel = super::step(self.sel, 1, n);
                        Outcome::Stay
                    }
                    KeyCode::Enter => {
                        let chosen = if self.sel == 0 {
                            self.recommendations(view)[i].clone().map(|p| Seat {
                                connection: p.connection,
                                model: p.model,
                            })
                        } else {
                            self.seat_rows(view, i).get(self.sel - 1).and_then(|m| {
                                let bad =
                                    SEATS[i] == "auditor" && self.auditor_conflict(&m.id).is_some();
                                (!bad).then(|| Seat {
                                    connection: m.connection.clone().unwrap_or_default(),
                                    model: m.id.clone(),
                                })
                            })
                        };
                        let Some(seat) = chosen else {
                            return Outcome::Stay;
                        };
                        self.draft[i] = Some(seat);
                        let next = if i + 1 < SEATS.len() {
                            Step::Seat(i + 1)
                        } else {
                            Step::Budget
                        };
                        self.go(next, view);
                        // A changed seat may now clash with the auditor; ask again.
                        if i < 3 {
                            if let Some(a) = self.seat(3).cloned() {
                                if self.auditor_conflict(&a.model).is_some() {
                                    self.draft[3] = None;
                                }
                            }
                        }
                        Outcome::Stay
                    }
                    // How hard the highlighted model reasons, wherever it runs.
                    KeyCode::Tab | KeyCode::BackTab => {
                        let model = if self.sel == 0 {
                            self.recommendations(view)[i].clone().map(|p| p.model)
                        } else {
                            self.seat_rows(view, i)
                                .get(self.sel - 1)
                                .map(|m| m.id.clone())
                        };
                        let Some(model) = model else {
                            return Outcome::Stay;
                        };
                        let level = ryter_core::config::cycle_reasoning(
                            view.model_reasoning.get(&model).map(String::as_str),
                            key.code == KeyCode::Tab,
                        );
                        Outcome::Act(Action::SetModelReasoning { model, level })
                    }
                    _ => {
                        if super::edit_field(&mut view.composer, key) {
                            self.sel = 0;
                        }
                        Outcome::Stay
                    }
                }
            }
            Step::Budget => {
                let n = JobSize::ALL.len() + 2;
                let cap_row = JobSize::ALL.len() + 1;
                let step = if self.budget < 5.0 { 0.5 } else { 1.0 };
                match key.code {
                    KeyCode::Esc => self.back(view),
                    KeyCode::Up => {
                        self.sel = super::step(self.sel, -1, n);
                        Outcome::Stay
                    }
                    KeyCode::Down => {
                        self.sel = super::step(self.sel, 1, n);
                        Outcome::Stay
                    }
                    KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') => {
                        let up = key.code != KeyCode::Left;
                        if let Some(&size) = JobSize::ALL.get(self.sel) {
                            self.size = size;
                            if !self.budget_touched {
                                self.budget = estimate(self.crew_rates(view), size).budget;
                            }
                        } else if self.sel == cap_row - 1 {
                            self.budget_on = !self.budget_on;
                        } else if self.sel == cap_row {
                            self.budget = if up {
                                self.budget + step
                            } else {
                                (self.budget - step).max(0.5)
                            };
                            self.budget_touched = true;
                            self.budget_on = true;
                        }
                        Outcome::Stay
                    }
                    KeyCode::Enter => {
                        if let Some(&size) = JobSize::ALL.get(self.sel) {
                            self.size = size;
                            if !self.budget_touched {
                                self.budget = estimate(self.crew_rates(view), size).budget;
                            }
                        }
                        self.go(Step::Review, view);
                        Outcome::Stay
                    }
                    _ => Outcome::Stay,
                }
            }
            Step::Review => match key.code {
                KeyCode::Esc => self.back(view),
                KeyCode::Enter => {
                    let problems = self.problems();
                    if problems.is_empty() && self.all_tested() {
                        return Outcome::CloseAct(self.save(view));
                    }
                    // Test whatever hasn't answered yet (failed ones again too).
                    let pending: Vec<(String, String)> = self
                        .draft
                        .iter()
                        .flatten()
                        .map(Seat::key)
                        .filter(|k| self.probes.get(k) != Some(&Probe::Ok))
                        .collect::<HashSet<_>>()
                        .into_iter()
                        .collect();
                    if pending.is_empty() || self.draft.iter().any(Option::is_none) {
                        return Outcome::Stay;
                    }
                    for k in &pending {
                        self.probes.insert(k.clone(), Probe::Running);
                    }
                    Outcome::Act(Action::ProbeModels(pending))
                }
                _ => Outcome::Stay,
            },
        }
    }

    fn on_notice(&mut self, n: &Notice, _view: &mut View) {
        match n {
            // Lists arrive per connection (the lead's first at startup), so
            // merge rather than keep whichever came first.
            Notice::Models(models) => {
                for m in models {
                    let known = self
                        .models
                        .iter()
                        .any(|x| x.id == m.id && x.connection == m.connection);
                    if !known {
                        self.models.push(m.clone());
                    }
                }
                self.loading = false;
            }
            Notice::Probed(results) => {
                for (conn, model, r) in results {
                    self.probes.insert(
                        (conn.clone(), model.clone()),
                        match r {
                            Ok(()) => Probe::Ok,
                            Err(e) => Probe::Failed(e.clone()),
                        },
                    );
                }
            }
            _ => {}
        }
    }

    fn box_clone(&self) -> Box<dyn Panel> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn m(id: &str, i: f64, o: f64) -> ModelInfo {
        ModelInfo {
            id: id.into(),
            context_length: Some(400_000),
            input_per_million: Some(i),
            output_per_million: Some(o),
            connection: Some("openrouter".into()),
            created: None,
            tools: Some(true),
        }
    }

    fn catalog() -> Vec<ModelInfo> {
        vec![
            m("deepseek/deepseek-v4.1-flash", 0.15, 0.6),
            m("qwen/qwen3.7-max", 1.2, 5.6),
            m("openai/gpt-5.5", 5.0, 30.0),
            m("anthropic/claude-opus-5", 5.0, 25.0),
        ]
    }

    fn setup() -> (View, CrewBuilder) {
        let mut v = View::new(
            ryter_core::Phase::Build,
            "openrouter".into(),
            "deepseek/deepseek-v4.1-flash".into(),
            "/tmp".into(),
        );
        v.budget_usd = 0.0;
        let mut b = CrewBuilder::new(&v, true);
        b.on_notice(&Notice::Models(catalog()), &mut v);
        (v, b)
    }

    fn press(b: &mut CrewBuilder, v: &mut View, code: KeyCode) -> Outcome {
        b.key(KeyEvent::new(code, KeyModifiers::NONE), v)
    }

    /// Start from the schooner, take every recommendation, size the job,
    /// test, and save: the lead, the crew, and the budget arrive together.
    #[test]
    fn recommendations_all_the_way_through() {
        let (mut v, mut b) = setup();
        b.sel = 1; // schooner
        press(&mut b, &mut v, KeyCode::Enter);
        for _ in SEATS {
            press(&mut b, &mut v, KeyCode::Enter); // ★ recommended
        }
        assert_eq!(b.step, Step::Budget);
        // Off by default; choose a large job and switch the budget on.
        assert!(!b.budget_on);
        b.sel = 2;
        press(&mut b, &mut v, KeyCode::Right);
        b.sel = 3;
        press(&mut b, &mut v, KeyCode::Char(' '));
        assert!(b.budget_on);
        press(&mut b, &mut v, KeyCode::Enter);
        assert_eq!(b.step, Step::Review);
        let Outcome::Act(Action::ProbeModels(asked)) = press(&mut b, &mut v, KeyCode::Enter) else {
            panic!("review must test before saving");
        };
        let results = asked
            .iter()
            .map(|(c, m)| (c.clone(), m.clone(), Ok(())))
            .collect();
        b.on_notice(&Notice::Probed(results), &mut v);
        match press(&mut b, &mut v, KeyCode::Enter) {
            Outcome::CloseAct(Action::SaveCrewSetup {
                lead_model,
                crew,
                budget,
                ..
            }) => {
                assert_eq!(lead_model, "deepseek/deepseek-v4.1-flash");
                assert_eq!(
                    crew["builder"].model.as_deref(),
                    Some("deepseek/deepseek-v4.1-flash")
                );
                let auditor = crew["auditor"].model.clone().unwrap();
                assert!(auditor == "anthropic/claude-opus-5" || auditor == "openai/gpt-5.5");
                let e = estimate(b.crew_rates(&v), JobSize::Large);
                assert_eq!(budget, e.budget, "the suggested budget for a large job");
            }
            _ => panic!("enter must save once every model answered"),
        }
    }

    /// A model the account can't use blocks the save and says why.
    #[test]
    fn a_failed_test_blocks_saving_and_says_why() {
        let (mut v, mut b) = setup();
        press(&mut b, &mut v, KeyCode::Enter);
        for _ in SEATS {
            press(&mut b, &mut v, KeyCode::Enter);
        }
        press(&mut b, &mut v, KeyCode::Enter);
        let Outcome::Act(Action::ProbeModels(asked)) = press(&mut b, &mut v, KeyCode::Enter) else {
            panic!("expected a test");
        };
        let results = asked
            .iter()
            .map(|(c, m)| {
                let r = if m.starts_with("anthropic/") || m.starts_with("openai/") {
                    Err("not available under your account's data policy".to_string())
                } else {
                    Ok(())
                };
                (c.clone(), m.clone(), r)
            })
            .collect();
        b.on_notice(&Notice::Probed(results), &mut v);
        assert!(b.problems().iter().any(|p| p.contains("data policy")));
        assert!(matches!(
            press(&mut b, &mut v, KeyCode::Enter),
            Outcome::Act(Action::ProbeModels(_))
        ));
    }

    /// OpenRouter's routers ("price" -1) led the cheapest-first seat lists.
    #[test]
    fn routers_and_negative_prices_are_not_seats() {
        let (mut v, mut b) = setup();
        let mut router = m("openrouter/auto", -1_000_000.0, -1_000_000.0);
        router.context_length = Some(2_000_000);
        b.on_notice(
            &Notice::Models(vec![router, m("vendor/odd", -1.0, 2.0)]),
            &mut v,
        );
        for seat in 0..SEATS.len() {
            let ids: Vec<&str> = b
                .seat_rows(&v, seat)
                .iter()
                .map(|m| m.id.as_str())
                .collect();
            assert!(
                !ids.contains(&"openrouter/auto") && !ids.contains(&"vendor/odd"),
                "{ids:?}"
            );
            assert_eq!(
                ids.first().copied().filter(|_| seat == 0),
                if seat == 0 {
                    Some("deepseek/deepseek-v4.1-flash")
                } else {
                    None
                }
            );
        }
    }

    /// The auditor can't be the lead's or the builder's model.
    #[test]
    fn the_auditor_seat_refuses_the_lead_and_builder_models() {
        let (mut v, mut b) = setup();
        b.sel = 3; // my current crew: everything on the lead's model
        press(&mut b, &mut v, KeyCode::Enter);
        for _ in 0..3 {
            press(&mut b, &mut v, KeyCode::Enter);
        }
        assert_eq!(b.step, Step::Seat(3));
        // Choose the lead's own model from the list: refused.
        let rows = b.seat_rows(&v, 3);
        let k = rows
            .iter()
            .position(|m| m.id == "deepseek/deepseek-v4.1-flash")
            .unwrap();
        b.sel = k + 1;
        press(&mut b, &mut v, KeyCode::Enter);
        assert_eq!(b.step, Step::Seat(3), "stays on the auditor step");
    }
}
