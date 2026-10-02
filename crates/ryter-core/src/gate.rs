//! The review gate: before work is committed, the review hat looks it over.
//!
//! One reviewer. It used to be two: the review hat, and `/audit`, a second
//! model run apart from the conversation with a reviewer and a limit of its
//! own. The review hat has its own model now (`/models`), so it is the
//! second model, and the audit is what it does: asked for with `/audit`, or
//! offered at the end of a build turn that changed files.
//!
//! Every review asks first, naming the model and what it should cost. It
//! is a turn in the conversation, so the reviewer has read what was asked
//! and the builder reads the findings next. It checks the change against
//! the plan the user approved, and ends with a verdict that the commit's
//! receipt carries. The review hat's gate holds: it reads and runs the
//! tests, and changes nothing.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::agent::{Agent, StopReason};
use crate::error::Result;
use crate::event::AgentEvent;
use crate::role::Role;
use crate::spend::{Rates, Usage};

/// Diff the reviewer is expected to read, for the estimate.
const DIFF_CHARS: usize = 60_000;
/// Per file.
const FILE_CHARS: usize = 16_000;
/// Past reviews shown as the estimate once there are this many.
const HISTORY_MIN: usize = 2;
/// Output a step is priced at when a limit is checked before it is sent.
const STEP_OUTPUT: u64 = 2_000;
/// Share of a review's limit after which the reviewer is told to write up.
const WRITE_UP_SHARE: f64 = 0.75;

/// The work to review.
struct Job {
    files: usize,
    added: u32,
    removed: u32,
    /// The files as they are now, as a git tree: what the verdict is about.
    tree: Option<String>,
    /// What the diff adds to what the reviewer reads.
    diff_tokens: u64,
}

/// Whether the next step of a review fits its limit.
pub(crate) enum Fit {
    /// Send it.
    Yes,
    /// Send it, and tell the reviewer to write up: little is left.
    WriteUp,
    /// It would pass the limit: the reason, for the user.
    No(String),
}

/// What the reviewer is told near its limit.
pub(crate) const WRITE_UP: &str = "[Ryter] You are near the spending limit the user set for a \
review. Stop now: use no more tools, and write your findings from what you have, saying what \
you didn't get to check. End with your verdict.";

/// The lowest and highest a review should cost with `context_tokens` of
/// context: at best one round that answers; at worst the six rounds a review
/// is asked to stay within, each re-reading the context and what it read
/// before (about 4k tokens a round), and writing about 4k.
pub fn estimate_range(context_tokens: u64) -> (Usage, Usage) {
    let low = Usage {
        input_tokens: context_tokens,
        output_tokens: 1_500,
        cached_tokens: 0,
        cache_write_tokens: 0,
    };
    let high = Usage {
        input_tokens: context_tokens * 6 + 60_000,
        output_tokens: 24_000,
        cached_tokens: 0,
        cache_write_tokens: 0,
    };
    (low, high)
}

/// What a review should cost with `rates`, low and high; `None` without
/// rates.
pub fn price_range(rates: Option<Rates>, context_tokens: u64) -> Option<(f64, f64)> {
    let r = rates?;
    let (low, high) = estimate_range(context_tokens);
    Some((r.cost(low), r.cost(high)))
}

/// `$0.04–$0.31`, with at least a cent shown; `$0` when free.
pub fn format_range((low, high): (f64, f64)) -> String {
    if high <= 0.0 {
        return "$0".into();
    }
    let cents = |v: f64| format!("${:.2}", v.max(0.01));
    if cents(low) == cents(high) {
        cents(low)
    } else {
        format!("{}–{}", cents(low), cents(high))
    }
}

/// One finished review, for estimates from the user's own history.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Past {
    model: String,
    context_tokens: u64,
    usd: Option<f64>,
}

fn history_path(home: &Path) -> std::path::PathBuf {
    home.join("reviews.jsonl")
}

/// What the user's recent reviews with `model` cost, oldest first.
pub fn history(home: &Path, model: &str) -> Vec<f64> {
    let Some(text) = past_text(history_path(home)) else {
        return Vec::new();
    };
    let all: Vec<f64> = text
        .lines()
        .filter_map(|l| serde_json::from_str::<Past>(l).ok())
        .filter(|p| p.model == model)
        .filter_map(|p| p.usd)
        .collect();
    all[all.len().saturating_sub(10)..].to_vec()
}

fn remember(home: &Path, past: &Past) {
    remember_at(&history_path(home), past);
}

fn remember_at(path: &Path, past: &Past) {
    // In Ryter's own folder, which a sandbox profile shuts: kept outside it.
    let (path, past) = (path.to_path_buf(), past.clone());
    crate::outside::run(move || {
        let _ = crate::session::append_jsonl(&path, &past);
    });
}

/// A history file's text, read where a sandbox profile doesn't reach.
fn past_text(path: std::path::PathBuf) -> Option<String> {
    crate::outside::run(move || std::fs::read_to_string(path)).ok()
}

/// `Some(true)` for a `VERDICT: PASS` line, `Some(false)` for any other
/// verdict, `None` when the review gave none. The last `VERDICT` line
/// wins: reviewers often restate the choices before deciding.
pub fn verdict(text: &str) -> Option<bool> {
    text.lines()
        .filter_map(|l| {
            let l = l
                .trim()
                .trim_matches(|c: char| matches!(c, '*' | '#' | '`' | '_' | '>' | ' '))
                .to_ascii_uppercase();
            let said = l.strip_prefix("VERDICT")?;
            Some(
                said.trim_start_matches([':', ' ', '*', '-'])
                    .starts_with("PASS"),
            )
        })
        .next_back()
}

/// The same model, whichever route reached it: `x-ai/grok-4.6`, `grok-4.6`,
/// and `grok-4.6-latest` are one model.
pub fn same_model(a: &str, b: &str) -> bool {
    let norm = |m: &str| {
        let m = m
            .rsplit('/')
            .next()
            .unwrap_or(m)
            .trim()
            .to_ascii_lowercase();
        m.trim_end_matches("-latest").to_string()
    };
    norm(a) == norm(b)
}

/// The lowest and highest a test should cost with `context_tokens` to
/// start from. A test is longer than a review: it starts the product, runs
/// its tests and tries it, so at best a handful of rounds and at worst a
/// few dozen, each re-reading what came before.
pub fn estimate_test(context_tokens: u64) -> (Usage, Usage) {
    let low = Usage {
        input_tokens: context_tokens * 5 + 15_000,
        output_tokens: 3_000,
        cached_tokens: 0,
        cache_write_tokens: 0,
    };
    let high = Usage {
        input_tokens: context_tokens * 25 + 300_000,
        output_tokens: 20_000,
        cached_tokens: 0,
        cache_write_tokens: 0,
    };
    (low, high)
}

fn test_history_path(home: &Path) -> std::path::PathBuf {
    home.join("tests.jsonl")
}

/// What the user's recent tests with `model` cost, oldest first.
pub fn test_history(home: &Path, model: &str) -> Vec<f64> {
    let Some(text) = past_text(test_history_path(home)) else {
        return Vec::new();
    };
    let all: Vec<f64> = text
        .lines()
        .filter_map(|l| serde_json::from_str::<Past>(l).ok())
        .filter(|p| p.model == model)
        .filter_map(|p| p.usd)
        .collect();
    all[all.len().saturating_sub(10)..].to_vec()
}

/// A step of the checks that follow a build: each can lead to the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Review,
    Test,
}

/// How a test came out, for what follows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TestRun {
    /// None ran, or it filed no report.
    NotRun,
    /// Every scenario passed.
    Passed,
    /// Something failed, and the build hat has not changed anything since.
    Failed,
    /// Something failed, and the build hat changed files to fix it: new
    /// work, to review.
    Fixed,
}

impl Agent {
    /// `/audit`: the review hat reviews the uncommitted work now, after the
    /// user agrees to what it should cost. A review that passes is followed
    /// by the offer of a test. Returns the review, or an empty string when
    /// none ran.
    pub async fn review_now(&mut self) -> Result<String> {
        self.checks(Step::Review, false).await
    }

    /// At the end of a build turn that changed files: offer a review, with
    /// what it would cost, unless the user turned offers off. Saying yes
    /// runs it; there is no second question. Nothing when there is nothing
    /// uncommitted to review.
    pub async fn offer_review(&mut self) -> Result<String> {
        if !self.offers_reviews() || self.role != Role::SoloBuild {
            return Ok(String::new());
        }
        self.checks(Step::Review, true).await
    }

    /// `/test`: the test hat uses the product now, after the user agrees to
    /// what it should cost.
    pub async fn test_now(&mut self) -> Result<()> {
        self.checks(Step::Test, false).await.map(|_| ())
    }

    fn offers_tests(&self) -> bool {
        self.cfg.as_ref().is_some_and(|c| c.ui.offer_test) && self.ctx.user_io.is_some()
    }

    /// Review, then test, then the fixes, for as long as each leads to the
    /// next: a review that passes is followed by the offer of a test, and a
    /// test whose failures the build hat fixed is followed by the offer of
    /// a review of the fixes. Returns the last review.
    async fn checks(&mut self, start: Step, offered: bool) -> Result<String> {
        let mut step = start;
        let mut offered = offered;
        let mut last = String::new();
        loop {
            match step {
                Step::Review => {
                    let (text, passed) = self.review(offered).await?;
                    last = text;
                    if !passed || !self.offers_tests() || self.role == Role::SoloTest {
                        return Ok(last);
                    }
                    step = Step::Test;
                }
                Step::Test => {
                    let run = self.test_once(offered).await?;
                    if run != TestRun::Fixed
                        || !self.offers_reviews()
                        || self.role != Role::SoloBuild
                    {
                        return Ok(last);
                    }
                    step = Step::Review;
                }
            }
            offered = true;
        }
    }

    /// What the tester is asked: where to start from, since it has not read
    /// the conversation the work was done in.
    fn test_brief(&self) -> String {
        let root = self.root();
        let mut s = String::from("[Ryter] Test the work as its user would.");
        match self.session.meta.plan_file.as_deref() {
            Some(file) => {
                s.push_str(&format!(
                    " The plan the user approved is in `{file}`. Read it: its \"How to verify\" \
                     section, and each step a user would notice, is a scenario to try."
                ));
                if let Some(p) = crate::decisions::pointer(&root, file) {
                    s.push(' ');
                    s.push_str(&p);
                }
            }
            None => s.push_str(
                " No plan was approved for this work: test what changed, from the project's \
                 README and the changes named below.",
            ),
        }
        if let Ok(r) = crate::review::root(&self.ctx.workspace) {
            let base = crate::review::head_base(&r);
            if let Ok(changes) = crate::review::changes(&r, &base).map(|c| c.work()) {
                if !changes.files.is_empty() {
                    let (added, removed) = changes.totals();
                    let names: Vec<&str> = changes
                        .files
                        .iter()
                        .take(12)
                        .map(|f| f.path.as_str())
                        .collect();
                    let more = changes.files.len().saturating_sub(names.len());
                    s.push_str(&format!(
                        " Since the last commit {} file{} changed (+{added} −{removed}): {}{}.",
                        changes.files.len(),
                        if changes.files.len() == 1 { "" } else { "s" },
                        names.join(", "),
                        if more > 0 {
                            format!(", and {more} more")
                        } else {
                            String::new()
                        }
                    ));
                }
            }
        }
        s.push_str(match crate::run::find(&root, &self.home) {
            crate::run::Found::Approved(_) => {
                " How it runs is in `.ryter/run.toml`, which the user approved: start it with \
                 run_project."
            }
            crate::run::Found::Unapproved(..) => {
                " It has a `.ryter/run.toml` the user has not approved as it stands: \
                 run_project will ask them."
            }
            _ => {
                " It has no run file yet: read how it starts and tests itself, and propose \
                 one with propose_run."
            }
        });
        let running = self.product.is_some() || crate::run::remembered(&self.home, &root).is_some();
        if let Some(last) = crate::testing::latest(&root) {
            s.push_str(&format!(
                " The last report filed here is `{last}`: retest what failed in it first."
            ));
        }
        s.push_str(if running {
            " Ryter started the product earlier and it should still be up: run_project \
             (action `start`) says whether it is, and starts it only if it has gone. Run \
             its tests, then try each scenario. File your report with report_test, and \
             leave the product running."
        } else {
            " Start it, run its tests, then try each scenario. File your report with \
             report_test, and leave the product running."
        });
        s
    }

    /// One test: ask, run it in the test hat, and, if it failed, offer its
    /// fixes in the build hat.
    async fn test_once(&mut self, offered: bool) -> Result<TestRun> {
        use crate::role::Thread;
        let cfg = self.cfg.clone().unwrap_or_default();
        let (_, model, connection) = self.stack_for(Role::SoloTest);
        let local = cfg
            .connections
            .get(&connection)
            .is_some_and(|c| c.is_local());
        let rates = self.book.rates(&model);
        let brief = self.test_brief();
        // What the tester reads to begin with: its instructions, its own
        // conversation so far, and the brief.
        let system = self.system_prompt()?;
        let thread: usize = self
            .session
            .messages_of(Thread::Test)
            .iter()
            .map(|m| m.content.len())
            .sum();
        let context_tokens = ((system.len() + thread + brief.len()) / 4) as u64 + 3_000;
        let past = test_history(&self.home, &model);
        let cost = if local {
            "runs on this machine, $0".to_string()
        } else {
            let range = rates.map(|r| {
                let (low, high) = estimate_test(context_tokens);
                format_range((r.cost(low), r.cost(high)))
            });
            let mut s = match range {
                Some(range) => format!("about {range}"),
                None => "no price is known for it".to_string(),
            };
            if past.len() >= HISTORY_MIN {
                let lo = past.iter().copied().fold(f64::MAX, f64::min);
                let hi = past.iter().copied().fold(0.0, f64::max);
                s.push_str(&format!(
                    "\nyour last {} tests with it cost {}",
                    past.len(),
                    format_range((lo, hi))
                ));
            }
            s
        };
        let root = self.root();
        let scenarios = self
            .session
            .meta
            .plan_file
            .as_deref()
            .and_then(|f| crate::plan::read_own(&root, f).ok())
            .map(|plan| crate::testing::scenarios_in_plan(&plan));
        let what = match scenarios {
            Some(Some(1)) => "starts the project, runs 1 scenario from the plan".to_string(),
            Some(Some(n)) => format!("starts the project, runs {n} scenarios from the plan"),
            Some(None) => "starts the project and tests it against the plan".to_string(),
            None => "starts the project and tests what changed".to_string(),
        };
        let summary = format!(
            "{}{model} on {connection} (the test hat's model)\n{what}, leaves it running\n{cost}",
            if offered { "Test this work?\n" } else { "" },
        );
        if let Some(io) = self.ctx.user_io.clone() {
            let tool = if offered { "test offer" } else { "test" };
            let answer = io.permission(tool, &summary, &self.ctx.cancel);
            if self.ctx.cancel.is_cancelled() {
                return Ok(TestRun::NotRun);
            }
            match answer {
                crate::user_io::Permission::Allow | crate::user_io::Permission::Always => {}
                crate::user_io::Permission::Deny if offered => return Ok(TestRun::NotRun),
                crate::user_io::Permission::Deny => {
                    return self.say("test not run").map(|_| TestRun::NotRun);
                }
            }
        }

        // The test is a turn in the test hat, in the tester's own
        // conversation. The hat the user was in comes back when it ends.
        let prior = self.role;
        if prior != Role::SoloTest {
            self.wear(Role::SoloTest)?;
        }
        // In the tester's conversation, what it was asked: the brief itself
        // is for the model.
        let asked = match self.session.meta.plan_file.as_deref() {
            Some(plan) => format!(
                "Ryter · test the work against the plan\n{}",
                plan.trim_start_matches(".ryter/")
            ),
            None => "Ryter · test what changed".to_string(),
        };
        self.emit(AgentEvent::Notice { message: asked })?;
        self.filed.delivered = None;
        let spent_from = self.session.spend_log().map_or(0, |l| l.len());
        let out = self.turn(&brief).await;
        if self.role == Role::SoloTest && prior != Role::SoloTest {
            self.wear(prior)?;
        }
        // A turn that failed has said why.
        let Ok(turn) = out else {
            return Ok(TestRun::NotRun);
        };
        let spent: Vec<_> = self
            .session
            .spend_log()
            .unwrap_or_default()
            .into_iter()
            .skip(spent_from)
            .filter(|r| r.role == Role::SoloTest)
            .collect();
        if !spent.is_empty() {
            remember_at(
                &test_history_path(&self.home),
                &Past {
                    model: model.clone(),
                    context_tokens,
                    usd: spent
                        .iter()
                        .map(|r| if r.incomplete { None } else { r.total_usd })
                        .sum::<Option<f64>>(),
                },
            );
        }
        if turn.reason == StopReason::Cancelled {
            return Ok(TestRun::NotRun);
        }
        match self.filed.delivered {
            None => self
                .say(
                    "the tester filed no report · what it found is in its own conversation \
                     (Tab to the test hat)",
                )
                .map(|_| TestRun::NotRun),
            Some(true) => Ok(TestRun::Passed),
            Some(false) => self.offer_fixes().await,
        }
    }

    /// After a test that failed: offer its fixes in the build hat, in the
    /// conversation the report is now part of.
    async fn offer_fixes(&mut self) -> Result<TestRun> {
        let Some(io) = self.ctx.user_io.clone() else {
            return Ok(TestRun::Failed);
        };
        // The fixes are the build hat's, in the shared conversation.
        if self.role == Role::SoloTest {
            return Ok(TestRun::Failed);
        }
        let answer = io.permission(
            "fix offer",
            "switch to the build hat and fix what the test found\nthe report is in the \
             conversation: the builder works from it",
            &self.ctx.cancel,
        );
        if self.ctx.cancel.is_cancelled()
            || !matches!(
                answer,
                crate::user_io::Permission::Allow | crate::user_io::Permission::Always
            )
        {
            return Ok(TestRun::Failed);
        }
        if self.role != Role::SoloBuild {
            self.wear(Role::SoloBuild)?;
        }
        let before = self.session.changed_turns;
        let out = self
            .turn(
                "[Ryter] The test report above has failures. Fix what it found, one failure \
                 at a time, and run the project's own tests for each fix. Don't change a \
                 test to make it pass. Say what you fixed and what you could not.",
            )
            .await;
        if out.is_err() {
            return Ok(TestRun::Failed);
        }
        Ok(if self.session.changed_turns > before {
            TestRun::Fixed
        } else {
            TestRun::Failed
        })
    }

    fn offers_reviews(&self) -> bool {
        self.cfg.as_ref().is_some_and(|c| c.ui.offer_audit) && self.ctx.user_io.is_some()
    }

    fn say(&mut self, message: impl Into<String>) -> Result<String> {
        self.emit(AgentEvent::Notice {
            message: message.into(),
        })?;
        Ok(String::new())
    }

    /// Wear `role` from here on, and say so.
    fn wear(&mut self, role: Role) -> Result<()> {
        self.put_on(role)?;
        self.emit(AgentEvent::ModeChanged { role })
    }

    /// What there is to review; `Err` says why there is nothing.
    fn review_job(&mut self) -> Result<std::result::Result<Job, String>> {
        let Ok(root) = crate::review::root(&self.ctx.workspace) else {
            return Ok(Err(
                "a review reads what changed, which needs a git repository".into(),
            ));
        };
        let base = crate::review::head_base(&root);
        // The work, not Ryter's own record of it: the plan and the
        // decisions are what the review is held against, not part of it.
        let changes = crate::review::changes(&root, &base)?.work();
        if changes.files.is_empty() {
            return Ok(Err("nothing uncommitted to review".into()));
        }
        let paths: Vec<String> = changes.files.iter().map(|f| f.path.clone()).collect();
        let diff = crate::review::diff_within(&root, &changes, &paths, DIFF_CHARS, FILE_CHARS);
        let (added, removed) = changes.totals();
        Ok(Ok(Job {
            files: paths.len(),
            added,
            removed,
            tree: crate::review::tree_of(&root, &changes.now),
            diff_tokens: diff.len() as u64 / 4,
        }))
    }

    /// What the reviewer is asked, as a message in the conversation.
    fn review_brief(&self, job: &Job) -> String {
        let plan = match self.session.meta.plan_file.as_deref() {
            Some(file) => {
                let root = self
                    .project_root
                    .clone()
                    .unwrap_or_else(|| self.ctx.workspace.clone());
                let decided = crate::decisions::pointer(&root, file)
                    .map(|p| format!(" {p}"))
                    .unwrap_or_default();
                format!(
                    "The plan the user approved is in `{file}`. Read it, and check the \
                     change does what it says: all of it, and nothing it doesn't call \
                     for.{decided}"
                )
            }
            None => "No plan was approved for this work: check it against what the user \
                     asked for in this conversation."
                .to_string(),
        };
        format!(
            "[Ryter] Review the uncommitted changes before they are committed: {} file{}, \
             +{} −{}. {plan} Start from `git status --short` and `git diff HEAD`. End with \
             your findings and your verdict.",
            job.files,
            if job.files == 1 { "" } else { "s" },
            job.added,
            job.removed,
        )
    }

    /// Ask, then run, and offer again for as long as a review ends with the
    /// build hat changing files (its fixes are new work). `offered`: Ryter
    /// is offering at the end of a build turn, so a "no" says nothing, and
    /// the prompt can stop the offers.
    ///
    /// Returns the last review, and whether the work stands reviewed: that
    /// review passed, and nothing was changed after it.
    async fn review(&mut self, offered: bool) -> Result<(String, bool)> {
        let mut offered = offered;
        let mut last = String::new();
        loop {
            let before = self.session.changed_turns;
            let Some((text, verdict)) = self.review_once(offered).await? else {
                return Ok((last, false));
            };
            last = text;
            let fixed = self.session.changed_turns > before;
            if !fixed || !self.offers_reviews() || self.role != Role::SoloBuild {
                return Ok((last, verdict == Some(true) && !fixed));
            }
            offered = true;
        }
    }

    /// One review, and its verdict: `None` when none ran.
    async fn review_once(&mut self, offered: bool) -> Result<Option<(String, Option<bool>)>> {
        let job = match self.review_job()? {
            Ok(job) => job,
            Err(_) if offered => return Ok(None),
            Err(why) => return self.say(why).map(|_| None),
        };
        let cfg = self.cfg.clone().unwrap_or_default();
        let (_, model, connection) = self.stack_for(Role::SoloReview);
        let (_, builder, _) = self.stack_for(Role::SoloBuild);
        let local = cfg
            .connections
            .get(&connection)
            .is_some_and(|c| c.is_local());
        let limit = cfg.spend.review_usd;
        let rates = self.book.rates(&model);
        // A limit can't hold a model with no price.
        if !local && limit > 0.0 && rates.is_none() {
            return self
                .say(format!(
                    "no review: no price is known for {model}, so your ${limit:.2} review \
                     limit can't be held to. Give the review hat another model in /models, \
                     set its price ([pricing] in config.toml), or turn the limit off in \
                     /settings."
                ))
                .map(|_| None);
        }

        let system = self.system_prompt()?;
        // A review reads the conversation the work was done in. Asked for
        // from the test hat, the session has the tester's thread in use:
        // what the review will read, and so what it costs, is the shared
        // one's.
        let was = self.session.thread();
        self.session.use_thread(crate::role::Thread::Main);
        // A limit its first step would pass: say so, rather than ask for a
        // yes to a review that stops before it starts.
        let spent = self.session.meta.spend_usd_total.unwrap_or(0.0);
        let fit = self.review_fit(&model, &connection, &system, spent);
        // What the user agrees to before anything is spent.
        let context_tokens = self.conversation_tokens(&system) + job.diff_tokens;
        self.session.use_thread(was);
        if let Fit::No(why) = fit {
            return self.say(why).map(|_| None);
        }
        let past = history(&self.home, &model);
        let cost = if local {
            "runs on this machine, $0".to_string()
        } else {
            let mut s = match price_range(rates, context_tokens).map(format_range) {
                Some(range) if limit > 0.0 => format!("about {range} of your ${limit:.2} limit"),
                Some(range) => format!("about {range}"),
                None => "no price is known for it".to_string(),
            };
            if past.len() >= HISTORY_MIN {
                let lo = past.iter().copied().fold(f64::MAX, f64::min);
                let hi = past.iter().copied().fold(0.0, f64::max);
                s.push_str(&format!(
                    "\nyour last {} reviews with it cost {}",
                    past.len(),
                    format_range((lo, hi))
                ));
            }
            s
        };
        // A second opinion is another model's. The same one may still
        // review; the user is told which they are getting.
        let who = if same_model(&model, &builder) {
            format!(
                "{model} on {connection}, the model that built it\n\
                 (give the review hat its own in /models for a second opinion)"
            )
        } else {
            format!("{model} on {connection} (the review hat's model)")
        };
        let summary = format!(
            "{}{who}\nreviews {} file{}, +{} −{}, read-only\n{cost}",
            if offered {
                "Review this work before you commit?\n"
            } else {
                ""
            },
            job.files,
            if job.files == 1 { "" } else { "s" },
            job.added,
            job.removed,
        );
        if let Some(io) = self.ctx.user_io.clone() {
            let tool = if offered { "review offer" } else { "review" };
            let answer = io.permission(tool, &summary, &self.ctx.cancel);
            if self.ctx.cancel.is_cancelled() {
                return Ok(None);
            }
            match answer {
                crate::user_io::Permission::Allow | crate::user_io::Permission::Always => {}
                crate::user_io::Permission::Deny if offered => return Ok(None),
                crate::user_io::Permission::Deny => {
                    return self.say("review not run").map(|_| None);
                }
            }
        }

        // The review is a turn in the review hat. The hat the user was in
        // comes back when it ends, unless the reviewer asked for another
        // (its fixes, in the build hat) and the user said yes.
        let prior = self.role;
        if prior != Role::SoloReview {
            self.wear(Role::SoloReview)?;
        }
        let from = self.session.messages_of(crate::role::Thread::Main).len();
        let spent_from = self.session.spend_log().map_or(0, |l| l.len());
        let brief = self.review_brief(&job);
        let out = self.turn(&brief).await;
        if self.role == Role::SoloReview && prior != Role::SoloReview {
            self.wear(prior)?;
        }
        // A turn that failed has said why.
        let Ok(turn) = out else {
            return Ok(None);
        };
        if turn.reason == StopReason::Cancelled {
            return Ok(None);
        }
        // The verdict is the reviewer's last one: after a FAIL the build
        // hat may have gone on in the same turn, and its words end it.
        // Read from the shared conversation by name: the hat the user was
        // in is back by now, and it may be the tester's. Read from the
        // thread in use, a review asked for in the test hat took its
        // verdict from the tester's last words.
        let said = self
            .session
            .messages_of(crate::role::Thread::Main)
            .iter()
            .skip(from)
            .filter(|m| m.role == "assistant")
            .filter_map(|m| verdict(&m.content))
            .next_back();
        let reviews: Vec<_> = self
            .session
            .spend_log()
            .unwrap_or_default()
            .into_iter()
            .skip(spent_from)
            .filter(|r| r.role == Role::SoloReview)
            .collect();
        let total_usd = reviews
            .iter()
            .map(|r| if r.incomplete { None } else { r.total_usd })
            .sum::<Option<f64>>()
            .filter(|_| !reviews.is_empty());
        // Stopped at its limit before anything was sent: no review ran.
        if reviews.is_empty() {
            return Ok(None);
        }
        remember(
            &self.home,
            &Past {
                model: model.clone(),
                context_tokens,
                usd: total_usd,
            },
        );
        self.emit(AgentEvent::Reviewed {
            model,
            connection,
            verdict: said,
            tree: job.tree,
            total_usd,
        })?;
        Ok(Some((turn.text, said)))
    }

    /// Before a step in the review hat: whether it fits the user's limit
    /// for a review, counted from `from` (the session's spend when the
    /// review began). No limit, a model on this machine, or a model with no
    /// price: it fits.
    pub(crate) fn review_fit(&self, model: &str, connection: &str, system: &str, from: f64) -> Fit {
        let Some(cfg) = &self.cfg else {
            return Fit::Yes;
        };
        let limit = cfg.spend.review_usd;
        let local = cfg
            .connections
            .get(connection)
            .is_some_and(|c| c.is_local());
        if limit <= 0.0 || local {
            return Fit::Yes;
        }
        let spent = (self.session.meta.spend_usd_total.unwrap_or(0.0) - from).max(0.0);
        let step = self
            .book
            .cost(
                model,
                Usage {
                    input_tokens: self.conversation_tokens(system),
                    output_tokens: STEP_OUTPUT,
                    ..Usage::default()
                },
            )
            .unwrap_or(0.0);
        if spent + step > limit {
            return Fit::No(format!(
                "review stopped at your ${limit:.2} limit: ${spent:.2} spent, and the next \
                 step is about ${step:.2}. /settings changes the limit."
            ));
        }
        if spent >= limit * WRITE_UP_SHARE || spent + 2.0 * step > limit {
            return Fit::WriteUp;
        }
        Fit::Yes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_verdict_line_is_the_verdict() {
        assert_eq!(
            verdict("VERDICT: PASS or VERDICT: FAIL?\n\n- a bug\n\n**VERDICT: FAIL**"),
            Some(false)
        );
        assert_eq!(
            verdict("VERDICT: FAIL\nOn reflection:\nVERDICT: PASS"),
            Some(true)
        );
        // A verdict that isn't a pass is not one.
        assert_eq!(verdict("VERDICT: UNVERIFIED"), Some(false));
        assert!(same_model("x-ai/grok-4.6", "grok-4.6-latest"));
        assert!(!same_model("x-ai/grok-4.6", "x-ai/grok-4.7"));
    }

    #[test]
    fn a_verdict_is_read_and_its_absence_is_not_a_fail() {
        assert_eq!(
            verdict("- a.rs:3 off by one (blocking)\n\nVERDICT: FAIL"),
            Some(false)
        );
        assert_eq!(verdict("Nothing to report.\n**VERDICT: PASS**"), Some(true));
        assert_eq!(verdict("Looks fine to me."), None);
    }

    /// The range grows with the context, and the top of it assumes a review
    /// that explores: a $90 review was not a one-round answer.
    #[test]
    fn estimates_are_ranges_that_grow_with_the_work() {
        let rates = Some(Rates::per_million(3.0, 15.0));
        let (lo, hi) = price_range(rates, 10_000).unwrap();
        assert!(hi > lo * 5.0, "{lo} {hi}");
        let (_, big) = price_range(rates, 200_000).unwrap();
        assert!(
            big > 3.0,
            "a large change reviewed by a pricey model: {big}"
        );
        assert_eq!(format_range((0.001, 0.002)), "$0.01");
        assert_eq!(format_range((0.0, 0.0)), "$0");
        assert_eq!(format_range((0.04, 0.31)), "$0.04–$0.31");
        assert!(price_range(None, 10).is_none());
    }

    #[test]
    fn history_is_per_model_and_recent() {
        let home = tempfile::TempDir::new().unwrap();
        for usd in [0.1, 0.2, 0.3] {
            remember(
                home.path(),
                &Past {
                    model: "a".into(),
                    context_tokens: 1,
                    usd: Some(usd),
                },
            );
        }
        remember(
            home.path(),
            &Past {
                model: "b".into(),
                context_tokens: 1,
                usd: None,
            },
        );
        assert_eq!(history(home.path(), "a"), vec![0.1, 0.2, 0.3]);
        assert!(history(home.path(), "b").is_empty());
    }
}
