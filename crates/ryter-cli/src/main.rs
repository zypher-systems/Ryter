//! Ryter CLI. `--version` must not open config, keyring, or the network.

use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::mpsc;

use clap::{Parser, Subcommand};
use ryter_core::config::{self, resolve_secret};
use ryter_core::ids::ConnectionId;
use ryter_core::llm::http_provider;
use ryter_core::sandbox::{self, SandboxProfile};
use ryter_core::session::Session;
use ryter_core::spend::{PriceBook, format_usd};
use ryter_core::tools::ToolContext;
use ryter_core::{Agent, AgentEvent, Error, HookSet, Phase, Provider, Role, VERSION};

#[derive(Parser)]
#[command(
    name = "ryter",
    version = VERSION,
    about = "Ryter — terminal AI coding harness",
    disable_help_subcommand = true
)]
struct Cli {
    /// Headless prompt (one turn, then exit).
    #[arg(short = 'p', long = "prompt")]
    prompt: Option<String>,

    /// NDJSON AgentEvent stream on stdout (with `-p`).
    #[arg(long)]
    json: bool,

    /// Continue the latest session in this directory (with `-p`): its
    /// transcript, task queue, and open patch.
    #[arg(short = 'c', long = "continue")]
    resume: bool,

    /// build | plan | review (one model), or crew (the lead and its crew).
    /// Default: build, or the mode a continued session was left in.
    #[arg(long)]
    hat: Option<String>,

    /// Treat Ask as Allow. Deny still wins.
    #[arg(long)]
    always_approve: bool,

    /// Connection name.
    #[arg(long)]
    connection: Option<String>,

    /// Model id.
    #[arg(short = 'm', long)]
    model: Option<String>,

    /// Landlock profile: off, workspace, read-only. Default from config (`off`).
    #[arg(long)]
    sandbox: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Print the version and exit.
    Version,
    /// Print spend for a session (default: latest in this directory).
    Spend {
        /// Session id.
        session: Option<String>,
        /// This project (its git repository) across every session.
        #[arg(long)]
        project: bool,
    },
    /// MCP: inbound server or echo helper.
    Mcp {
        #[command(subcommand)]
        cmd: McpCmd,
    },
    /// Inbound MCP on a unix socket or TCP (attach / daemon).
    Serve {
        /// Unix socket path (default `~/.ryter/ryter.sock`).
        #[arg(long)]
        socket: Option<std::path::PathBuf>,
        /// TCP bind address, e.g. `127.0.0.1:8765`.
        #[arg(long)]
        bind: Option<String>,
        /// Shared secret for TCP (`initialize.params.token`). Or `RYTER_MCP_TOKEN`.
        #[arg(long)]
        token: Option<String>,
        /// Allow binding `0.0.0.0` / `::`.
        #[arg(long = "i-mean-it")]
        i_mean_it: bool,
    },
    /// Check tty, keys, spend catalog, Landlock. No network.
    Doctor,
    /// Trust this directory's `.ryter/` overlay (skills, hooks, project config).
    Trust,
    /// List sessions for this directory.
    Sessions,
    /// Open the TUI on a saved session (`latest` if omitted).
    Resume {
        /// Session id or unique prefix.
        id: Option<String>,
    },
    /// List or configure BYOK connections.
    Connections {
        #[command(subcommand)]
        cmd: Option<ConnCmd>,
    },
    /// List models on a connection (default: last-used).
    Models {
        /// Connection name.
        connection: Option<String>,
    },
    /// Run the benchmark suite through the real crew: what lands, what passes
    /// the hidden tests, and what it costs. Spends real money on your keys.
    Bench {
        /// Suite directory.
        #[arg(long, default_value = "bench")]
        suite: std::path::PathBuf,
        /// Only these tasks (repeatable).
        #[arg(long)]
        only: Vec<String>,
        /// Run with a saved crew preset instead of the current crew, to
        /// compare tierings.
        #[arg(long)]
        crew: Option<String>,
        /// Spend cap per task, in USD.
        #[arg(long, default_value_t = 1.0)]
        budget_usd: f64,
        /// Run each task this many times (models vary run to run).
        #[arg(long, default_value_t = 1)]
        repeat: u32,
    },
    /// Crew model assignments.
    Crew {
        #[command(subcommand)]
        cmd: CrewCmd,
    },
}

#[derive(Subcommand)]
enum CrewCmd {
    /// Suggest a crew from every model you can reach. Tiers: skiff (low
    /// cost), schooner (balanced, the default), galleon (high cost).
    Suggest {
        /// skiff | schooner | galleon (or low | medium | high).
        #[arg(long, default_value = "schooner")]
        tier: String,
        /// Save it to ~/.ryter/crew.toml (the current crew is kept as the
        /// `before-suggest` preset).
        #[arg(long)]
        apply: bool,
    },
    /// Show all three tiers side by side, from the models you can reach.
    Tiers,
    /// Send each crew model (and the lead) one tiny request with a tool, to
    /// catch a data policy, missing tool support, access, or credits. Costs
    /// well under a cent.
    Check,
}

#[derive(Subcommand)]
enum ConnCmd {
    /// Store an API key (keyring, or ~/.ryter/keys/<name>).
    SetKey {
        /// Connection name (`spacexai`, `openrouter`, …).
        name: String,
    },
    /// Add a user connection (`~/.ryter/connections.toml`).
    Add {
        /// Name (`local`, `work`, …).
        name: String,
        /// Kind: spacexai, openrouter, openai, anthropic, or a local server:
        /// ollama, lmstudio, llamacpp (no key needed, no API cost).
        #[arg(long)]
        kind: String,
        /// Override the template base URL.
        #[arg(long)]
        base_url: Option<String>,
        /// Default model id.
        #[arg(long)]
        model: Option<String>,
    },
    /// Remove a user connection (built-ins stay).
    Remove {
        /// Name.
        name: String,
    },
    /// List models (needs a key).
    Test {
        /// Connection name.
        name: String,
    },
}

#[derive(Subcommand)]
enum McpCmd {
    /// Speak MCP on stdio (other agents spawn this).
    Serve,
    /// Tiny echo MCP server for tests.
    Echo,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Version) => {
            println!("ryter {VERSION}");
            ExitCode::SUCCESS
        }
        None if cli.prompt.is_none() => {
            if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
                eprintln!("ryter: not a tty (use -p for headless, --version for version)");
                return ExitCode::from(1);
            }
            match ryter_tui::run(ryter_tui::TuiOpts {
                always_approve: cli.always_approve,
                connection: cli.connection,
                model: cli.model,
                phase: None,
                sandbox: cli.sandbox,
                session: None,
            }) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::from(1)
                }
            }
        }
        Some(Command::Spend { project: true, .. }) => match project_spend_cmd() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(1)
            }
        },
        Some(Command::Spend { session, .. }) => match spend_cmd(session.as_deref()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(1)
            }
        },
        Some(Command::Mcp { cmd }) => match mcp_cmd(cmd) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(1)
            }
        },
        Some(Command::Serve {
            socket,
            bind,
            token,
            i_mean_it,
        }) => match serve_cmd(socket, bind, token, i_mean_it) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(1)
            }
        },
        Some(Command::Connections { cmd }) => match connections_cmd(cmd) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(1)
            }
        },
        Some(Command::Bench {
            suite,
            only,
            crew,
            budget_usd,
            repeat,
        }) => match bench_cmd(&suite, &only, crew.as_deref(), budget_usd, repeat) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(1)
            }
        },
        Some(Command::Crew {
            cmd: CrewCmd::Suggest { tier, apply },
        }) => match crew_suggest_cmd(&tier, apply) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(1)
            }
        },
        Some(Command::Crew {
            cmd: CrewCmd::Check,
        }) => match crew_check_cmd() {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::from(1),
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(1)
            }
        },
        Some(Command::Crew {
            cmd: CrewCmd::Tiers,
        }) => match crew_tiers_cmd() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(1)
            }
        },
        Some(Command::Models { connection }) => match models_cmd(connection.as_deref()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(1)
            }
        },
        Some(Command::Sessions) => match sessions_cmd() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(1)
            }
        },
        Some(Command::Resume { id }) => {
            if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
                eprintln!("ryter resume: not a tty");
                return ExitCode::from(1);
            }
            match ryter_tui::run(ryter_tui::TuiOpts {
                always_approve: cli.always_approve,
                connection: cli.connection,
                model: cli.model,
                phase: None,
                sandbox: cli.sandbox,
                session: Some(id.unwrap_or_else(|| "latest".into())),
            }) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::from(1)
                }
            }
        }
        Some(Command::Trust) => match trust_cmd() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(1)
            }
        },
        Some(Command::Doctor) => match doctor_cmd() {
            Ok(ok) => {
                if ok {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::from(1)
                }
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(1)
            }
        },
        _ => match run_headless(cli) {
            Ok(code) => code,
            Err(e) => {
                eprintln!("{e}");
                if matches!(e, Error::Budget { .. }) {
                    ExitCode::from(3)
                } else {
                    ExitCode::from(1)
                }
            }
        },
    }
}

fn resolve_sandbox(
    flag: Option<&str>,
    cfg: &ryter_core::Config,
) -> ryter_core::Result<SandboxProfile> {
    if let Some(s) = flag {
        s.parse()
    } else {
        cfg.sandbox.profile()
    }
}

fn run_headless(cli: Cli) -> ryter_core::Result<ExitCode> {
    let cwd = std::env::current_dir().map_err(|e| Error::Io(e.to_string()))?;
    let trusted = config::is_trusted(&cwd);
    let cfg = config::load(Some(&cwd), trusted)?;
    let profile = resolve_sandbox(cli.sandbox.as_deref(), &cfg)?;
    let rt = sandbox::runtime(profile).map_err(|e| Error::Io(e.to_string()))?;
    rt.block_on(run_prompt(cli, cfg, cwd, profile))
}

async fn run_prompt(
    cli: Cli,
    cfg: ryter_core::Config,
    cwd: std::path::PathBuf,
    profile: SandboxProfile,
) -> ryter_core::Result<ExitCode> {
    let prompt = cli
        .prompt
        .ok_or_else(|| Error::Config("missing -p/--prompt".into()))?;
    if !cfg.spend.enabled {
        eprintln!("warning: [spend] enabled = false; counters still increment");
    }
    let home = config::home_dir();
    let last = config::load_last_route(&home);
    let (conn_name, model) = config::resolve_route(
        &cfg,
        last.as_ref(),
        cli.connection.as_deref(),
        cli.model.as_deref(),
    );
    let conn = cfg
        .connections
        .get(&conn_name)
        .ok_or_else(|| Error::Config(format!("unknown connection {conn_name}")))?;
    // Sessions still record a phase for compatibility; nothing routes on it.
    let phase = Phase::Build;
    let key = resolve_secret(&cfg, &ConnectionId::new(&conn_name))?;
    let provider = http_provider(conn, key);
    let _ = config::save_last_route(&home, &config::LastRoute::new(&conn_name, &model));
    let trusted = config::is_trusted(&cwd);
    // A budget stop tells the user to continue; headless could only start over.
    let mut session = if cli.resume {
        Session::latest(&home, &cwd)?
            .ok_or_else(|| Error::Config("no session in this directory to continue".into()))?
    } else {
        Session::create(&home, &cwd, phase, conn_name.clone(), model.clone())?
    };
    session.set_auditor(cfg.auditor.enabled)?;
    sandbox::apply(profile, &cwd, &home)?;
    let notes = session.notes_dir();
    let queue = std::sync::Arc::new(std::sync::Mutex::new(ryter_core::queue::TaskQueue::open(
        session.dir.join("tasks.json"),
    )));
    let (tx, rx) = mpsc::channel();
    let json = cli.json;
    let printer = std::thread::spawn(move || {
        while let Ok(ev) = rx.recv() {
            if json {
                if let Ok(line) = serde_json::to_string(&ev) {
                    println!("{line}");
                }
            } else if let AgentEvent::Token { text } = ev {
                let _ = io::stdout().write_all(text.as_bytes());
                let _ = io::stdout().flush();
            } else if let AgentEvent::Notice { message } = ev {
                eprintln!("ryter: {message}");
            }
        }
    });
    let role = match cli.hat.as_deref() {
        Some(h) => match h.parse::<Role>() {
            Ok(r) if r.is_solo() || r == Role::Orchestrator => r,
            _ => {
                return Err(Error::Config(format!(
                    "unknown hat {h:?}: build, plan, review, or crew"
                )));
            }
        },
        None => session.meta.mode.unwrap_or(Role::SoloBuild),
    };
    let _ = session.set_mode(role);
    let mut agent = Agent {
        provider: Arc::new(provider),
        book: PriceBook::from_config(&cfg),
        session,
        ctx: ToolContext {
            workspace: cwd.clone(),
            notes_dir: notes,
            role,
            always_approve: cli.always_approve,
            queue: queue.clone(),
            mcp: None,
            hooks: if cfg.hooks.is_empty() {
                None
            } else {
                Some(Arc::new(HookSet::from_config(&cfg.hooks)))
            },
            cancel: ryter_core::Cancel::new(),
            user_io: None,
            sticky_approve: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            web: cfg.features.web,
        },
        connection: conn_name,
        model,
        role,
        max_turns: 40,
        budget_usd: cfg.spend.session_budget_usd,
        sink: Some(tx),
        home,
        project_root: Some(cwd),
        trusted,
        queue,
        max_crew: cfg.subagents.max,
        max_retries: cfg.auditor.max_retries,
        checks: cfg.auditor.checks.clone(),
        check_timeout_secs: cfg.auditor.check_timeout_secs,
        context_window: 0,
        cfg: Some(cfg.clone()),
        running: Arc::new(std::sync::Mutex::new(Vec::new())),
    };
    agent.fire_session_start()?;
    let result = agent.turn(&prompt).await;
    drop(agent.sink.take());
    let _ = printer.join();
    match result {
        Ok(r) => {
            if !cli.json {
                if !r.text.is_empty() && !r.text.ends_with('\n') {
                    println!();
                }
                if let Some(total) = agent.session.meta.spend_usd_total {
                    eprintln!(
                        "spend {}  session {}",
                        format_usd(Some(total)),
                        agent.session.meta.id
                    );
                } else if agent.session.meta.spend_unknown {
                    eprintln!(
                        "spend {}  session {}",
                        format_usd(None),
                        agent.session.meta.id
                    );
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        Err(Error::Budget { spent, cap }) => {
            eprintln!("budget exceeded (${spent:.4} >= ${cap:.2})");
            Err(Error::Budget { spent, cap })
        }
        Err(e) => Err(e),
    }
}

fn connections_cmd(cmd: Option<ConnCmd>) -> ryter_core::Result<()> {
    let cwd = std::env::current_dir().map_err(|e| Error::Io(e.to_string()))?;
    let trusted = config::is_trusted(&cwd);
    let cfg = config::load(Some(&cwd), trusted)?;
    match cmd {
        None => {
            for (name, c) in &cfg.connections {
                let key = if config::has_secret(&cfg, name) {
                    "key set"
                } else {
                    "key missing"
                };
                let model = c.default_model.as_deref().unwrap_or("-");
                let mark = if *name == cfg.default_connection {
                    "*"
                } else {
                    " "
                };
                println!("{mark} {name:12}  {}  {model}  {key}", c.kind);
            }
            println!("set a key: ryter connections set-key <name>");
            Ok(())
        }
        Some(ConnCmd::SetKey { name }) => {
            if !cfg.connections.contains_key(&name) {
                return Err(Error::Config(format!("unknown connection {name}")));
            }
            eprint!("API key for {name}: ");
            let _ = io::stderr().flush();
            let mut line = String::new();
            io::stdin()
                .read_line(&mut line)
                .map_err(|e| Error::Io(e.to_string()))?;
            let store = config::store_secret_at(&config::home_dir(), &name, line.trim())?;
            println!("saved key for {name} in {store}");
            Ok(())
        }
        Some(ConnCmd::Add {
            name,
            kind,
            base_url,
            model,
        }) => {
            if cfg.connections.contains_key(&name) {
                return Err(Error::Config(format!("connection {name} already exists")));
            }
            let mut row = config::connection_template(&kind)?;
            if let Some(u) = base_url {
                row.base_url = u;
            }
            if let Some(m) = model {
                row.default_model = Some(m);
            }
            let home = config::home_dir();
            let mut extra = config::load_user_connections(&home);
            extra.insert(name.clone(), row);
            config::save_user_connections(&home, &extra)?;
            println!("added {name}  set a key: ryter connections set-key {name}");
            Ok(())
        }
        Some(ConnCmd::Remove { name }) => {
            let home = config::home_dir();
            let mut extra = config::load_user_connections(&home);
            if extra.remove(&name).is_none() {
                return Err(Error::Config(format!(
                    "{name} is not a user connection (built-ins cannot be removed)"
                )));
            }
            config::save_user_connections(&home, &extra)?;
            println!("removed {name}");
            Ok(())
        }
        Some(ConnCmd::Test { name }) => models_cmd(Some(&name)),
    }
}

fn bench_cmd(
    suite: &std::path::Path,
    only: &[String],
    crew: Option<&str>,
    budget_usd: f64,
    repeat: u32,
) -> ryter_core::Result<()> {
    use ryter_core::bench::{BenchEnv, Summary, load_suite, run_task};
    let cwd = std::env::current_dir().map_err(|e| Error::Io(e.to_string()))?;
    let trusted = config::is_trusted(&cwd);
    let mut cfg = config::load(Some(&cwd), trusted)?;
    let home = config::home_dir();
    if let Some(name) = crew {
        config::load_crew_preset(&home, &mut cfg, name)?;
    }
    let mut tasks = load_suite(suite)?;
    if !only.is_empty() {
        tasks.retain(|t| only.contains(&t.name));
    }
    if tasks.is_empty() {
        return Err(Error::Config(format!("no tasks in {}", suite.display())));
    }
    let last = config::load_last_route(&home);
    let (connection, model) = config::resolve_route(&cfg, last.as_ref(), None, None);
    let conn = cfg
        .connections
        .get(&connection)
        .ok_or_else(|| Error::Config(format!("unknown connection {connection}")))?;
    let key = resolve_secret(&cfg, &ConnectionId::new(&connection))?;
    let provider: Arc<dyn ryter_core::Provider> = Arc::new(http_provider(conn, key));
    let (_, builder) = cfg.route_for(Role::Builder);
    let (_, auditor) = cfg.route_for(Role::Auditor);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let run_home = home.join("bench").join(stamp.to_string());
    std::fs::create_dir_all(&run_home).map_err(|e| Error::Io(e.to_string()))?;
    let results_path = run_home.join("results.jsonl");
    println!("crew      lead {model} · builder {builder} · auditor {auditor}");
    println!(
        "running   {} task(s) × {repeat}, capped at ${budget_usd:.2} each — this spends real money",
        tasks.len()
    );
    let env = BenchEnv {
        cfg,
        provider,
        connection,
        model: model.clone(),
        home: run_home.clone(),
        budget_usd,
        accept_timeout: std::time::Duration::from_secs(600),
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Error::Io(e.to_string()))?;
    let mut results = Vec::new();
    for task in &tasks {
        for _ in 0..repeat {
            // A real task takes minutes; say what is running.
            print!("{:<24} running…\r", task.name);
            let _ = io::stdout().flush();
            let r = rt.block_on(run_task(task, &env));
            let mark = match (r.landed, r.accepted) {
                (true, true) => "accepted",
                (true, false) => "FALSE PASS",
                _ => "not landed",
            };
            let bound = if r.unpriced { "≥" } else { "" };
            println!(
                "{:<24} {mark:<11} {bound}${:.3}  {:>7} tok  {:>5.0}s  {}",
                r.task, r.usd, r.billable_tokens, r.secs, r.outcome
            );
            let line = serde_json::json!({
                "lead": model, "builder": builder, "auditor": auditor, "result": r,
            });
            use std::io::Write as _;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&results_path)
            {
                let _ = writeln!(f, "{line}");
            }
            let paused = r.outcome.starts_with("builds paused");
            results.push(r);
            if paused {
                eprintln!("stopping: the crew cannot run until the auditor is a different model");
                println!("\n{}", Summary::of(&results).render());
                return Ok(());
            }
        }
    }
    println!("\n{}", Summary::of(&results).render());
    println!("results   {}", results_path.display());
    Ok(())
}

/// The lead's route and every model the user can reach, for tier suggestions.
struct Reach {
    cfg: ryter_core::Config,
    home: std::path::PathBuf,
    lead_conn: String,
    lead_model: String,
    models: Vec<ryter_core::llm::ModelInfo>,
}

fn reach() -> ryter_core::Result<Reach> {
    let cwd = std::env::current_dir().map_err(|e| Error::Io(e.to_string()))?;
    let trusted = config::is_trusted(&cwd);
    let cfg = config::load(Some(&cwd), trusted)?;
    let home = config::home_dir();
    let last = config::load_last_route(&home);
    let (lead_conn, lead_model) = config::resolve_route(&cfg, last.as_ref(), None, None);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Error::Io(e.to_string()))?;
    let (models, failed) = rt.block_on(ryter_core::tiering::reachable_models(&cfg));
    for f in &failed {
        eprintln!("skipped {f}");
    }
    Ok(Reach {
        cfg,
        home,
        lead_conn,
        lead_model,
        models,
    })
}

/// Test the lead and every crew seat; true when all answered.
fn crew_check_cmd() -> ryter_core::Result<bool> {
    let cwd = std::env::current_dir().map_err(|e| Error::Io(e.to_string()))?;
    let trusted = config::is_trusted(&cwd);
    let cfg = config::load(Some(&cwd), trusted)?;
    let home = config::home_dir();
    let last = config::load_last_route(&home);
    let (lead_conn, lead_model) = config::resolve_route(&cfg, last.as_ref(), None, None);
    let mut seats = vec![("lead".to_string(), lead_conn.clone(), lead_model.clone())];
    for role in ["architect", "builder", "auditor"] {
        let r = cfg.specialists.get(role);
        let conn = r
            .and_then(|r| r.connection.clone())
            .unwrap_or_else(|| lead_conn.clone());
        let model = r
            .and_then(|r| r.model.clone())
            .unwrap_or_else(|| lead_model.clone());
        seats.push((role.to_string(), conn, model));
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Error::Io(e.to_string()))?;
    let mut ok = true;
    let mut seen: std::collections::HashMap<(String, String), Result<(), String>> =
        std::collections::HashMap::new();
    for (role, conn, model) in seats {
        let key = (conn.clone(), model.clone());
        if !seen.contains_key(&key) {
            let r = match (
                cfg.connections.get(&conn),
                resolve_secret(&cfg, &ConnectionId::new(&conn)),
            ) {
                (Some(c), Ok(k)) => {
                    rt.block_on(ryter_core::tiering::probe(&http_provider(c, k), &model))
                }
                (None, _) => Err(format!("unknown connection {conn}")),
                (_, Err(_)) => Err(format!("no key for {conn}")),
            };
            seen.insert(key.clone(), r);
        }
        match &seen[&key] {
            Ok(()) => println!("✓ {role:<10}{model} on {conn}"),
            Err(e) => {
                ok = false;
                println!("✗ {role:<10}{model} on {conn}: {e}");
            }
        }
    }
    Ok(ok)
}

fn crew_tiers_cmd() -> ryter_core::Result<()> {
    use ryter_core::tiering::{Tier, suggest_tier};
    let r = reach()?;
    println!(
        "lead      {} on {} (unchanged by any tier)\n",
        r.lead_model, r.lead_conn
    );
    for tier in Tier::ALL {
        let t = suggest_tier(
            tier,
            &r.lead_conn,
            &r.lead_model,
            &r.models,
            &r.cfg.local_connections(),
        );
        println!("{} — {}: {}", tier.name(), tier.cost(), tier.tagline());
        for (role, p) in [
            ("builder", &t.builder),
            ("auditor", &t.auditor),
            ("architect", &t.architect),
        ] {
            let label = p
                .as_ref()
                .map(ryter_core::tiering::Pick::label)
                .unwrap_or_else(|| "(no suitable model)".into());
            println!("  {role:<10}{label}");
        }
        println!();
    }
    println!("Use one: `ryter crew suggest --tier <name> --apply`, or pick it in /crew.");
    Ok(())
}

fn crew_suggest_cmd(tier: &str, apply: bool) -> ryter_core::Result<()> {
    let tier = ryter_core::tiering::Tier::parse(tier).ok_or_else(|| {
        Error::Config(format!(
            "unknown tier {tier:?}: skiff (low), schooner (balanced), galleon (high)"
        ))
    })?;
    let Reach {
        cfg,
        home,
        lead_conn,
        lead_model,
        models,
    } = reach()?;
    let t = ryter_core::tiering::suggest_tier(
        tier,
        &lead_conn,
        &lead_model,
        &models,
        &cfg.local_connections(),
    );
    println!("{} — {}: {}", tier.name(), tier.cost(), tier.tagline());
    println!("lead      {lead_model} on {lead_conn} (unchanged)");
    print!("{}", t.render());
    if !apply {
        println!(
            "\nRun `ryter crew suggest --tier {} --apply` to use it, or pick it in /crew.",
            tier.name()
        );
        return Ok(());
    }
    if t.auditor.is_none() {
        return Err(Error::Config(
            "not applied: no model qualifies as an independent auditor".into(),
        ));
    }
    config::save_crew_preset(&home, "before-suggest", &cfg.specialists)?;
    let mut rows = cfg.specialists.clone();
    rows.extend(t.as_specialists());
    config::save_crew(&home, &rows)?;
    println!(
        "\nApplied to ~/.ryter/crew.toml. Your previous crew is the `before-suggest` preset in /crew."
    );
    Ok(())
}

fn models_cmd(connection: Option<&str>) -> ryter_core::Result<()> {
    let cwd = std::env::current_dir().map_err(|e| Error::Io(e.to_string()))?;
    let trusted = config::is_trusted(&cwd);
    let cfg = config::load(Some(&cwd), trusted)?;
    let home = config::home_dir();
    let last = config::load_last_route(&home);
    let (conn_name, _) = config::resolve_route(&cfg, last.as_ref(), connection, None);
    let conn = cfg
        .connections
        .get(&conn_name)
        .ok_or_else(|| Error::Config(format!("unknown connection {conn_name}")))?;
    let key = resolve_secret(&cfg, &ConnectionId::new(&conn_name))?;
    let provider = http_provider(conn, key);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Error::Io(e.to_string()))?;
    let models = rt.block_on(provider.list_models())?;
    if models.is_empty() {
        println!("{conn_name}: no models returned");
        return Ok(());
    }
    println!("{conn_name}");
    for m in models {
        let price = match (m.input_per_million, m.output_per_million) {
            (Some(i), Some(o)) => format!("  ${i}/{o}"),
            _ => String::new(),
        };
        println!("  {}{price}", m.id);
    }
    Ok(())
}

fn trust_cmd() -> ryter_core::Result<()> {
    let cwd = std::env::current_dir().map_err(|e| Error::Io(e.to_string()))?;
    config::trust(&cwd)?;
    println!("trusted {}", cwd.display());
    Ok(())
}

fn doctor_cmd() -> ryter_core::Result<bool> {
    let cwd = std::env::current_dir().map_err(|e| Error::Io(e.to_string()))?;
    let home = config::home_dir();
    let trusted = config::is_trusted(&cwd);
    let sandbox = config::load(Some(&cwd), trusted)
        .ok()
        .and_then(|c| c.sandbox.profile().ok())
        .unwrap_or(SandboxProfile::Off);
    let report = ryter_core::doctor::run(ryter_core::doctor::DoctorOpts {
        home: &home,
        cwd: &cwd,
        trusted,
        sandbox,
    });
    print!("{}", report.render());
    Ok(!report.failed())
}

fn mcp_cmd(cmd: McpCmd) -> ryter_core::Result<()> {
    match cmd {
        McpCmd::Echo => ryter_core::serve_echo(),
        McpCmd::Serve => mcp_serve(),
    }
}

struct ServeHost {
    agent: std::sync::Mutex<Agent>,
    rt: std::sync::Mutex<tokio::runtime::Runtime>,
    cancel: Arc<ryter_core::Cancel>,
}

impl ryter_core::InboundHost for ServeHost {
    fn prompt(&self, text: &str) -> ryter_core::Result<String> {
        self.cancel.reset();
        let mut agent = self
            .agent
            .lock()
            .map_err(|e| Error::Config(e.to_string()))?;
        let rt = self.rt.lock().map_err(|e| Error::Config(e.to_string()))?;
        let r = rt.block_on(agent.turn(text))?;
        Ok(r.text)
    }

    fn status(&self) -> ryter_core::StatusSnapshot {
        let agent = self.agent.lock().expect("serve host");
        ryter_core::StatusSnapshot {
            model: agent.model.clone(),
            connection: agent.connection.clone(),
            session: agent.session.meta.id.to_string(),
            last_error: String::new(),
        }
    }

    fn spend(&self) -> String {
        let agent = self.agent.lock().expect("serve host");
        format_usd(agent.session.meta.spend_usd_total)
    }

    fn cancel(&self) {
        self.cancel.cancel();
    }
}

fn mcp_serve() -> ryter_core::Result<()> {
    let cwd = std::env::current_dir().map_err(|e| Error::Io(e.to_string()))?;
    let _ = ryter_core::ensure_project_memory(&cwd);
    let trusted = config::is_trusted(&cwd);
    let cfg = config::load(Some(&cwd), trusted)?;
    if !cfg.mcp.inbound {
        return Err(Error::Config("[mcp] inbound = false".into()));
    }
    let home = config::home_dir();
    let last = config::load_last_route(&home);
    let (conn_name, model) = config::resolve_route(&cfg, last.as_ref(), None, None);
    let conn = cfg
        .connections
        .get(&conn_name)
        .ok_or_else(|| Error::Config(format!("unknown connection {conn_name}")))?;
    let key = resolve_secret(&cfg, &ConnectionId::new(&conn_name))?;
    let provider = http_provider(conn, key);
    let mut session = Session::create(&home, &cwd, Phase::Build, conn_name.clone(), model.clone())?;
    session.set_auditor(cfg.auditor.enabled)?;
    let notes = session.notes_dir();
    let queue = Arc::new(std::sync::Mutex::new(ryter_core::queue::TaskQueue::open(
        session.dir.join("tasks.json"),
    )));
    let hub = ryter_core::McpHub::connect(&cfg.mcp_servers)
        .ok()
        .map(|h| Arc::new(std::sync::Mutex::new(h)));
    let profile = cfg.sandbox.profile()?;
    sandbox::apply(profile, &cwd, &home)?;
    let rt = sandbox::runtime(profile).map_err(|e| Error::Io(e.to_string()))?;
    let agent = Agent {
        provider: Arc::new(provider),
        book: PriceBook::from_config(&cfg),
        session,
        ctx: ToolContext {
            workspace: cwd.clone(),
            notes_dir: notes,
            role: Role::Orchestrator,
            always_approve: true,
            queue: queue.clone(),
            mcp: hub,
            hooks: if cfg.hooks.is_empty() {
                None
            } else {
                Some(Arc::new(HookSet::from_config(&cfg.hooks)))
            },
            cancel: ryter_core::Cancel::new(),
            user_io: None,
            sticky_approve: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            web: cfg.features.web,
        },
        connection: conn_name,
        model,
        role: Role::Orchestrator,
        max_turns: 40,
        budget_usd: cfg.spend.session_budget_usd,
        sink: None,
        home,
        project_root: Some(cwd),
        trusted,
        queue,
        max_crew: cfg.subagents.max,
        max_retries: cfg.auditor.max_retries,
        checks: cfg.auditor.checks.clone(),
        check_timeout_secs: cfg.auditor.check_timeout_secs,
        context_window: 0,
        cfg: Some(cfg.clone()),
        running: Arc::new(std::sync::Mutex::new(Vec::new())),
    };
    if let Err(e) = agent.fire_session_start() {
        eprintln!("{e}");
        return Err(e);
    }
    let cancel = agent.ctx.cancel.clone();
    let host = ServeHost {
        agent: std::sync::Mutex::new(agent),
        rt: std::sync::Mutex::new(rt),
        cancel,
    };
    eprintln!("ryter mcp serve on stdio");
    ryter_core::serve_inbound(&host)
}

fn serve_cmd(
    socket: Option<std::path::PathBuf>,
    bind: Option<String>,
    token: Option<String>,
    i_mean_it: bool,
) -> ryter_core::Result<()> {
    let cwd = std::env::current_dir().map_err(|e| Error::Io(e.to_string()))?;
    let _ = ryter_core::ensure_project_memory(&cwd);
    let trusted = config::is_trusted(&cwd);
    let cfg = config::load(Some(&cwd), trusted)?;
    if !cfg.mcp.inbound {
        return Err(Error::Config("[mcp] inbound = false".into()));
    }
    if socket.is_some() && bind.is_some() {
        return Err(Error::Config("use --socket or --bind, not both".into()));
    }
    let host = serve_host_from_config(&cfg, &cwd)?;
    let host: Arc<dyn ryter_core::InboundHost> = Arc::new(host);
    if let Some(bind) = bind {
        let addr: std::net::SocketAddr = bind
            .parse()
            .map_err(|e| Error::Config(format!("bad --bind {bind:?}: {e}")))?;
        let token = token
            .or_else(|| std::env::var("RYTER_MCP_TOKEN").ok())
            .unwrap_or_default();
        ryter_core::check_tcp(addr, &token, i_mean_it)?;
        ryter_core::serve_tcp(addr, host, vec![token])
    } else {
        #[cfg(unix)]
        {
            let home = config::home_dir();
            let path = socket
                .or_else(|| cfg.mcp.socket.as_ref().map(std::path::PathBuf::from))
                .unwrap_or_else(|| ryter_core::default_socket_path(&home));
            ryter_core::serve_unix(&path, host)
        }
        #[cfg(not(unix))]
        {
            let _ = socket;
            Err(Error::Config(
                "unix sockets are Linux-first; use --bind on this OS".into(),
            ))
        }
    }
}

fn serve_host_from_config(
    cfg: &ryter_core::Config,
    cwd: &std::path::Path,
) -> ryter_core::Result<ServeHost> {
    let home = config::home_dir();
    let last = config::load_last_route(&home);
    let (conn_name, model) = config::resolve_route(cfg, last.as_ref(), None, None);
    let conn = cfg
        .connections
        .get(&conn_name)
        .ok_or_else(|| Error::Config(format!("unknown connection {conn_name}")))?;
    let key = resolve_secret(cfg, &ConnectionId::new(&conn_name))?;
    let provider = http_provider(conn, key);
    let mut session = Session::create(&home, cwd, Phase::Build, conn_name.clone(), model.clone())?;
    session.set_auditor(cfg.auditor.enabled)?;
    let notes = session.notes_dir();
    let queue = Arc::new(std::sync::Mutex::new(ryter_core::queue::TaskQueue::open(
        session.dir.join("tasks.json"),
    )));
    let hub = ryter_core::McpHub::connect(&cfg.mcp_servers)
        .ok()
        .map(|h| Arc::new(std::sync::Mutex::new(h)));
    let profile = cfg.sandbox.profile()?;
    sandbox::apply(profile, cwd, &home)?;
    let rt = sandbox::runtime(profile).map_err(|e| Error::Io(e.to_string()))?;
    let cancel = ryter_core::Cancel::new();
    let agent = Agent {
        provider: Arc::new(provider),
        book: PriceBook::from_config(cfg),
        session,
        ctx: ToolContext {
            workspace: cwd.to_path_buf(),
            notes_dir: notes,
            role: Role::Orchestrator,
            always_approve: true,
            queue: queue.clone(),
            mcp: hub,
            hooks: if cfg.hooks.is_empty() {
                None
            } else {
                Some(Arc::new(HookSet::from_config(&cfg.hooks)))
            },
            cancel: cancel.clone(),
            user_io: None,
            sticky_approve: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            web: cfg.features.web,
        },
        connection: conn_name,
        model,
        role: Role::Orchestrator,
        max_turns: 40,
        budget_usd: cfg.spend.session_budget_usd,
        sink: None,
        home,
        project_root: Some(cwd.to_path_buf()),
        trusted: config::is_trusted(cwd),
        queue,
        max_crew: cfg.subagents.max,
        max_retries: cfg.auditor.max_retries,
        checks: cfg.auditor.checks.clone(),
        check_timeout_secs: cfg.auditor.check_timeout_secs,
        context_window: 0,
        cfg: Some(cfg.clone()),
        running: Arc::new(std::sync::Mutex::new(Vec::new())),
    };
    agent.fire_session_start()?;
    Ok(ServeHost {
        agent: std::sync::Mutex::new(agent),
        rt: std::sync::Mutex::new(rt),
        cancel,
    })
}

fn project_spend_cmd() -> ryter_core::Result<()> {
    let cwd = std::env::current_dir().map_err(|e| Error::Io(e.to_string()))?;
    let p = ryter_core::project::project_spend(&config::home_dir(), &cwd)?;
    let unpriced = if p.unpriced_calls > 0 {
        format!(
            "  ({} of {} calls unpriced, not included)",
            p.unpriced_calls, p.calls
        )
    } else {
        String::new()
    };
    println!(
        "project {}  total {}  sessions {}{unpriced}",
        p.root.display(),
        format_usd(Some(p.total_usd)),
        p.sessions
    );
    println!(
        "this month {}  solo {}  crew {}",
        format_usd(Some(p.this_month())),
        format_usd(Some(p.solo_usd())),
        format_usd(Some(p.crew_usd()))
    );
    for (title, map) in [
        ("role", &p.by_role),
        ("model", &p.by_model),
        ("month", &p.by_month),
    ] {
        let mut rows: Vec<_> = map.iter().collect();
        if title == "month" {
            rows.sort_by(|a, b| b.0.cmp(a.0));
        } else {
            rows.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));
        }
        for (k, v) in rows {
            let k = if k == "orchestrator" {
                "lead"
            } else {
                k.as_str()
            };
            println!("  {title:<6} {k:<40} {}", format_usd(Some(*v)));
        }
    }
    Ok(())
}

fn spend_cmd(id: Option<&str>) -> ryter_core::Result<()> {
    let cwd = std::env::current_dir().map_err(|e| Error::Io(e.to_string()))?;
    let home = config::home_dir();
    let session = if let Some(id) = id {
        open_by_id(&home, id)?
    } else {
        Session::latest(&home, &cwd)?
            .ok_or_else(|| Error::Config("no session in this directory".into()))?
    };
    println!(
        "session {}  model {}  total {}",
        session.meta.id,
        session.meta.model,
        format_usd(session.meta.spend_usd_total)
    );
    if session.meta.spend_unknown {
        println!("(some turns unpriced — shown as {})", format_usd(None));
    }
    for rec in session.spend_log()? {
        println!(
            "  {}  {:<14}  in={} out={}  {}",
            rec.role,
            rec.model,
            rec.input_tokens,
            rec.output_tokens,
            format_usd(rec.total_usd)
        );
    }
    Ok(())
}

fn sessions_cmd() -> ryter_core::Result<()> {
    let cwd = std::env::current_dir().map_err(|e| Error::Io(e.to_string()))?;
    let home = config::home_dir();
    let list = Session::list(&home, &cwd)?;
    if list.is_empty() {
        println!("no sessions in this directory");
        return Ok(());
    }
    for s in list {
        println!(
            "{}  {:<8}  {:<12}  {}  {}",
            s.meta.id,
            s.meta.phase,
            s.meta.model,
            format_usd(s.meta.spend_usd_total),
            s.preview
        );
    }
    Ok(())
}

fn open_by_id(home: &std::path::Path, id: &str) -> ryter_core::Result<Session> {
    Session::find(home, None, id)
}
