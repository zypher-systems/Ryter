//! `ryter doctor` — tty, config, connections, spend catalog, Landlock.

use std::io::IsTerminal;
use std::path::Path;
use std::process::Command;

use crate::config::{self, Config};
use crate::ids::ConnectionId;
use crate::sandbox::{self, SandboxProfile};
use crate::spend::{PriceBook, Usage, format_usd};

/// One diagnostic row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    /// Short label.
    pub name: String,
    /// Outcome.
    pub status: CheckStatus,
    /// Extra text (no secrets).
    pub detail: String,
}

/// Severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckStatus {
    /// Healthy.
    Ok,
    /// Degraded but usable.
    Warn,
    /// Will block a run.
    Fail,
}

impl CheckStatus {
    fn tag(self) -> &'static str {
        match self {
            Self::Ok => "ok  ",
            Self::Warn => "warn",
            Self::Fail => "fail",
        }
    }
}

/// Full report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// Ordered checks.
    pub checks: Vec<Check>,
}

impl Report {
    /// True when any check failed.
    pub fn failed(&self) -> bool {
        self.checks.iter().any(|c| c.status == CheckStatus::Fail)
    }

    /// Plain text for the CLI / `/doctor`.
    pub fn render(&self) -> String {
        let mut s = String::from("ryter doctor\n");
        for c in &self.checks {
            s.push_str("  ");
            s.push_str(c.status.tag());
            s.push(' ');
            s.push_str(&c.name);
            if !c.detail.is_empty() {
                s.push_str("  ");
                s.push_str(&c.detail);
            }
            s.push('\n');
        }
        s
    }
}

/// Inputs for a doctor pass. No network.
pub struct DoctorOpts<'a> {
    /// `~/.ryter` (or `RYTER_HOME`).
    pub home: &'a Path,
    /// Project cwd.
    pub cwd: &'a Path,
    /// Trusted-project flag.
    pub trusted: bool,
    /// Profile that a run would apply.
    pub sandbox: SandboxProfile,
}

/// Run every check.
pub fn run(opts: DoctorOpts<'_>) -> Report {
    let mut checks = Vec::new();

    checks.push(Check {
        name: "os".into(),
        status: CheckStatus::Ok,
        detail: std::env::consts::OS.into(),
    });

    let tty = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    checks.push(Check {
        name: "tty".into(),
        status: if tty {
            CheckStatus::Ok
        } else {
            CheckStatus::Warn
        },
        detail: if tty {
            "stdin+stdout".into()
        } else {
            "not a tty (TUI needs one; use -p)".into()
        },
    });

    let home_ok = std::fs::create_dir_all(opts.home).is_ok();
    checks.push(Check {
        name: "home".into(),
        status: if home_ok {
            CheckStatus::Ok
        } else {
            CheckStatus::Fail
        },
        detail: opts.home.display().to_string(),
    });

    let cfg = match config::load_at(opts.home, Some(opts.cwd), opts.trusted) {
        Ok(c) => {
            checks.push(Check {
                name: "config".into(),
                status: CheckStatus::Ok,
                detail: "loaded".into(),
            });
            Some(c)
        }
        Err(e) => {
            checks.push(Check {
                name: "config".into(),
                status: CheckStatus::Fail,
                detail: e.to_string(),
            });
            None
        }
    };

    if let Some(cfg) = &cfg {
        connection_checks(&mut checks, cfg);
        spend_checks(&mut checks, cfg);
    }

    project_trust_check(&mut checks, opts.cwd, opts.trusted);
    git_check(&mut checks);
    landlock_checks(&mut checks, opts.sandbox);

    Report { checks }
}

fn connection_checks(checks: &mut Vec<Check>, cfg: &Config) {
    for name in ["spacexai", "openrouter"] {
        let Some(conn) = cfg.connections.get(name) else {
            checks.push(Check {
                name: format!("connection {name}"),
                status: CheckStatus::Fail,
                detail: "missing built-in".into(),
            });
            continue;
        };
        let key =
            config::resolve_secret_with(cfg, &ConnectionId::new(name), |k| std::env::var(k).ok());
        let key_bit = match key {
            Ok(_) => "key=set".to_string(),
            Err(_) => format!(
                "key missing ({})",
                conn.env_key.as_deref().unwrap_or(if name == "spacexai" {
                    "XAI_API_KEY"
                } else {
                    "OPENROUTER_API_KEY"
                })
            ),
        };
        let status = if key_bit.starts_with("key=set") {
            CheckStatus::Ok
        } else {
            CheckStatus::Warn
        };
        let model = conn.default_model.as_deref().unwrap_or("-");
        checks.push(Check {
            name: format!("connection {name}"),
            status,
            detail: format!("{}  {model}  {key_bit}", conn.api_backend),
        });
    }
}

fn spend_checks(checks: &mut Vec<Check>, cfg: &Config) {
    let book = PriceBook::from_config(cfg);
    let usage = Usage {
        input_tokens: 1_000,
        output_tokens: 1_000,
        cached_tokens: 0,
    };
    match book.cost("grok-4.6", usage) {
        Some(v) if v > 0.0 => checks.push(Check {
            name: "spend catalog".into(),
            status: CheckStatus::Ok,
            detail: format!("grok-4.6 {}", format_usd(Some(v))),
        }),
        Some(_) => checks.push(Check {
            name: "spend catalog".into(),
            status: CheckStatus::Fail,
            detail: "grok-4.6 priced as $0.00".into(),
        }),
        None => checks.push(Check {
            name: "spend catalog".into(),
            status: CheckStatus::Fail,
            detail: "grok-4.6 unpriced".into(),
        }),
    }
    let unknown = book.cost("mystery-model-not-real", usage);
    let label = format_usd(unknown);
    checks.push(Check {
        name: "spend unknown".into(),
        status: if unknown.is_none() && label == "$?.??" {
            CheckStatus::Ok
        } else {
            CheckStatus::Fail
        },
        detail: format!("mystery-model is {label} (must not be $0.00)"),
    });
}

fn project_trust_check(checks: &mut Vec<Check>, cwd: &Path, trusted: bool) {
    let overlay = cwd.join(".ryter");
    if !overlay.is_dir() {
        return;
    }
    if trusted {
        checks.push(Check {
            name: "project".into(),
            status: CheckStatus::Ok,
            detail: ".ryter/ trusted".into(),
        });
    } else {
        checks.push(Check {
            name: "project".into(),
            status: CheckStatus::Warn,
            detail: ".ryter/ present but untrusted (ryter trust)".into(),
        });
    }
}

fn git_check(checks: &mut Vec<Check>) {
    match Command::new("git").arg("--version").output() {
        Ok(o) if o.status.success() => {
            let v = String::from_utf8_lossy(&o.stdout);
            checks.push(Check {
                name: "git".into(),
                status: CheckStatus::Ok,
                detail: v.trim().to_string(),
            });
        }
        _ => checks.push(Check {
            name: "git".into(),
            status: CheckStatus::Warn,
            detail: "not found (builders need git worktrees)".into(),
        }),
    }
}

fn landlock_checks(checks: &mut Vec<Check>, profile: SandboxProfile) {
    let probe = sandbox::probe();
    let available = probe == "available";
    let status = if available {
        CheckStatus::Ok
    } else if profile == SandboxProfile::Off {
        CheckStatus::Warn
    } else {
        CheckStatus::Fail
    };
    checks.push(Check {
        name: "landlock".into(),
        status,
        detail: probe,
    });
    checks.push(Check {
        name: "sandbox".into(),
        status: if profile != SandboxProfile::Off && !available {
            CheckStatus::Fail
        } else {
            CheckStatus::Ok
        },
        detail: profile.as_str().into(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn report_includes_builtins_and_honest_unknown_price() {
        let home = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        let r = run(DoctorOpts {
            home: home.path(),
            cwd: cwd.path(),
            trusted: false,
            sandbox: SandboxProfile::Off,
        });
        let text = r.render();
        assert!(text.contains("spacexai"), "{text}");
        assert!(text.contains("openrouter"), "{text}");
        assert!(text.contains("grok-4.6"), "{text}");
        assert!(text.contains("$?"), "{text}");
        assert!(text.contains("landlock"), "{text}");
        assert!(text.contains("sandbox"), "{text}");
        assert!(!text.to_lowercase().contains("xai-"));
        assert!(!r.failed(), "{text}");
    }

    #[test]
    fn untrusted_project_overlay_is_a_warning() {
        let home = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        std::fs::create_dir_all(cwd.path().join(".ryter")).unwrap();
        let r = run(DoctorOpts {
            home: home.path(),
            cwd: cwd.path(),
            trusted: false,
            sandbox: SandboxProfile::Off,
        });
        let text = r.render();
        assert!(text.contains("untrusted"), "{text}");
        assert!(!r.failed(), "{text}");
    }
}
