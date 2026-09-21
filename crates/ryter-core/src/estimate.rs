//! What a crew is likely to cost for a job, and what budget to set.
//!
//! The token profiles are measured, not guessed: they are per-unit averages
//! from paid live runs on 2026-09-21 (`ROADMAP.md`). An Opus design took ~51k
//! input and ~28k output tokens (mostly reasoning); a builder task ~90k input
//! (~60% cache reads) and ~8k output; an audit ~40k input and ~3.6k output.
//! A handful of runs is a small sample, so every estimate is labelled rough.
//! `ryter bench` is how to replace these with numbers for your own crew.

/// Price of one model, $ per million tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rates {
    /// Uncached input.
    pub input: f64,
    /// Output (reasoning included).
    pub output: f64,
}

impl Rates {
    /// Cache reads, when the provider does not say: a quarter of input is
    /// typical (Anthropic bills a tenth, OpenAI a quarter to a half).
    fn cached(self) -> f64 {
        self.input / 4.0
    }

    fn cost(self, p: Profile) -> f64 {
        (p.uncached * self.input + p.cached * self.cached() + p.output * self.output) / 1e6
    }
}

/// Tokens one unit of work uses.
#[derive(Debug, Clone, Copy)]
struct Profile {
    uncached: f64,
    cached: f64,
    output: f64,
}

const LEAD_PER_TASK: Profile = Profile {
    uncached: 2_000.0,
    cached: 4_000.0,
    output: 600.0,
};
const DESIGN: Profile = Profile {
    uncached: 37_000.0,
    cached: 14_000.0,
    output: 28_000.0,
};
const BUILD: Profile = Profile {
    uncached: 35_000.0,
    cached: 55_000.0,
    output: 8_000.0,
};
const AUDIT: Profile = Profile {
    uncached: 12_000.0,
    cached: 28_000.0,
    output: 3_600.0,
};
/// About a third of tasks are sent back once; a retry in place costs less
/// than a build, so this is a rough allowance on build and review.
const RETRIES: f64 = 1.3;
/// Headroom between the estimate and the suggested budget.
const HEADROOM: f64 = 1.5;

/// How big the job is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobSize {
    /// A fix or a small feature: a couple of tasks, no design.
    Small,
    /// A feature across a few files: ~5 tasks and one design.
    Medium,
    /// A new app or a big change: ~20 tasks and three designs.
    Large,
}

impl JobSize {
    /// Smallest first.
    pub const ALL: [JobSize; 3] = [JobSize::Small, JobSize::Medium, JobSize::Large];

    /// `(builder tasks, designs)`.
    pub fn shape(self) -> (u32, u32) {
        match self {
            JobSize::Small => (2, 0),
            JobSize::Medium => (5, 1),
            JobSize::Large => (20, 3),
        }
    }

    /// Lowercase name.
    pub fn name(self) -> &'static str {
        match self {
            JobSize::Small => "small",
            JobSize::Medium => "medium",
            JobSize::Large => "large",
        }
    }

    /// What it looks like, for people.
    pub fn describe(self) -> &'static str {
        match self {
            JobSize::Small => "a fix or small feature · ~2 tasks, no design",
            JobSize::Medium => "a feature across a few files · ~5 tasks, 1 design",
            JobSize::Large => "a new app or big change · ~20 tasks, 3 designs",
        }
    }
}

/// The crew's prices; `None` where a model's price is unknown.
#[derive(Debug, Clone, Copy, Default)]
pub struct CrewRates {
    /// Lead.
    pub lead: Option<Rates>,
    /// Architect.
    pub architect: Option<Rates>,
    /// Builder.
    pub builder: Option<Rates>,
    /// Auditor.
    pub auditor: Option<Rates>,
}

/// A rough cost and the budget to set for it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Estimate {
    /// One builder task: its build, its review, retries, and the lead's share.
    pub per_task: f64,
    /// One architect design.
    pub per_design: f64,
    /// The whole job.
    pub total: f64,
    /// `total` with headroom, rounded up to a round number.
    pub budget: f64,
    /// Some model's price is unknown, so the figures leave it out.
    pub partial: bool,
}

/// Estimate `size` for a crew priced at `rates`.
pub fn estimate(rates: CrewRates, size: JobSize) -> Estimate {
    let cost = |r: Option<Rates>, p: Profile| r.map(|r| r.cost(p)).unwrap_or(0.0);
    let per_task = cost(rates.lead, LEAD_PER_TASK)
        + RETRIES * (cost(rates.builder, BUILD) + cost(rates.auditor, AUDIT));
    let per_design = cost(rates.architect, DESIGN);
    let (tasks, designs) = size.shape();
    let total = per_task * f64::from(tasks) + per_design * f64::from(designs);
    let partial = rates.lead.is_none()
        || rates.builder.is_none()
        || rates.auditor.is_none()
        || (designs > 0 && rates.architect.is_none());
    Estimate {
        per_task,
        per_design,
        total,
        budget: round_up(total * HEADROOM),
        partial,
    }
}

/// A per-task cap that lets this crew's normal task or design finish, with
/// room for a hard one, and still stops a runaway: twice the larger of the
/// two, rounded up.
pub fn task_cap_for(e: &Estimate) -> f64 {
    round_up(2.0 * e.per_task.max(e.per_design))
}

/// $0.50 steps under $2, whole dollars under $20, then $5 steps.
fn round_up(v: f64) -> f64 {
    let step = if v < 2.0 {
        0.5
    } else if v < 20.0 {
        1.0
    } else {
        5.0
    };
    ((v / step).ceil() * step).max(0.5)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(input: f64, output: f64) -> Option<Rates> {
        Some(Rates { input, output })
    }

    /// The crew from the live runs: deepseek lead, Opus architect, grok-4.6
    /// builder, glm auditor.
    fn live_crew() -> CrewRates {
        CrewRates {
            lead: r(0.15, 0.6),
            architect: r(5.0, 25.0),
            builder: r(2.0, 6.0),
            auditor: r(0.6, 2.2),
        }
    }

    /// Run 4+5 was three tasks and one Opus design for $1.72, the design
    /// $0.91 of it. The model should land near that, not an order off.
    #[test]
    fn it_matches_the_paid_live_run() {
        let e = estimate(live_crew(), JobSize::Medium);
        assert!((e.per_design - 0.91).abs() < 0.1, "{e:?}");
        let three_tasks = 3.0 * e.per_task + e.per_design;
        assert!((1.2..2.2).contains(&three_tasks), "{three_tasks}");
        assert!(!e.partial);
    }

    /// A large job on that crew outgrew $5 and $10 budgets in real use.
    #[test]
    fn a_large_job_suggests_more_than_ten_dollars_on_that_crew() {
        let e = estimate(live_crew(), JobSize::Large);
        assert!(e.budget > 10.0, "{e:?}");
        assert!(e.budget >= e.total * HEADROOM);
    }

    #[test]
    fn sizes_and_crews_order_as_expected() {
        let cheap = CrewRates {
            lead: r(0.15, 0.6),
            architect: r(0.4, 1.6),
            builder: r(0.15, 0.6),
            auditor: r(0.4, 1.6),
        };
        for size in JobSize::ALL {
            assert!(estimate(cheap, size).total < estimate(live_crew(), size).total);
        }
        let t = |s| estimate(live_crew(), s).total;
        assert!(t(JobSize::Small) < t(JobSize::Medium) && t(JobSize::Medium) < t(JobSize::Large));
    }

    #[test]
    fn unknown_prices_are_flagged_not_hidden() {
        let mut c = live_crew();
        c.auditor = None;
        assert!(estimate(c, JobSize::Small).partial);
        // A small job has no design, so an unpriced architect doesn't matter.
        let mut c = live_crew();
        c.architect = None;
        assert!(!estimate(c, JobSize::Small).partial);
    }

    /// An Opus design (~$0.91) must fit under the task cap; the old $1
    /// default nearly cut it off.
    #[test]
    fn the_task_cap_fits_a_design_with_room() {
        let e = estimate(live_crew(), JobSize::Medium);
        let cap = task_cap_for(&e);
        assert!(
            cap >= 1.5 * e.per_design && cap >= 1.5 * e.per_task,
            "{cap} {e:?}"
        );
    }

    #[test]
    fn budgets_round_to_numbers_people_type() {
        assert_eq!(round_up(0.3), 0.5);
        assert_eq!(round_up(1.2), 1.5);
        assert_eq!(round_up(7.2), 8.0);
        assert_eq!(round_up(21.0), 25.0);
    }
}
