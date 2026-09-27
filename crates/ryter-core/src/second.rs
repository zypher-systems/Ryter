//! A second opinion: another model reviews the uncommitted work, when the
//! user asks (`/second`).
//!
//! The crew's best idea, independent review, without the crew. Ryter never
//! chooses the reviewer or what it may spend: the user picks both, once,
//! from the live catalog, with each model's price for this review in
//! dollars. Models change too fast to curate, and a model Ryter chose would
//! be a bill Ryter chose.
//!
//! Every review asks first, with an estimate against the user's limit. Near
//! the limit the reviewer is told to write up what it has; at the limit it
//! stops, and the user still gets its findings. It works under the review
//! hat's gate, so it can read and run the tests but change nothing.

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::agent::Agent;
use crate::config::ReviewerConfig;
use crate::crew::{self, Bill, Progress};
use crate::error::{Error, Result};
use crate::event::AgentEvent;
use crate::llm::{Message, Provider};
use crate::meter::{Caps, Meter};
use crate::role::Role;
use crate::spend::{Rates, Usage};

/// The meter's task id for a review's spend.
const TASK: &str = "second-opinion";
/// Characters of conversation the reviewer reads.
const DIGEST_CHARS: usize = 12_000;
/// Diff given up front: more than a commit draft gets, so the reviewer
/// reads fewer files itself (each file read is paid again every round).
const DIFF_CHARS: usize = 60_000;
/// Per file.
const FILE_CHARS: usize = 16_000;
/// Reasoning for reviews unless the user set one for the model. Reasoning is
/// billed as output, and a review needs judgement more than search.
const REVIEW_EFFORT: &str = "medium";
/// Past reviews shown as the estimate once there are this many.
const HISTORY_MIN: usize = 2;

/// Who gives the second opinion, ready to run.
#[derive(Clone)]
pub struct Reviewer {
    /// Inference for its connection.
    pub provider: Arc<dyn Provider>,
    /// The user's choice.
    pub choice: ReviewerConfig,
    /// Runs on this machine, for $0.
    pub local: bool,
}

/// The work to review and what the reviewer will read.
struct Job {
    messages: Vec<Message>,
    files: usize,
    added: u32,
    removed: u32,
    context_tokens: u64,
}

/// The lowest and highest a review should cost with `context_tokens` of
/// context: at best one round that answers; at worst the six rounds a review
/// is asked to stay within, each re-reading the context and what it read
/// before (about 4k tokens a round), and writing about 4k.
pub fn estimate_range(context_tokens: u64) -> (Usage, Usage) {
    let low = Usage {
        input_tokens: context_tokens,
        output_tokens: 1_500,
        cached_tokens: 0,
    };
    let high = Usage {
        input_tokens: context_tokens * 6 + 60_000,
        output_tokens: 24_000,
        cached_tokens: 0,
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
    let Ok(text) = std::fs::read_to_string(history_path(home)) else {
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
    let _ = crate::session::append_jsonl(&history_path(home), past);
}

/// Models already found in their connection's catalog this process.
fn listed() -> &'static Mutex<HashSet<String>> {
    static LISTED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    LISTED.get_or_init(Default::default)
}

/// `Some(true)` for a `VERDICT: PASS` line, `Some(false)` for FAIL, `None`
/// when the review gave no verdict. The merge gate reads "no verdict" as a
/// fail; here it is only reported.
pub fn verdict(text: &str) -> Option<bool> {
    let has = text.lines().any(|l| {
        l.trim()
            .trim_matches(|c: char| matches!(c, '*' | '#' | '`' | '_' | '>' | ' '))
            .to_ascii_uppercase()
            .starts_with("VERDICT")
    });
    has.then(|| crew::parse_verdict(text))
}

/// The review as the model working in the project will read it next.
fn carry(model: &str, review: &str) -> String {
    format!(
        "[second opinion from {model} — the user asked for it; written by that model, not \
         typed by the user]\n\n{review}\n\nWeigh these findings against the code. Fix what \
         the user asks you to fix, and say plainly where you disagree and why."
    )
}

/// Why a reviewer can't be used, for the chooser.
enum Unusable {
    /// Nobody chose yet.
    Unchosen,
    /// The choice exists but can't run; the reason is for the user.
    Because(String),
}

impl Agent {
    /// `/audit`: the user's chosen reviewer reviews the uncommitted work,
    /// after the user agrees to what it should cost. With no usable choice,
    /// asks the user to choose ([`AgentEvent::ReviewerNeeded`]) and runs
    /// nothing. Returns the review, or an empty string when none ran.
    pub async fn second_opinion(&mut self) -> Result<String> {
        self.audit(false).await
    }

    /// At the end of a build turn that changed files: offer an audit, with
    /// what it would cost, unless the user turned offers off. Saying yes
    /// runs it; there is no second question. Nothing when there is nothing
    /// uncommitted to review.
    pub async fn offer_audit(&mut self) -> Result<String> {
        let offers = self.cfg.as_ref().is_some_and(|c| c.ui.offer_audit);
        if !offers || self.role != Role::SoloBuild || self.ctx.user_io.is_none() {
            return Ok(String::new());
        }
        self.audit(true).await
    }

    /// `/audit model`: ask the user to choose again, priced for the work
    /// there is to review now.
    pub fn choose_reviewer(&mut self) -> Result<()> {
        let context_tokens = match self.review_job()? {
            Ok(job) => job.context_tokens,
            Err(_) => 0,
        };
        self.emit(AgentEvent::ReviewerNeeded {
            context_tokens,
            then_run: false,
            reason: String::new(),
        })
    }

    fn say(&mut self, message: impl Into<String>) -> Result<String> {
        self.emit(AgentEvent::Notice {
            message: message.into(),
        })?;
        Ok(String::new())
    }

    /// The diff and the conversation, as the reviewer will read them; `Err`
    /// says why there is nothing to review.
    fn review_job(&mut self) -> Result<std::result::Result<Job, String>> {
        let Ok(root) = crate::review::root(&self.ctx.workspace) else {
            return Ok(Err(
                "an audit reads what changed, which needs a git repository".into(),
            ));
        };
        let base = crate::review::head_base(&root);
        let changes = crate::review::changes(&root, &base)?;
        if changes.files.is_empty() {
            return Ok(Err("nothing uncommitted to review".into()));
        }
        let paths: Vec<String> = changes.files.iter().map(|f| f.path.clone()).collect();
        let diff = crate::review::diff_within(&root, &changes, &paths, DIFF_CHARS, FILE_CHARS);
        let body = format!(
            "The conversation so far (the user's requests and the other model's account):\n\n\
             {}\n\n---\n\nUncommitted changes, against HEAD:\n\n{diff}",
            self.conversation_digest(DIGEST_CHARS)
        );
        let mut messages = crate::prompt::reading_messages(
            &self.home,
            self.project_root.as_deref(),
            self.trusted,
            Role::Auditor,
            "",
            &body,
            &paths,
        );
        if let Some(system) = messages.first_mut() {
            system.content = crate::prompt::load(
                crate::prompt::PromptKind::Second,
                &self.home,
                self.project_root.as_deref(),
                self.trusted,
            );
        }
        let chars: usize = messages.iter().map(|m| m.content.len()).sum();
        let (added, removed) = changes.totals();
        Ok(Ok(Job {
            messages,
            files: paths.len(),
            added,
            removed,
            context_tokens: chars as u64 / 4,
        }))
    }

    /// Ask, then run. `offered`: Ryter is offering at the end of a build
    /// turn, so a "no" says nothing, and the prompt can stop the offers.
    async fn audit(&mut self, offered: bool) -> Result<String> {
        if !self.role.is_solo() {
            return self
                .say("audits are for solo mode; in crew mode every task already has an auditor");
        }
        let job = match self.review_job()? {
            Ok(job) => job,
            Err(_) if offered => return Ok(String::new()),
            Err(why) => return self.say(why),
        };
        let reviewer = match self.reviewer().await {
            Ok(r) => r,
            Err(why) => {
                let reason = match why {
                    Unusable::Unchosen => String::new(),
                    Unusable::Because(s) => s,
                };
                // Offered with no reviewer: ask whether to choose one, rather
                // than open a chooser nobody asked for.
                if offered {
                    let Some(io) = self.ctx.user_io.clone() else {
                        return Ok(String::new());
                    };
                    let why = if reason.is_empty() {
                        "nobody is chosen to audit yet".to_string()
                    } else {
                        reason.clone()
                    };
                    let answer = io.permission(
                        "audit offer",
                        &format!(
                            "Audit this work with a second model before you commit?\n{why}: \
                             y to choose a model and your limit per audit"
                        ),
                        &self.ctx.cancel,
                    );
                    if answer != crate::user_io::Permission::Allow {
                        return Ok(String::new());
                    }
                }
                self.emit(AgentEvent::ReviewerNeeded {
                    context_tokens: job.context_tokens,
                    then_run: true,
                    reason: if offered { String::new() } else { reason },
                })?;
                return Ok(String::new());
            }
        };
        let ReviewerConfig {
            connection,
            model,
            limit_usd,
        } = reviewer.choice.clone();

        // What the user agrees to before anything is spent.
        let past = history(&self.home, &model);
        let cost = if reviewer.local {
            "runs on this machine, $0".to_string()
        } else {
            let range = price_range(self.book.rates(&model), job.context_tokens)
                .map(format_range)
                .unwrap_or_else(|| "$?.??".into());
            let mut s = format!("about {range} of your ${limit_usd:.2} limit");
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
        let summary = format!(
            "{}{model} on {connection} (your choice)\nreviews {} file{}, +{} −{}, read-only\n{cost}",
            if offered {
                "Audit this work before you commit?\n"
            } else {
                ""
            },
            job.files,
            if job.files == 1 { "" } else { "s" },
            job.added,
            job.removed,
        );
        if let Some(io) = self.ctx.user_io.clone() {
            let tool = if offered { "audit offer" } else { "audit" };
            let answer = io.permission(tool, &summary, &self.ctx.cancel);
            if self.ctx.cancel.is_cancelled() {
                return Ok(String::new());
            }
            match answer {
                crate::user_io::Permission::Allow => {}
                _ if offered => return Ok(String::new()),
                _ => return self.say("audit not run"),
            }
        }

        // Only now is it a turn: busy, and `Esc` stops it. An offer
        // declined leaves the build turn's summary on screen.
        let turn = crate::agent::next_turn();
        let started = std::time::Instant::now();
        self.emit(AgentEvent::TurnStarted { turn })?;
        let out = self.run_audit(reviewer, job).await;
        if matches!(out, Err(Error::Cancelled)) {
            self.kill_all_children();
            let _ = self.emit(AgentEvent::Cancelled);
        }
        let _ = self.emit(AgentEvent::TurnFinished {
            turn,
            tools: 0,
            duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        });
        match out {
            Err(Error::Cancelled) => Ok(String::new()),
            other => other,
        }
    }

    async fn run_audit(&mut self, reviewer: Reviewer, job: Job) -> Result<String> {
        let ReviewerConfig {
            connection,
            model,
            limit_usd,
        } = reviewer.choice.clone();
        let cfg = self.cfg.clone().unwrap_or_default();
        let caps = Caps {
            session_usd: self.budget_usd,
            session_before: self.session.meta.spend_usd_total.unwrap_or(0.0),
            task_usd: limit_usd,
            task_tokens: 0,
        };
        let mut efforts = cfg.reasoning_effort.clone();
        efforts
            .entry(Role::Auditor.to_string())
            .or_insert_with(|| REVIEW_EFFORT.to_string());
        let mut meter = Meter::new(self.book.clone(), caps)
            .with_free(cfg.local_connections())
            .with_log(self.session.spend_path())
            .with_efforts(efforts, cfg.model_reasoning.clone());
        let progress = self.sink.clone().map(|sink| Progress {
            sink,
            id: crate::queue::new_sub_id(),
        });
        if let Some(sink) = &self.sink {
            meter = meter.with_sink(sink.clone());
        }
        // The review hat's gate: read, search, tests and linters; no writes,
        // installs, or formatting, in the user's own tree.
        let ctx = crate::tools::ToolContext {
            role: Role::SoloReview,
            ..self.ctx.clone()
        };
        let partial = Mutex::new(String::new());
        let bill = Bill {
            meter: &meter,
            task: TASK,
            connection: &connection,
            progress: progress.as_ref(),
            wrap_up_usd: (!reviewer.local).then_some(limit_usd),
            last_text: Some(&partial),
        };
        let ran = crew::run_specialist(
            reviewer.provider.as_ref(),
            &model,
            Role::Auditor,
            job.messages,
            &ctx,
            &bill,
        )
        .await;
        self.record_crew_spend(&meter)?;
        let total = meter.task(TASK);
        let spent = (!total.unpriced).then_some(total.usd);
        let mut stopped = String::new();
        let (text, cut_short) = match ran {
            Ok(t) => (t, false),
            // At the limit: what it had written by then is the review.
            Err(Error::TaskBudget(why)) => {
                stopped = why;
                (partial.lock().map(|p| p.clone()).unwrap_or_default(), true)
            }
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            // Retired, renamed, or refused. Never a quiet fallback to a model
            // the user didn't choose.
            Err(e) => {
                return self.say(format!(
                    "the auditor {model} failed: {e}. `/audit model` chooses another."
                ));
            }
        };
        remember(
            &self.home,
            &Past {
                model: model.clone(),
                context_tokens: job.context_tokens,
                usd: spent,
            },
        );
        let text = text.trim().to_string();
        if text.is_empty() {
            return self.say(if cut_short {
                format!(
                    "no audit: {stopped}. {} spent. `/audit model` can raise the limit.",
                    total.label()
                )
            } else {
                format!(
                    "{model} finished without writing an audit ({} spent)",
                    total.label()
                )
            });
        }
        let body = if cut_short {
            format!("{text}\n\n_Cut short at your ${limit_usd:.2} limit._")
        } else {
            text
        };
        self.emit(AgentEvent::SecondOpinion {
            model: model.clone(),
            connection: connection.clone(),
            verdict: verdict(&body),
            body: body.clone(),
            total_usd: spent,
        })?;
        self.session.set_second_opinion(&carry(&model, &body))?;
        Ok(body)
    }

    /// The user's chosen reviewer, ready to run, or why it can't be used.
    async fn reviewer(&mut self) -> std::result::Result<Reviewer, Unusable> {
        let cfg = self.cfg.clone().ok_or(Unusable::Unchosen)?;
        let choice = cfg.reviewer.clone().ok_or(Unusable::Unchosen)?;
        if crew::same_model(&choice.model, &self.model) {
            return Err(Unusable::Because(format!(
                "{} is the model doing the work now, so it can't be its second opinion",
                choice.model
            )));
        }
        let Some(conn) = cfg.connections.get(&choice.connection) else {
            return Err(Unusable::Because(format!(
                "the connection {} is gone",
                choice.connection
            )));
        };
        let local = conn.is_local();
        let provider: Arc<dyn Provider> = if choice.connection == self.connection {
            self.provider.clone()
        } else {
            let key = crate::config::resolve_secret(
                &cfg,
                &crate::ids::ConnectionId::new(&choice.connection),
            )
            .map_err(|e| Unusable::Because(format!("{}: {e}", choice.connection)))?;
            Arc::new(crate::llm::http_provider(conn, key))
        };
        // Priced and still listed: checked against the live catalog once
        // per process. A limit can't hold a model with no price.
        let known = listed().lock().is_ok_and(|l| l.contains(&choice.model));
        if !known && !local {
            if let Ok(models) = provider.list_models().await {
                if !models.is_empty() && !models.iter().any(|m| m.id == choice.model) {
                    return Err(Unusable::Because(format!(
                        "{} is no longer listed by {}",
                        choice.model, choice.connection
                    )));
                }
                self.book.ingest_model_info(&models);
            }
            if self.book.rates(&choice.model).is_some() {
                if let Ok(mut l) = listed().lock() {
                    l.insert(choice.model.clone());
                }
            }
        }
        if !local && self.book.rates(&choice.model).is_none() {
            return Err(Unusable::Because(format!(
                "no price is known for {}, so your ${:.2} limit can't be held to",
                choice.model, choice.limit_usd
            )));
        }
        Ok(Reviewer {
            provider,
            choice,
            local,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
