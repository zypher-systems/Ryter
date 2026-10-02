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
use ryter_core::{Agent, AgentEvent, Error, HookSet, Provider, Role, VERSION};

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

    /// Continue the latest session in this directory (with `-p`).
    #[arg(short = 'c', long = "continue")]
    resume: bool,

    /// build | plan | review | test. Default: build, or the hat a
    /// continued session was left in.
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
    /// Install the latest release, if it is newer, after checking its
    /// signature and checksum. Restart Ryter to use it.
    Update {
        /// Only say whether a newer release is out.
        #[arg(long)]
        check: bool,
    },
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
        Some(Command::Update { check }) => match update_cmd(check) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("ryter update: {e}");
                ExitCode::from(1)
            }
        },
        None if cli.prompt.is_none() => {
            if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
                eprintln!("ryter: not a tty (use -p for headless, --version for version)");
                return ExitCode::from(1);
            }
            match ryter_tui::run(ryter_tui::TuiOpts {
                always_approve: cli.always_approve,
                connection: cli.connection,
                model: cli.model,
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
    let key = resolve_secret(&cfg, &ConnectionId::new(&conn_name))?;
    let provider = http_provider(conn, key);
    let _ = config::save_last_route(&home, &config::LastRoute::new(&conn_name, &model));
    let trusted = config::is_trusted(&cwd);
    // The hat asked for, read before a session is made: a hat that can't
    // be worn must not leave an empty session behind.
    let hat = match cli.hat.as_deref() {
        Some("crew" | "lead") => {
            return Err(Error::Config(
                "crew mode was removed: --hat takes build, plan, review, or test".into(),
            ));
        }
        Some(h) => Some(h.parse::<Role>()?),
        None => None,
    };
    // A budget stop tells the user to continue; headless could only start over.
    let mut session = if cli.resume {
        Session::latest(&home, &cwd)?
            .ok_or_else(|| Error::Config("no session in this directory to continue".into()))?
    } else {
        Session::create(&home, &cwd, conn_name.clone(), model.clone())?
    };
    let tool_sandbox = sandbox::Scope::for_profile(profile, &home);
    if let Some(scope) = &tool_sandbox {
        scope.check(&cwd, &session.notes_dir())?;
    }
    let notes = session.notes_dir();
    let (tx, rx) = mpsc::channel();
    let json = cli.json;
    let printer = std::thread::spawn(move || {
        // Whether the reply so far ends mid-line, so a notice starts its own.
        let mut open_line = false;
        while let Ok(ev) = rx.recv() {
            if json {
                if let Ok(line) = serde_json::to_string(&ev) {
                    println!("{line}");
                }
            } else if let AgentEvent::Token { text } = ev {
                let _ = io::stdout().write_all(text.as_bytes());
                let _ = io::stdout().flush();
                if !text.is_empty() {
                    open_line = !text.ends_with('\n');
                }
            } else if let AgentEvent::Notice { message } = ev {
                if std::mem::take(&mut open_line) {
                    println!();
                }
                eprintln!("ryter: {message}");
            }
        }
    });
    // A session left in crew mode, before it was removed, opens in build.
    let role = hat.unwrap_or_else(|| session.meta.mode.map_or(Role::SoloBuild, Role::hat));
    let _ = session.set_mode(role);
    let mut agent = Agent {
        provider: Arc::new(provider),
        book: PriceBook::from_config(&cfg),
        session,
        ctx: ToolContext {
            sandbox: tool_sandbox,
            live: None,
            workspace: cwd.clone(),
            notes_dir: notes,
            role,
            always_approve: cli.always_approve,
            mcp: None,
            hooks: if cfg.hooks.is_empty() {
                None
            } else {
                Some(Arc::new(HookSet::from_config(&cfg.hooks)))
            },
            cancel: ryter_core::Cancel::new(),
            user_io: None,
            allowed: Default::default(),
            web: cfg.features.web,
            cwd: Default::default(),
            vars: Default::default(),
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
        context_window: 0,
        cfg: Some(cfg.clone()),
        machine: ryter_core::prompt::machine_for(profile),
        product: None,
        filed: Default::default(),
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
        // `main` prints the error, and exits 3 for a budget stop.
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

/// `ryter update [--check]`.
fn update_cmd(check_only: bool) -> ryter_core::Result<()> {
    use ryter_core::update::{self, Source, Version};
    let src = Source::new();
    let current = Version::current();
    let Some(avail) = update::check(&src, current)? else {
        println!("ryter {current} is the latest release");
        return Ok(());
    };
    // A cargo build is told how to update; asked to install, that's a failure.
    if !update::is_release_build() {
        let how = update::built_with_cargo(&avail, current);
        if check_only {
            println!("{how}");
            return Ok(());
        }
        return Err(ryter_core::Error::Config(how));
    }
    if check_only {
        println!(
            "ryter {} is out (you have {current}). `ryter update` installs it.\nWhat's new: {}",
            avail.version, avail.notes
        );
        return Ok(());
    }
    let exe = update::installed_binary()?;
    println!(
        "installing ryter {} over {current} at {}",
        avail.version,
        exe.display()
    );
    update::install(&src, &avail, &exe)?;
    println!(
        "installed ryter {}. Restart Ryter to use it.\nWhat's new: {}",
        avail.version, avail.notes
    );
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
    snapshot: std::sync::Mutex<(ryter_core::StatusSnapshot, String)>,
}

impl ServeHost {
    fn new(agent: Agent, rt: tokio::runtime::Runtime) -> Self {
        let snapshot = Self::snapshot(&agent, String::new());
        Self {
            cancel: agent.ctx.cancel.clone(),
            agent: std::sync::Mutex::new(agent),
            rt: std::sync::Mutex::new(rt),
            snapshot: std::sync::Mutex::new(snapshot),
        }
    }

    fn snapshot(agent: &Agent, last_error: String) -> (ryter_core::StatusSnapshot, String) {
        let mut spend = format_usd(agent.session.meta.spend_usd_total);
        if agent.session.meta.spend_unknown {
            spend.push_str(" + unknown");
        }
        if agent.session.meta.spend_incomplete {
            spend.push_str(" (incomplete)");
        }
        (
            ryter_core::StatusSnapshot {
                model: agent.model.clone(),
                connection: agent.connection.clone(),
                session: agent.session.meta.id.to_string(),
                last_error,
            },
            spend,
        )
    }
}

impl ryter_core::InboundHost for ServeHost {
    fn prepare_prompt(&self) {
        self.cancel.reset();
    }

    fn prompt(&self, text: &str) -> ryter_core::Result<String> {
        let mut agent = self
            .agent
            .try_lock()
            .map_err(|_| Error::Config("busy; a prompt is already running".into()))?;
        let rt = self.rt.lock().map_err(|e| Error::Config(e.to_string()))?;
        let result = rt.block_on(agent.turn(text));
        let last_error = result
            .as_ref()
            .err()
            .map(ToString::to_string)
            .unwrap_or_default();
        if let Ok(mut snapshot) = self.snapshot.lock() {
            *snapshot = Self::snapshot(&agent, last_error);
        }
        result.map(|r| r.text)
    }

    fn status(&self) -> ryter_core::StatusSnapshot {
        self.snapshot
            .lock()
            .map(|s| s.0.clone())
            .unwrap_or_default()
    }

    fn spend(&self) -> String {
        self.snapshot
            .lock()
            .map(|s| s.1.clone())
            .unwrap_or_default()
    }

    fn cancel(&self) {
        self.cancel.cancel();
    }
}

fn mcp_serve() -> ryter_core::Result<()> {
    let cwd = std::env::current_dir().map_err(|e| Error::Io(e.to_string()))?;
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
    let session = Session::create(&home, &cwd, conn_name.clone(), model.clone())?;
    let notes = session.notes_dir();
    let hub = ryter_core::McpHub::connect(&cfg.mcp_servers)
        .ok()
        .map(|h| Arc::new(std::sync::Mutex::new(h)));
    let profile = cfg.sandbox.profile()?;
    let tool_sandbox = sandbox::Scope::for_profile(profile, &home);
    if let Some(scope) = &tool_sandbox {
        scope.check(&cwd, &session.notes_dir())?;
    }
    let rt = sandbox::runtime(profile).map_err(|e| Error::Io(e.to_string()))?;
    let agent = Agent {
        provider: Arc::new(provider),
        book: PriceBook::from_config(&cfg),
        session,
        ctx: ToolContext {
            sandbox: tool_sandbox,
            live: None,
            workspace: cwd.clone(),
            notes_dir: notes,
            // Nobody is there to be asked: a message is worked on in the
            // build hat, and what it would ask about inside the project
            // is allowed. Outside the project still needs a person.
            role: Role::SoloBuild,
            always_approve: true,
            mcp: hub,
            hooks: if cfg.hooks.is_empty() {
                None
            } else {
                Some(Arc::new(HookSet::from_config(&cfg.hooks)))
            },
            cancel: ryter_core::Cancel::new(),
            user_io: None,
            allowed: Default::default(),
            web: cfg.features.web,
            cwd: Default::default(),
            vars: Default::default(),
        },
        connection: conn_name,
        model,
        role: Role::SoloBuild,
        max_turns: 40,
        budget_usd: cfg.spend.session_budget_usd,
        sink: None,
        home,
        project_root: Some(cwd),
        trusted,
        context_window: 0,
        cfg: Some(cfg.clone()),
        machine: ryter_core::prompt::machine_for(profile),
        product: None,
        filed: Default::default(),
    };
    if let Err(e) = agent.fire_session_start() {
        eprintln!("{e}");
        return Err(e);
    }
    let host = ServeHost::new(agent, rt);
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
    let session = Session::create(&home, cwd, conn_name.clone(), model.clone())?;
    let notes = session.notes_dir();
    let hub = ryter_core::McpHub::connect(&cfg.mcp_servers)
        .ok()
        .map(|h| Arc::new(std::sync::Mutex::new(h)));
    let profile = cfg.sandbox.profile()?;
    let tool_sandbox = sandbox::Scope::for_profile(profile, &home);
    if let Some(scope) = &tool_sandbox {
        scope.check(cwd, &session.notes_dir())?;
    }
    let rt = sandbox::runtime(profile).map_err(|e| Error::Io(e.to_string()))?;
    let cancel = ryter_core::Cancel::new();
    let agent = Agent {
        provider: Arc::new(provider),
        book: PriceBook::from_config(cfg),
        session,
        ctx: ToolContext {
            sandbox: tool_sandbox,
            live: None,
            workspace: cwd.to_path_buf(),
            notes_dir: notes,
            // Nobody is there to be asked: a message is worked on in the
            // build hat, and what it would ask about inside the project
            // is allowed. Outside the project still needs a person.
            role: Role::SoloBuild,
            always_approve: true,
            mcp: hub,
            hooks: if cfg.hooks.is_empty() {
                None
            } else {
                Some(Arc::new(HookSet::from_config(&cfg.hooks)))
            },
            cancel: cancel.clone(),
            user_io: None,
            allowed: Default::default(),
            web: cfg.features.web,
            cwd: Default::default(),
            vars: Default::default(),
        },
        connection: conn_name,
        model,
        role: Role::SoloBuild,
        max_turns: 40,
        budget_usd: cfg.spend.session_budget_usd,
        sink: None,
        home,
        project_root: Some(cwd.to_path_buf()),
        trusted: config::is_trusted(cwd),
        context_window: 0,
        cfg: Some(cfg.clone()),
        machine: ryter_core::prompt::machine_for(profile),
        product: None,
        filed: Default::default(),
    };
    agent.fire_session_start()?;
    Ok(ServeHost::new(agent, rt))
}

fn project_spend_cmd() -> ryter_core::Result<()> {
    let cwd = std::env::current_dir().map_err(|e| Error::Io(e.to_string()))?;
    let p = ryter_core::project::project_spend(&config::home_dir(), &cwd)?;
    let unpriced = if p.unpriced_calls > 0 {
        format!(
            "  ({} of {} calls unpriced or incomplete; total is a lower bound)",
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
    // Crew mode is gone; what it spent here before is still part of the
    // project's total, and is said when there is any.
    let crew = p.crew_usd();
    if crew > 0.005 {
        println!(
            "this month {}  hats {}  crew mode (removed) {}",
            format_usd(Some(p.this_month())),
            format_usd(Some(p.solo_usd())),
            format_usd(Some(crew))
        );
    } else {
        println!("this month {}", format_usd(Some(p.this_month())));
    }
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
        println!("(some calls are unpriced or incomplete; the total is a lower bound)");
    }
    for rec in session.spend_log()? {
        println!(
            "  {}  {:<14}  in={} out={}  {}{}",
            rec.role,
            rec.model,
            rec.input_tokens,
            rec.output_tokens,
            format_usd(rec.total_usd),
            if rec.incomplete { " (incomplete)" } else { "" }
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
            s.meta.mode.map_or(Role::SoloBuild, Role::hat),
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
