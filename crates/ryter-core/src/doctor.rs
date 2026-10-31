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
        connection_checks(&mut checks, cfg, opts.home);
        spend_checks(&mut checks, cfg, opts.home);
    }

    project_trust_check(&mut checks, opts.cwd, opts.trusted);
    git_check(&mut checks);
    landlock_checks(&mut checks, opts.sandbox);

    Report { checks }
}

fn connection_checks(checks: &mut Vec<Check>, cfg: &Config, home: &Path) {
    for name in ["spacexsi", "openrouter", "anthropic", "openai"] {
        let Some(conn) = cfg.connections.get(name) else {
            checks.push(Check {
                name: format!("connection {name}"),
                status: CheckStatus::Fail,
                detail: "missing built-in".into(),
            });
            continue;
        };
        // The home the doctor was given, so a saved key is read from there
        // and nowhere else.
        let key = config::resolve_secret_in(cfg, &ConnectionId::new(name), home, |k| {
            std::env::var(k).ok()
        });
        let key_bit = match key {
            Ok(_) => "key=set".to_string(),
            Err(_) => format!(
                "key missing ({})",
                conn.env_key
                    .as_deref()
                    .or_else(|| config::well_known_env(&conn.kind))
                    .unwrap_or("-")
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

fn spend_checks(checks: &mut Vec<Check>, cfg: &Config, home: &Path) {
    let book = PriceBook::load(cfg, home);
    // The pulled catalog: what prices a model the shipped tables don't know.
    let pulled = crate::prices::load(home);
    checks.push(Check {
        name: "price catalog".into(),
        status: CheckStatus::Ok,
        detail: match (&pulled, cfg.spend.catalog) {
            (Some(c), _) => format!(
                "{} models from {} · pulled {}{}",
                c.providers.values().map(|m| m.len()).sum::<usize>(),
                if c.source.is_empty() { "the catalog" } else { &c.source },
                crate::prices::age(c.fetched),
                if cfg.spend.catalog { "" } else { " · [spend] catalog = false: not read or refreshed" }
            ),
            (None, true) => "not pulled yet · fetched once a day at launch; the shipped tables price what they know".into(),
            (None, false) => "off ([spend] catalog = false) · the shipped tables price what they know".into(),
        },
    });
    let usage = Usage {
        input_tokens: 1_000,
        output_tokens: 1_000,
        cached_tokens: 0,
        cache_write_tokens: 0,
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
        detail: format!("a model with no known price shows {label}, never $0.00"),
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
            detail: "not found (undo, /changes, /commit and reviews need git)".into(),
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
    let (covers, leaves) = sandbox::coverage(profile, available, sandbox::signal_scoping());
    checks.push(Check {
        name: "sandbox covers".into(),
        status: CheckStatus::Ok,
        detail: covers,
    });
    if let Some(leaves) = leaves {
        checks.push(Check {
            name: "sandbox leaves".into(),
            status: CheckStatus::Ok,
            detail: leaves,
        });
    }
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
        assert!(text.contains("spacexsi"), "{text}");
        assert!(text.contains("openrouter"), "{text}");
        assert!(text.contains("grok-4.6"), "{text}");
        for (name, var) in [
            ("anthropic", "ANTHROPIC_API_KEY"),
            ("openai", "OPENAI_API_KEY"),
        ] {
            assert!(text.contains(&format!("connection {name}")), "{text}");
            assert!(text.contains(var), "{text}");
        }
        assert!(text.contains("$?"), "{text}");
        assert!(text.contains("landlock"), "{text}");
        assert!(text.contains("sandbox"), "{text}");
        assert!(!text.to_lowercase().contains("xai-"));
        assert!(!r.failed(), "{text}");
    }

    /// A key saved under the doctor's home is seen (and never printed), so
    /// the report is about that home and not whatever this machine holds.
    #[test]
    fn a_saved_key_is_read_from_the_doctors_home() {
        let home = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        std::fs::create_dir_all(home.path().join("keys")).unwrap();
        std::fs::write(home.path().join("keys").join("openai"), "sk-test\n").unwrap();
        let r = run(DoctorOpts {
            home: home.path(),
            cwd: cwd.path(),
            trusted: false,
            sandbox: SandboxProfile::Off,
        });
        let text = r.render();
        let row = |name: &str| {
            text.lines()
                .find(|l| l.contains(&format!("connection {name}")))
                .unwrap_or_else(|| panic!("no {name} row in {text}"))
                .to_string()
        };
        assert!(row("openai").contains("key=set"), "{text}");
        assert!(!text.contains("sk-test"), "{text}");
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
