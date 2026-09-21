//! Layered TOML configuration. No network.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::ids::ConnectionId;
use crate::phase::Phase;

const KEYRING_SERVICE: &str = "ryter";

/// Top-level harness config after defaults + files are merged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Connection used when a session does not pick one.
    pub default_connection: String,
    /// Named inference endpoints.
    pub connections: BTreeMap<String, ConnectionConfig>,
    /// User-facing orchestrator model routing.
    pub orchestrator: RoleModel,
    /// Per-specialist model routing.
    pub specialists: BTreeMap<String, RoleModel>,
    /// Parallelism cap for specialists.
    pub subagents: SubagentsConfig,
    /// Automatic merge auditor.
    pub auditor: AuditorConfig,
    /// Spend accounting.
    pub spend: SpendConfig,
    /// Per-model USD rates; always wins over shipped / catalog prices.
    #[serde(default)]
    pub pricing: BTreeMap<String, PriceOverride>,
    /// Inbound MCP server knobs.
    #[serde(default)]
    pub mcp: McpSettings,
    /// Outbound MCP servers.
    #[serde(default)]
    pub mcp_servers: BTreeMap<String, McpServerConfig>,
    /// Lifecycle hooks (command or HTTP).
    #[serde(default)]
    pub hooks: Vec<HookConfig>,
    /// Landlock profile (`off` by default).
    #[serde(default)]
    pub sandbox: SandboxConfig,
    /// Optional tools (web fetch/search).
    #[serde(default)]
    pub features: FeaturesConfig,
    /// TUI presentation knobs (`[ui]`).
    #[serde(default)]
    pub ui: UiConfig,
    /// Non-fatal load warnings (unknown `[ui]` keys). Never serialized.
    #[serde(skip)]
    pub warnings: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            default_connection: "spacexai".into(),
            connections: builtin_connections(),
            orchestrator: RoleModel {
                connection: Some("spacexai".into()),
                model: Some("grok-4.6".into()),
            },
            specialists: BTreeMap::new(),
            subagents: SubagentsConfig::default(),
            auditor: AuditorConfig::default(),
            spend: SpendConfig::default(),
            pricing: BTreeMap::new(),
            mcp: McpSettings::default(),
            mcp_servers: BTreeMap::new(),
            hooks: Vec::new(),
            sandbox: SandboxConfig::default(),
            features: FeaturesConfig::default(),
            ui: UiConfig::default(),
            warnings: Vec::new(),
        }
    }
}

/// `[ui]` table. Every key is optional; see `config.example.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    /// Speaker name for user messages. Empty → `git user.name` → `$USER` → `you`.
    pub username: String,
    /// Theme name (`dark`, `light`, `default-16`, or `~/.ryter/themes/<name>.toml`).
    pub theme: String,
    /// `collapsed` | `expanded` | `off` — startup state of the reasoning strip.
    pub reasoning: String,
    /// Enable mouse capture (wheel scroll, card clicks).
    pub mouse: bool,
    /// Info panel visible at startup.
    pub panel: bool,
    /// `auto` | `truecolor` | `256` | `16`.
    pub colors: String,
    /// Show `HH:MM` on speaker headers.
    pub timestamps: bool,
    /// Line-number gutter in code blocks.
    pub line_numbers: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            username: String::new(),
            theme: "dark".into(),
            reasoning: "collapsed".into(),
            mouse: true,
            panel: true,
            colors: "auto".into(),
            timestamps: true,
            line_numbers: true,
        }
    }
}

/// Known `[ui]` keys, for the unknown-key warning.
pub const UI_KEYS: &[&str] = &[
    "username",
    "theme",
    "reasoning",
    "mouse",
    "panel",
    "colors",
    "timestamps",
    "line_numbers",
];

/// Unknown keys under `[ui]` in a TOML document (empty when none).
pub fn unknown_ui_keys(text: &str) -> Vec<String> {
    let Ok(table) = toml::from_str::<toml::Table>(text) else {
        return Vec::new();
    };
    let Some(ui) = table.get("ui").and_then(|v| v.as_table()) else {
        return Vec::new();
    };
    ui.keys()
        .filter(|k| !UI_KEYS.contains(&k.as_str()))
        .cloned()
        .collect()
}

/// Optional capabilities.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FeaturesConfig {
    /// Offer `web_fetch` / `web_search`.
    pub web: bool,
}

/// One named LLM endpoint.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectionConfig {
    /// Built-in kind: `spacexai`, `openrouter`, `openai_compat`, `anthropic`.
    pub kind: String,
    /// API base URL.
    pub base_url: String,
    /// Wire protocol.
    #[serde(default = "default_backend")]
    pub api_backend: String,
    /// Environment variable holding the key (string or first of an array later).
    #[serde(default)]
    pub env_key: Option<String>,
    /// Raw key. Never logged. File containing this must be mode 0600.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Default model id on this connection.
    #[serde(default)]
    pub default_model: Option<String>,
    /// OpenRouter `HTTP-Referer`.
    #[serde(default)]
    pub http_referer: Option<String>,
    /// OpenRouter `X-Title`.
    #[serde(default)]
    pub x_title: Option<String>,
}

impl std::fmt::Debug for ConnectionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectionConfig")
            .field("kind", &self.kind)
            .field("base_url", &self.base_url)
            .field("api_backend", &self.api_backend)
            .field("env_key", &self.env_key)
            .field("api_key", &self.api_key.as_ref().map(|_| "***"))
            .field("default_model", &self.default_model)
            .field("http_referer", &self.http_referer)
            .field("x_title", &self.x_title)
            .finish()
    }
}

fn default_backend() -> String {
    "chat_completions".into()
}

/// Model routing for orchestrator or a specialist kind.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RoleModel {
    /// Connection name.
    #[serde(default)]
    pub connection: Option<String>,
    /// Model id.
    #[serde(default)]
    pub model: Option<String>,
}

/// How many specialists may run at once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentsConfig {
    /// Parallelism cap. Must be ≥ 1.
    pub max: u32,
}

impl Default for SubagentsConfig {
    fn default() -> Self {
        Self { max: 4 }
    }
}

/// Automatic auditor gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditorConfig {
    /// When true, builders cannot merge / complete without a pass.
    pub enabled: bool,
    /// Builder fix-turns after a failed audit.
    #[serde(default = "default_retries")]
    pub max_retries: u32,
}

fn default_retries() -> u32 {
    2
}

impl Default for AuditorConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_retries: 2,
        }
    }
}

/// Spend tracking knobs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpendConfig {
    /// Soft switch; counters still increment when false.
    pub enabled: bool,
    /// Display currency.
    #[serde(default = "usd")]
    pub currency: String,
    /// Stop the loop at this USD total. `0` means no cap.
    #[serde(default)]
    pub session_budget_usd: f64,
    /// Status-line warning threshold.
    #[serde(default)]
    pub warn_usd: f64,
}

fn usd() -> String {
    "USD".into()
}

/// `[mcp]` table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct McpSettings {
    /// Allow `ryter mcp serve`.
    pub inbound: bool,
    /// If true, inbound `--always-approve` may run dangerous bash.
    pub allow_dangerous: bool,
    /// Optional unix socket path for `ryter serve`.
    pub socket: Option<String>,
    /// Optional TCP bind (`127.0.0.1:8765`) for inbound MCP.
    #[serde(default)]
    pub bind: Option<String>,
}

impl Default for McpSettings {
    fn default() -> Self {
        Self {
            inbound: true,
            allow_dangerous: false,
            socket: None,
            bind: None,
        }
    }
}

/// One outbound MCP server (`[mcp_servers.<name>]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Executable.
    pub command: String,
    /// Args.
    #[serde(default)]
    pub args: Vec<String>,
    /// When false, skip at connect.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Extra env (API keys are not inherited by default).
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

fn default_true() -> bool {
    true
}

/// `[sandbox]` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SandboxConfig {
    /// `off`, `workspace`, or `read-only`.
    pub profile: String,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            profile: "off".into(),
        }
    }
}

impl SandboxConfig {
    /// Parse the configured profile.
    pub fn profile(&self) -> Result<crate::sandbox::SandboxProfile> {
        self.profile.parse()
    }
}

/// One `[[hooks]]` row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookConfig {
    /// `PreToolUse`, `PostToolUse`, `SessionStart`, or `Handoff`.
    pub event: String,
    /// Shell command. Receives JSON on stdin. Exit 2 denies.
    #[serde(default)]
    pub command: Option<String>,
    /// HTTP POST URL. 401/403/409 denies.
    #[serde(default)]
    pub url: Option<String>,
    /// Optional glob on the tool name (Pre/Post only).
    #[serde(default)]
    pub matcher: Option<String>,
}

/// TOML `[pricing."<model>"]` override. Values are USD per million tokens.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PriceOverride {
    /// Input / prompt tokens.
    #[serde(default)]
    pub input_per_million: Option<f64>,
    /// Cached input tokens.
    #[serde(default)]
    pub cached_per_million: Option<f64>,
    /// Output / completion tokens.
    #[serde(default)]
    pub output_per_million: Option<f64>,
}

impl Default for SpendConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            currency: usd(),
            session_budget_usd: 5.0,
            warn_usd: 1.0,
        }
    }
}

fn builtin_connections() -> BTreeMap<String, ConnectionConfig> {
    let mut m = BTreeMap::new();
    m.insert(
        "spacexai".into(),
        ConnectionConfig {
            kind: "spacexai".into(),
            base_url: "https://api.x.ai/v1".into(),
            api_backend: "responses".into(),
            env_key: Some("XAI_API_KEY".into()),
            api_key: None,
            default_model: Some("grok-4.6".into()),
            http_referer: None,
            x_title: None,
        },
    );
    m.insert(
        "openrouter".into(),
        ConnectionConfig {
            kind: "openrouter".into(),
            base_url: "https://openrouter.ai/api/v1".into(),
            api_backend: "chat_completions".into(),
            env_key: Some("OPENROUTER_API_KEY".into()),
            api_key: None,
            default_model: Some("anthropic/claude-sonnet-4.6".into()),
            http_referer: Some("https://github.com/zypher-systems/ryter".into()),
            x_title: Some("Ryter".into()),
        },
    );
    m
}

/// Starter row for `ryter connections add`.
pub fn connection_template(kind: &str) -> Result<ConnectionConfig> {
    match kind {
        "spacexai" => Ok(ConnectionConfig {
            kind: "spacexai".into(),
            base_url: "https://api.x.ai/v1".into(),
            api_backend: "responses".into(),
            env_key: Some("XAI_API_KEY".into()),
            api_key: None,
            default_model: Some("grok-4.6".into()),
            http_referer: None,
            x_title: None,
        }),
        "openrouter" => Ok(ConnectionConfig {
            kind: "openrouter".into(),
            base_url: "https://openrouter.ai/api/v1".into(),
            api_backend: "chat_completions".into(),
            env_key: Some("OPENROUTER_API_KEY".into()),
            api_key: None,
            default_model: Some("anthropic/claude-sonnet-4.6".into()),
            http_referer: Some("https://github.com/zypher-systems/ryter".into()),
            x_title: Some("Ryter".into()),
        }),
        "openai" | "openai_compat" => Ok(ConnectionConfig {
            kind: "openai_compat".into(),
            base_url: "https://api.openai.com/v1".into(),
            api_backend: "chat_completions".into(),
            env_key: Some("OPENAI_API_KEY".into()),
            api_key: None,
            default_model: Some("gpt-4o".into()),
            http_referer: None,
            x_title: None,
        }),
        "anthropic" => Ok(ConnectionConfig {
            kind: "anthropic".into(),
            base_url: "https://api.anthropic.com".into(),
            api_backend: "messages".into(),
            env_key: Some("ANTHROPIC_API_KEY".into()),
            api_key: None,
            default_model: Some("claude-sonnet-4-6".into()),
            http_referer: None,
            x_title: None,
        }),
        other => Err(Error::Config(format!(
            "unknown kind {other:?} (spacexai, openrouter, openai, anthropic)"
        ))),
    }
}

impl RoleModel {
    /// True when this row actually overrides the orchestrator.
    pub fn is_override(&self) -> bool {
        self.connection.is_some() || self.model.is_some()
    }
}

impl Config {
    fn specialist_row(&self, role: crate::role::Role) -> RoleModel {
        let key = match role {
            crate::role::Role::Orchestrator => return self.orchestrator.clone(),
            crate::role::Role::Planner => "planner",
            crate::role::Role::Architect => "architect",
            crate::role::Role::Builder => "builder",
            crate::role::Role::Auditor => "auditor",
        };
        self.specialists.get(key).cloned().unwrap_or_default()
    }

    /// True when this role uses the live orchestrator provider and model.
    pub fn follows_orchestrator(&self, role: crate::role::Role) -> bool {
        role == crate::role::Role::Orchestrator || !self.specialist_row(role).is_override()
    }

    /// Connection + model for a role. Empty specialist rows follow the orchestrator.
    pub fn route_for(&self, role: crate::role::Role) -> (String, String) {
        if self.follows_orchestrator(role) && role != crate::role::Role::Orchestrator {
            return self.route_for(crate::role::Role::Orchestrator);
        }
        let rm = self.specialist_row(role);
        let conn = rm
            .connection
            .filter(|c| self.connections.contains_key(c))
            .or_else(|| self.orchestrator.connection.clone())
            .unwrap_or_else(|| self.default_connection.clone());
        let model = rm
            .model
            .or_else(|| self.orchestrator.model.clone())
            .or_else(|| {
                self.connections
                    .get(&conn)
                    .and_then(|c| c.default_model.clone())
            })
            .unwrap_or_else(|| "grok-4.6".into());
        (conn, model)
    }
}

/// Where Ryter stores user data (`RYTER_HOME` or `~/.ryter`).
pub fn home_dir() -> PathBuf {
    if let Ok(p) = std::env::var("RYTER_HOME") {
        return PathBuf::from(p);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ryter")
}

/// Load defaults, then user file, then optional project file if `trusted`.
pub fn load(project_root: Option<&Path>, trusted: bool) -> Result<Config> {
    let home = home_dir();
    let _ = write_default_config(&home);
    load_at(&home, project_root, trusted)
}

/// Write `config.example.toml` to `home/config.toml` when missing.
pub fn write_default_config(home: &Path) -> Result<()> {
    fs::create_dir_all(home).map_err(|e| Error::Config(e.to_string()))?;
    let path = home.join("config.toml");
    if path.is_file() {
        return Ok(());
    }
    let body = include_str!("../../../config.example.toml");
    fs::write(&path, body).map_err(|e| Error::Config(e.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Like [`load`], but reads user config from `home` (tests, `RYTER_HOME`).
pub fn load_at(home: &Path, project_root: Option<&Path>, trusted: bool) -> Result<Config> {
    let mut cfg = Config::default();
    let user_path = home.join("config.toml");
    if user_path.is_file() {
        merge_file(&mut cfg, &user_path)?;
        check_key_file_mode(&user_path, &cfg)?;
    }
    if trusted {
        if let Some(root) = project_root {
            let project = root.join(".ryter").join("config.toml");
            if project.is_file() {
                merge_file(&mut cfg, &project)?;
                check_key_file_mode(&project, &cfg)?;
            }
        }
    }
    apply_crew_file(&mut cfg, &home.join("crew.toml"));
    apply_mcp_file(&mut cfg, &home.join("mcp.toml"));
    apply_hooks_file(&mut cfg, &home.join("hooks.toml"));
    apply_connections_file(&mut cfg, &home.join("connections.toml"));
    apply_settings_file(&mut cfg, &home.join("settings.toml"));
    validate(&cfg)?;
    Ok(cfg)
}

const CREW_ROLES: &[&str] = &["planner", "architect", "builder", "auditor"];

/// Live crew assignment file (`~/.ryter/crew.toml`).
pub fn crew_path(home: &Path) -> PathBuf {
    home.join("crew.toml")
}

/// Preset directory (`~/.ryter/crews/`).
pub fn crews_dir(home: &Path) -> PathBuf {
    home.join("crews")
}

fn apply_crew_file(cfg: &mut Config, path: &Path) {
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    let map = match parse_crew_map(&text) {
        Ok(m) => m,
        Err(_) => return,
    };
    apply_crew_map(cfg, map);
}

fn parse_crew_map(text: &str) -> Result<BTreeMap<String, RoleModel>> {
    if text.trim().is_empty() {
        return Ok(BTreeMap::new());
    }
    toml::from_str(text).map_err(|e| Error::Config(e.to_string()))
}

fn apply_crew_map(cfg: &mut Config, map: BTreeMap<String, RoleModel>) {
    for (k, v) in map {
        if CREW_ROLES.contains(&k.as_str()) {
            cfg.specialists.insert(k, v);
        }
    }
}

fn crew_overrides(specialists: &BTreeMap<String, RoleModel>) -> BTreeMap<String, RoleModel> {
    let mut map = BTreeMap::new();
    for role in CREW_ROLES {
        if let Some(rm) = specialists.get(*role) {
            if rm.is_override() {
                map.insert((*role).to_string(), rm.clone());
            }
        }
    }
    map
}

/// Persist the live crew assignment (does not rewrite `config.toml`).
pub fn save_crew(home: &Path, specialists: &BTreeMap<String, RoleModel>) -> Result<()> {
    fs::create_dir_all(home).map_err(|e| Error::Config(e.to_string()))?;
    let map = crew_overrides(specialists);
    let body = toml::to_string(&map).map_err(|e| Error::Config(e.to_string()))?;
    fs::write(crew_path(home), body).map_err(|e| Error::Config(e.to_string()))
}

/// Save a named preset.
pub fn save_crew_preset(
    home: &Path,
    name: &str,
    specialists: &BTreeMap<String, RoleModel>,
) -> Result<()> {
    let name = sanitize_preset_name(name)?;
    let dir = crews_dir(home);
    fs::create_dir_all(&dir).map_err(|e| Error::Config(e.to_string()))?;
    let map = crew_overrides(specialists);
    let body = toml::to_string(&map).map_err(|e| Error::Config(e.to_string()))?;
    fs::write(dir.join(format!("{name}.toml")), body).map_err(|e| Error::Config(e.to_string()))
}

/// Load a named preset into `cfg.specialists` and write `crew.toml`.
pub fn load_crew_preset(home: &Path, cfg: &mut Config, name: &str) -> Result<()> {
    let name = sanitize_preset_name(name)?;
    let path = crews_dir(home).join(format!("{name}.toml"));
    let text =
        fs::read_to_string(&path).map_err(|_| Error::Config(format!("no crew preset {name:?}")))?;
    let map = parse_crew_map(&text)?;
    for role in CREW_ROLES {
        cfg.specialists.remove(*role);
    }
    apply_crew_map(cfg, map);
    save_crew(home, &cfg.specialists)?;
    Ok(())
}

/// Stem names of `~/.ryter/crews/*.toml`.
pub fn list_crew_presets(home: &Path) -> Vec<String> {
    let mut names = Vec::new();
    let Ok(rd) = fs::read_dir(crews_dir(home)) else {
        return names;
    };
    for ent in rd.flatten() {
        let p = ent.path();
        if p.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
            names.push(stem.to_string());
        }
    }
    names.sort();
    names
}

/// Live MCP UI file (`~/.ryter/mcp.toml`). Overlays inbound flags and outbound servers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpFile {
    /// Inbound listen.
    #[serde(default)]
    pub inbound: Option<bool>,
    /// Unix socket path.
    #[serde(default)]
    pub socket: Option<String>,
    /// TCP bind address.
    #[serde(default)]
    pub bind: Option<String>,
    /// Outbound servers. `None` means leave config.toml servers alone.
    #[serde(default)]
    pub servers: Option<BTreeMap<String, McpServerConfig>>,
}

fn apply_mcp_file(cfg: &mut Config, path: &Path) {
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    let Ok(file) = toml::from_str::<McpFile>(&text) else {
        return;
    };
    if let Some(on) = file.inbound {
        cfg.mcp.inbound = on;
    }
    if file.socket.is_some() {
        cfg.mcp.socket = file.socket;
    }
    if file.bind.is_some() {
        cfg.mcp.bind = file.bind;
    }
    if let Some(servers) = file.servers {
        cfg.mcp_servers = servers;
    }
}

/// Persist inbound flags + outbound servers (does not rewrite `config.toml`).
pub fn save_mcp(home: &Path, cfg: &Config) -> Result<()> {
    fs::create_dir_all(home).map_err(|e| Error::Config(e.to_string()))?;
    let file = McpFile {
        inbound: Some(cfg.mcp.inbound),
        socket: cfg.mcp.socket.clone(),
        bind: cfg.mcp.bind.clone(),
        servers: Some(cfg.mcp_servers.clone()),
    };
    let body = toml::to_string(&file).map_err(|e| Error::Config(e.to_string()))?;
    fs::write(home.join("mcp.toml"), body).map_err(|e| Error::Config(e.to_string()))
}

fn mcp_tokens_path(home: &Path) -> PathBuf {
    home.join("keys").join("mcp-inbound.toml")
}

#[derive(Default, Serialize, Deserialize)]
struct McpTokenFile {
    #[serde(default)]
    tokens: BTreeMap<String, String>,
}

/// Named inbound bearer tokens (`~/.ryter/keys/mcp-inbound.toml`, mode 0600).
pub fn list_mcp_tokens(home: &Path) -> BTreeMap<String, String> {
    let Ok(text) = fs::read_to_string(mcp_tokens_path(home)) else {
        return BTreeMap::new();
    };
    toml::from_str::<McpTokenFile>(&text)
        .map(|f| f.tokens)
        .unwrap_or_default()
}

/// Write inbound tokens. File mode 0600.
pub fn save_mcp_tokens(home: &Path, tokens: &BTreeMap<String, String>) -> Result<()> {
    let dir = home.join("keys");
    fs::create_dir_all(&dir).map_err(|e| Error::Config(e.to_string()))?;
    let body = toml::to_string(&McpTokenFile {
        tokens: tokens.clone(),
    })
    .map_err(|e| Error::Config(e.to_string()))?;
    let path = mcp_tokens_path(home);
    fs::write(&path, body).map_err(|e| Error::Config(e.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .map_err(|e| Error::Config(e.to_string()))?;
    }
    Ok(())
}

/// Fresh inbound token (`ryt_` + uuid, no hyphens).
pub fn new_inbound_token() -> String {
    format!("ryt_{}", uuid::Uuid::now_v7().to_string().replace('-', ""))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct HooksFile {
    #[serde(default)]
    hooks: Vec<HookConfig>,
}

fn apply_hooks_file(cfg: &mut Config, path: &Path) {
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    if text.trim().is_empty() {
        cfg.hooks.clear();
        return;
    }
    let Ok(file) = toml::from_str::<HooksFile>(&text) else {
        return;
    };
    cfg.hooks = file.hooks;
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ConnectionsFile {
    #[serde(default)]
    connections: BTreeMap<String, ConnectionConfig>,
}

fn apply_connections_file(cfg: &mut Config, path: &Path) {
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    let Ok(file) = toml::from_str::<ConnectionsFile>(&text) else {
        return;
    };
    for (k, v) in file.connections {
        cfg.connections.insert(k, v);
    }
}

/// Extra connections from the TUI/CLI (`~/.ryter/connections.toml`).
pub fn load_user_connections(home: &Path) -> BTreeMap<String, ConnectionConfig> {
    let Ok(text) = fs::read_to_string(home.join("connections.toml")) else {
        return BTreeMap::new();
    };
    toml::from_str::<ConnectionsFile>(&text)
        .map(|f| f.connections)
        .unwrap_or_default()
}

/// Extra connections from the TUI/CLI (`~/.ryter/connections.toml`).
pub fn save_user_connections(
    home: &Path,
    connections: &BTreeMap<String, ConnectionConfig>,
) -> Result<()> {
    fs::create_dir_all(home).map_err(|e| Error::Config(e.to_string()))?;
    let body = toml::to_string(&ConnectionsFile {
        connections: connections.clone(),
    })
    .map_err(|e| Error::Config(e.to_string()))?;
    fs::write(home.join("connections.toml"), body).map_err(|e| Error::Config(e.to_string()))
}

/// Names stored in `connections.toml` (safe to remove).
pub fn user_connection_names(home: &Path) -> Vec<String> {
    let Ok(text) = fs::read_to_string(home.join("connections.toml")) else {
        return Vec::new();
    };
    toml::from_str::<ConnectionsFile>(&text)
        .map(|f| f.connections.keys().cloned().collect())
        .unwrap_or_default()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SettingsFile {
    session_budget_usd: Option<f64>,
    warn_usd: Option<f64>,
    max: Option<u32>,
    sandbox: Option<String>,
    inbound: Option<bool>,
    web: Option<bool>,
    #[serde(default)]
    auditor: Option<bool>,
    #[serde(default)]
    ui: Option<UiFile>,
}

fn apply_settings_file(cfg: &mut Config, path: &Path) {
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    let Ok(file) = toml::from_str::<SettingsFile>(&text) else {
        return;
    };
    if let Some(v) = file.auditor {
        cfg.auditor.enabled = v;
    }
    if let Some(ui) = file.ui {
        ui.apply(&mut cfg.ui);
    }
    if let Some(v) = file.session_budget_usd {
        cfg.spend.session_budget_usd = v;
    }
    if let Some(v) = file.warn_usd {
        cfg.spend.warn_usd = v;
    }
    if let Some(v) = file.max {
        cfg.subagents.max = v;
    }
    if let Some(v) = file.sandbox {
        cfg.sandbox.profile = v;
    }
    if let Some(v) = file.inbound {
        cfg.mcp.inbound = v;
    }
    if let Some(v) = file.web {
        cfg.features.web = v;
    }
}

/// Persist budget / max / sandbox / inbound / web (does not rewrite `config.toml`).
pub fn save_settings(home: &Path, cfg: &Config) -> Result<()> {
    fs::create_dir_all(home).map_err(|e| Error::Config(e.to_string()))?;
    let file = SettingsFile {
        session_budget_usd: Some(cfg.spend.session_budget_usd),
        warn_usd: Some(cfg.spend.warn_usd),
        max: Some(cfg.subagents.max),
        sandbox: Some(cfg.sandbox.profile.clone()),
        inbound: Some(cfg.mcp.inbound),
        web: Some(cfg.features.web),
        auditor: Some(cfg.auditor.enabled),
        ui: Some(UiFile::from(&cfg.ui)),
    };
    let body = toml::to_string(&file).map_err(|e| Error::Config(e.to_string()))?;
    fs::write(home.join("settings.toml"), body).map_err(|e| Error::Config(e.to_string()))
}

/// Persist `[[hooks]]` (does not rewrite `config.toml`).
pub fn save_hooks(home: &Path, hooks: &[HookConfig]) -> Result<()> {
    fs::create_dir_all(home).map_err(|e| Error::Config(e.to_string()))?;
    let body = toml::to_string(&HooksFile {
        hooks: hooks.to_vec(),
    })
    .map_err(|e| Error::Config(e.to_string()))?;
    fs::write(home.join("hooks.toml"), body).map_err(|e| Error::Config(e.to_string()))
}

fn sanitize_preset_name(name: &str) -> Result<String> {
    let s = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        Err(Error::Config("preset name is empty".into()))
    } else {
        Ok(s)
    }
}

fn merge_file(cfg: &mut Config, path: &Path) -> Result<()> {
    let text = fs::read_to_string(path).map_err(|e| Error::Config(format!("{path:?}: {e}")))?;
    let overlay: ConfigFile =
        toml::from_str(&text).map_err(|e| Error::Config(format!("{path:?}: {e}")))?;
    for k in unknown_ui_keys(&text) {
        cfg.warnings.push(format!(
            "{}: unknown [ui] key `{k}` ignored",
            path.display()
        ));
    }
    overlay.apply(cfg);
    Ok(())
}

/// On-disk shape: all fields optional so files can be sparse.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ConfigFile {
    default_connection: Option<String>,
    connections: BTreeMap<String, ConnectionConfig>,
    orchestrator: Option<RoleModel>,
    specialists: BTreeMap<String, RoleModel>,
    subagents: Option<SubagentsConfig>,
    auditor: Option<AuditorConfig>,
    spend: Option<SpendConfig>,
    pricing: BTreeMap<String, PriceOverride>,
    mcp: Option<McpSettings>,
    mcp_servers: BTreeMap<String, McpServerConfig>,
    hooks: Vec<HookConfig>,
    sandbox: Option<SandboxConfig>,
    features: Option<FeaturesConfig>,
    ui: Option<UiFile>,
}

/// Sparse `[ui]` overlay: only keys present in the file win.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct UiFile {
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mouse: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    panel: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    colors: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timestamps: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    line_numbers: Option<bool>,
}

impl From<&UiConfig> for UiFile {
    fn from(ui: &UiConfig) -> Self {
        Self {
            username: Some(ui.username.clone()),
            theme: Some(ui.theme.clone()),
            reasoning: Some(ui.reasoning.clone()),
            mouse: Some(ui.mouse),
            panel: Some(ui.panel),
            colors: Some(ui.colors.clone()),
            timestamps: Some(ui.timestamps),
            line_numbers: Some(ui.line_numbers),
        }
    }
}

impl UiFile {
    fn apply(self, ui: &mut UiConfig) {
        if let Some(v) = self.username {
            ui.username = v;
        }
        if let Some(v) = self.theme {
            ui.theme = v;
        }
        if let Some(v) = self.reasoning {
            ui.reasoning = v;
        }
        if let Some(v) = self.mouse {
            ui.mouse = v;
        }
        if let Some(v) = self.panel {
            ui.panel = v;
        }
        if let Some(v) = self.colors {
            ui.colors = v;
        }
        if let Some(v) = self.timestamps {
            ui.timestamps = v;
        }
        if let Some(v) = self.line_numbers {
            ui.line_numbers = v;
        }
    }
}

impl ConfigFile {
    fn apply(self, cfg: &mut Config) {
        if let Some(d) = self.default_connection {
            cfg.default_connection = d;
        }
        for (k, v) in self.connections {
            cfg.connections.insert(k, v);
        }
        if let Some(o) = self.orchestrator {
            cfg.orchestrator = o;
        }
        for (k, v) in self.specialists {
            cfg.specialists.insert(k, v);
        }
        if let Some(s) = self.subagents {
            cfg.subagents = s;
        }
        if let Some(a) = self.auditor {
            cfg.auditor = a;
        }
        if let Some(s) = self.spend {
            cfg.spend = s;
        }
        for (k, v) in self.pricing {
            cfg.pricing.insert(k, v);
        }
        if let Some(m) = self.mcp {
            cfg.mcp = m;
        }
        for (k, v) in self.mcp_servers {
            cfg.mcp_servers.insert(k, v);
        }
        cfg.hooks.extend(self.hooks);
        if let Some(s) = self.sandbox {
            cfg.sandbox = s;
        }
        if let Some(f) = self.features {
            cfg.features = f;
        }
        if let Some(u) = self.ui {
            u.apply(&mut cfg.ui);
        }
    }
}

fn validate(cfg: &Config) -> Result<()> {
    if cfg.subagents.max < 1 {
        return Err(Error::Config(
            "[subagents] max must be >= 1 (the orchestrator cannot write source)".into(),
        ));
    }
    if !cfg.connections.contains_key(&cfg.default_connection) {
        return Err(Error::Config(format!(
            "default_connection {:?} is not a known connection",
            cfg.default_connection
        )));
    }
    Ok(())
}

fn check_key_file_mode(path: &Path, cfg: &Config) -> Result<()> {
    let has_key = cfg.connections.values().any(|c| c.api_key.is_some());
    if !has_key {
        return Ok(());
    }
    check_mode_0600(path)
}

#[cfg(unix)]
fn check_mode_0600(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = fs::metadata(path)
        .map_err(|e| Error::Config(format!("{path:?}: {e}")))?
        .permissions()
        .mode()
        & 0o777;
    if mode & 0o077 != 0 {
        return Err(Error::Config(format!(
            "{path:?} contains api_key and is mode {mode:o}; chmod 600 the file or move the key to env/keyring"
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_mode_0600(_path: &Path) -> Result<()> {
    Ok(())
}

/// Resolve a secret for `connection` without logging it.
pub fn resolve_secret(cfg: &Config, connection: &ConnectionId) -> Result<String> {
    resolve_secret_with(cfg, connection, |k| std::env::var(k).ok())
}

/// Resolve a secret, using `env` instead of the process environment (tests).
pub fn resolve_secret_with(
    cfg: &Config,
    connection: &ConnectionId,
    env: impl Fn(&str) -> Option<String>,
) -> Result<String> {
    let name = connection.as_str();
    let Some(conn) = cfg.connections.get(name) else {
        return Err(Error::Config(format!("unknown connection {name:?}")));
    };
    if let Some(k) = conn.api_key.as_deref().map(str::trim) {
        if !k.is_empty() {
            return Ok(k.to_string());
        }
    }
    if let Some(var) = conn.env_key.as_deref() {
        if let Some(v) = env(var) {
            let t = v.trim();
            if !t.is_empty() {
                return Ok(t.to_string());
            }
        }
    }
    if let Some(k) = keyring_get(name) {
        return Ok(k);
    }
    if let Some(k) = file_key(name) {
        return Ok(k);
    }
    let fallback = match conn.kind.as_str() {
        "spacexai" => Some("XAI_API_KEY"),
        "openrouter" => Some("OPENROUTER_API_KEY"),
        _ => None,
    };
    if let Some(var) = fallback {
        if let Some(v) = env(var) {
            let t = v.trim();
            if !t.is_empty() {
                return Ok(t.to_string());
            }
        }
        return Err(Error::Config(format!(
            "no key for connection {name:?}; export {var}=... or run `ryter connections set-key {name}`"
        )));
    }
    Err(Error::Config(format!(
        "no key for connection {name:?}; set env_key or api_key, or `ryter connections set-key {name}`"
    )))
}

fn keyring_get(connection: &str) -> Option<String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, &format!("connection:{connection}")).ok()?;
    entry.get_password().ok().filter(|s| !s.is_empty())
}

/// Store a key in the OS keyring.
pub fn keyring_set(connection: &str, secret: &str) -> Result<()> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, &format!("connection:{connection}"))
        .map_err(|e| Error::Config(e.to_string()))?;
    entry
        .set_password(secret)
        .map_err(|e| Error::Config(e.to_string()))
}

/// Whether a connection has a resolvable key (never returns the secret).
pub fn has_secret(cfg: &Config, connection: &str) -> bool {
    resolve_secret(cfg, &ConnectionId::new(connection)).is_ok()
}

/// Persist a secret: keyring if it works, else `home/keys/<name>` mode 0600.
pub fn store_secret_at(home: &Path, connection: &str, secret: &str) -> Result<()> {
    let secret = secret.trim();
    if secret.is_empty() {
        return Err(Error::Config("empty API key".into()));
    }
    let _ = keyring_set(connection, secret);
    let dir = home.join("keys");
    fs::create_dir_all(&dir).map_err(|e| Error::Config(e.to_string()))?;
    let path = dir.join(connection);
    fs::write(&path, secret).map_err(|e| Error::Config(e.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .map_err(|e| Error::Config(e.to_string()))?;
    }
    Ok(())
}

/// Last provider + model the user picked (TUI or CLI).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LastRoute {
    /// Connection name.
    pub connection: String,
    /// Model id.
    pub model: String,
    /// Last known context window (tokens).
    #[serde(default)]
    pub context_length: Option<u64>,
    /// Last known USD / million input.
    #[serde(default)]
    pub input_per_million: Option<f64>,
    /// Last known USD / million output.
    #[serde(default)]
    pub output_per_million: Option<f64>,
}

impl LastRoute {
    /// Connection + model only.
    pub fn new(connection: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            connection: connection.into(),
            model: model.into(),
            context_length: None,
            input_per_million: None,
            output_per_million: None,
        }
    }
}

/// `~/.ryter/last.toml`.
pub fn last_route_path(home: &Path) -> PathBuf {
    home.join("last.toml")
}

/// Load the last used provider/model, if any.
pub fn load_last_route(home: &Path) -> Option<LastRoute> {
    let text = fs::read_to_string(last_route_path(home)).ok()?;
    toml::from_str(&text)
        .ok()
        .filter(|r: &LastRoute| !r.connection.trim().is_empty() && !r.model.trim().is_empty())
}

/// Remember the last used provider and model. Also updates `default_connection`.
pub fn save_last_route(home: &Path, route: &LastRoute) -> Result<()> {
    fs::create_dir_all(home).map_err(|e| Error::Config(e.to_string()))?;
    let mut stored = route.clone();
    if let Some(prev) = load_last_route(home) {
        if prev.connection == stored.connection && prev.model == stored.model {
            if stored.context_length.is_none() {
                stored.context_length = prev.context_length;
            }
            if stored.input_per_million.is_none() {
                stored.input_per_million = prev.input_per_million;
            }
            if stored.output_per_million.is_none() {
                stored.output_per_million = prev.output_per_million;
            }
        }
    }
    let body = toml::to_string(&stored).map_err(|e| Error::Config(e.to_string()))?;
    fs::write(last_route_path(home), body).map_err(|e| Error::Config(e.to_string()))?;
    write_default_connection(home, &stored.connection)?;
    Ok(())
}

/// CLI flags beat last-used, which beats config defaults.
pub fn resolve_route(
    cfg: &Config,
    last: Option<&LastRoute>,
    cli_connection: Option<&str>,
    cli_model: Option<&str>,
) -> (String, String) {
    let conn = cli_connection
        .map(str::to_string)
        .or_else(|| {
            last.and_then(|l| {
                cfg.connections
                    .contains_key(&l.connection)
                    .then(|| l.connection.clone())
            })
        })
        .unwrap_or_else(|| cfg.default_connection.clone());
    let conn_cfg = cfg.connections.get(&conn);
    let model = cli_model
        .map(str::to_string)
        .or_else(|| {
            last.and_then(|l| {
                (l.connection == conn && !l.model.trim().is_empty()).then(|| l.model.clone())
            })
        })
        .or_else(|| cfg.orchestrator.model.clone())
        .or_else(|| conn_cfg.and_then(|c| c.default_model.clone()))
        .unwrap_or_else(|| "grok-4.6".into());
    (conn, model)
}

/// Persist `default_connection` in `home/config.toml`.
pub fn write_default_connection(home: &Path, name: &str) -> Result<()> {
    fs::create_dir_all(home).map_err(|e| Error::Config(e.to_string()))?;
    let path = home.join("config.toml");
    let existing = fs::read_to_string(&path).unwrap_or_default();
    let line = format!("default_connection = \"{name}\"\n");
    let mut out = String::new();
    let mut replaced = false;
    for l in existing.lines() {
        if l.trim_start().starts_with("default_connection") {
            if !replaced {
                out.push_str(&line);
                replaced = true;
            }
        } else {
            out.push_str(l);
            out.push('\n');
        }
    }
    if !replaced {
        out = format!("{line}{out}");
    }
    fs::write(&path, out).map_err(|e| Error::Config(e.to_string()))?;
    Ok(())
}

fn file_key(connection: &str) -> Option<String> {
    let p = home_dir().join("keys").join(connection);
    fs::read_to_string(p)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Trusted-project list lives under the home dir.
#[derive(Debug, Default, Serialize, Deserialize)]
struct TrustStore {
    paths: Vec<String>,
}

fn trust_path() -> PathBuf {
    home_dir().join("trusted.json")
}

/// Whether `root` is a trusted project folder.
pub fn is_trusted(root: &Path) -> bool {
    let Ok(canon) = fs::canonicalize(root) else {
        return false;
    };
    let Ok(text) = fs::read_to_string(trust_path()) else {
        return false;
    };
    let Ok(store) = serde_json::from_str::<TrustStore>(&text) else {
        return false;
    };
    store.paths.iter().any(|p| Path::new(p) == canon)
}

/// Mark `root` trusted.
pub fn trust(root: &Path) -> Result<()> {
    let canon = fs::canonicalize(root).map_err(|e| Error::Config(e.to_string()))?;
    let mut store = fs::read_to_string(trust_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(TrustStore { paths: Vec::new() });
    let s = canon.to_string_lossy().into_owned();
    if !store.paths.iter().any(|p| p == &s) {
        store.paths.push(s);
    }
    let dir = home_dir();
    fs::create_dir_all(&dir).map_err(|e| Error::Config(e.to_string()))?;
    let tmp = dir.join("trusted.json.tmp");
    let mut f = fs::File::create(&tmp).map_err(|e| Error::Config(e.to_string()))?;
    let body = serde_json::to_vec_pretty(&store).map_err(|e| Error::Config(e.to_string()))?;
    f.write_all(&body)
        .map_err(|e| Error::Config(e.to_string()))?;
    f.flush().map_err(|e| Error::Config(e.to_string()))?;
    fs::rename(tmp, trust_path()).map_err(|e| Error::Config(e.to_string()))?;
    Ok(())
}

/// Default phase for a new session.
pub fn default_phase() -> Phase {
    Phase::Build
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn ui_section_is_optional_and_sparse() {
        let home = TempDir::new().unwrap();
        fs::write(
            home.path().join("config.toml"),
            "[ui]\nusername = \"Dusty\"\nreasoning = \"expanded\"\nbogus = 1\n",
        )
        .unwrap();
        let cfg = load_at(home.path(), None, false).unwrap();
        assert_eq!(cfg.ui.username, "Dusty");
        assert_eq!(cfg.ui.reasoning, "expanded");
        assert_eq!(cfg.ui.theme, "dark", "untouched keys keep defaults");
        assert!(cfg.ui.mouse);
        assert_eq!(cfg.warnings.len(), 1, "{:?}", cfg.warnings);
        assert!(cfg.warnings[0].contains("bogus"));
        // settings.toml persists the theme without clobbering config.toml keys.
        let mut saved = cfg.clone();
        saved.ui.theme = "light".into();
        save_settings(home.path(), &saved).unwrap();
        let again = load_at(home.path(), None, false).unwrap();
        assert_eq!(again.ui.theme, "light");
        assert_eq!(again.ui.username, "Dusty");
    }

    #[test]
    fn unknown_ui_keys_lists_only_strangers() {
        assert!(unknown_ui_keys("[ui]\ntheme = \"dark\"\n").is_empty());
        assert_eq!(unknown_ui_keys("[ui]\nfoo = 1\n"), vec!["foo".to_string()]);
        assert!(unknown_ui_keys("not toml [[").is_empty());
    }

    #[test]
    fn defaults_include_spacexai_and_openrouter() {
        let cfg = Config::default();
        assert!(cfg.connections.contains_key("spacexai"));
        assert!(cfg.connections.contains_key("openrouter"));
        assert_eq!(cfg.connections["spacexai"].api_backend, "responses");
        assert_eq!(cfg.subagents.max, 4);
        assert!(cfg.auditor.enabled);
        assert!(
            cfg.specialists.is_empty(),
            "crew must not ship a factory provider split"
        );
    }

    #[test]
    fn max_zero_is_rejected() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("config.toml"), "[subagents]\nmax = 0\n").unwrap();
        let err = load_at(dir.path(), None, false).unwrap_err();
        assert!(err.to_string().contains("max must be >= 1"));
    }

    #[test]
    fn crew_preset_round_trip() {
        let dir = TempDir::new().unwrap();
        let mut specs = BTreeMap::new();
        specs.insert(
            "builder".into(),
            RoleModel {
                connection: Some("openrouter".into()),
                model: Some("anthropic/claude-sonnet-4.6".into()),
            },
        );
        save_crew_preset(dir.path(), "web apps", &specs).unwrap();
        assert!(list_crew_presets(dir.path()).contains(&"web-apps".to_string()));
        let mut cfg = Config::default();
        load_crew_preset(dir.path(), &mut cfg, "web-apps").unwrap();
        assert_eq!(
            cfg.specialists.get("builder").and_then(|r| r.model.clone()),
            Some("anthropic/claude-sonnet-4.6".into())
        );
        let live = load_at(dir.path(), None, false).unwrap();
        assert_eq!(
            live.specialists
                .get("builder")
                .and_then(|r| r.model.clone()),
            Some("anthropic/claude-sonnet-4.6".into())
        );
    }

    #[test]
    fn route_for_empty_specialists_follows_orchestrator() {
        let cfg = Config::default();
        let orch = cfg.route_for(crate::role::Role::Orchestrator);
        assert_eq!(orch, ("spacexai".into(), "grok-4.6".into()));
        assert_eq!(cfg.route_for(crate::role::Role::Planner), orch);
        assert_eq!(cfg.route_for(crate::role::Role::Architect), orch);
        assert_eq!(cfg.route_for(crate::role::Role::Builder), orch);
        assert_eq!(cfg.route_for(crate::role::Role::Auditor), orch);
        let mut cfg = cfg;
        cfg.specialists.insert(
            "architect".into(),
            RoleModel {
                connection: Some("openrouter".into()),
                model: Some("anthropic/claude-sonnet-4.6".into()),
            },
        );
        let (conn, model) = cfg.route_for(crate::role::Role::Architect);
        assert_eq!(conn, "openrouter");
        assert_eq!(model, "anthropic/claude-sonnet-4.6");
        assert_eq!(cfg.route_for(crate::role::Role::Planner), orch);
        assert!(cfg.follows_orchestrator(crate::role::Role::Planner));
        assert!(!cfg.follows_orchestrator(crate::role::Role::Architect));
    }

    #[test]
    fn save_crew_omits_empty_rows() {
        let dir = TempDir::new().unwrap();
        let mut specs = BTreeMap::new();
        specs.insert("planner".into(), RoleModel::default());
        specs.insert(
            "builder".into(),
            RoleModel {
                connection: Some("openrouter".into()),
                model: Some("anthropic/claude-sonnet-4.6".into()),
            },
        );
        save_crew(dir.path(), &specs).unwrap();
        let live = load_at(dir.path(), None, false).unwrap();
        assert!(!live.specialists.contains_key("planner"));
        assert_eq!(
            live.specialists
                .get("builder")
                .and_then(|r| r.model.clone()),
            Some("anthropic/claude-sonnet-4.6".into())
        );
        assert!(live.follows_orchestrator(crate::role::Role::Planner));
        assert!(!live.follows_orchestrator(crate::role::Role::Builder));
    }

    #[test]
    fn write_default_config_is_once() {
        let dir = TempDir::new().unwrap();
        write_default_config(dir.path()).unwrap();
        let path = dir.path().join("config.toml");
        let body = fs::read_to_string(&path).unwrap();
        assert!(body.contains("spacexai"));
        fs::write(&path, "sentinel = true\n").unwrap();
        write_default_config(dir.path()).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "sentinel = true\n");
    }

    #[test]
    fn settings_and_user_connections_round_trip() {
        let dir = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.features.web = true;
        cfg.spend.session_budget_usd = 9.0;
        cfg.subagents.max = 2;
        save_settings(dir.path(), &cfg).unwrap();
        let extra = connection_template("openai").unwrap();
        let mut map = BTreeMap::new();
        map.insert("local".into(), extra);
        save_user_connections(dir.path(), &map).unwrap();
        let live = load_at(dir.path(), None, false).unwrap();
        assert!(live.features.web);
        assert!((live.spend.session_budget_usd - 9.0).abs() < f64::EPSILON);
        assert_eq!(live.subagents.max, 2);
        assert_eq!(live.connections["local"].kind, "openai_compat");
        assert!(user_connection_names(dir.path()).contains(&"local".to_string()));
    }

    #[test]
    fn mcp_file_overlays_servers_and_tokens_are_0600() {
        let dir = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.mcp.inbound = true;
        cfg.mcp.bind = Some("127.0.0.1:8765".into());
        cfg.mcp_servers.insert(
            "github".into(),
            McpServerConfig {
                command: "npx".into(),
                args: vec!["-y".into(), "@modelcontextprotocol/server-github".into()],
                enabled: true,
                env: BTreeMap::new(),
            },
        );
        save_mcp(dir.path(), &cfg).unwrap();
        let live = load_at(dir.path(), None, false).unwrap();
        assert_eq!(live.mcp.bind.as_deref(), Some("127.0.0.1:8765"));
        assert_eq!(live.mcp_servers["github"].command, "npx");
        let mut tokens = BTreeMap::new();
        tokens.insert("cursor".into(), new_inbound_token());
        save_mcp_tokens(dir.path(), &tokens).unwrap();
        let loaded = list_mcp_tokens(dir.path());
        assert!(loaded.get("cursor").unwrap().starts_with("ryt_"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.path().join("keys/mcp-inbound.toml"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn hooks_file_replaces_config_hooks() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("config.toml"),
            "[[hooks]]\nevent = \"Handoff\"\ncommand = \"/bin/false\"\n",
        )
        .unwrap();
        let from_cfg = load_at(dir.path(), None, false).unwrap();
        assert_eq!(from_cfg.hooks.len(), 1);
        save_hooks(
            dir.path(),
            &[HookConfig {
                event: "PreToolUse".into(),
                command: Some("/bin/true".into()),
                url: None,
                matcher: Some("bash".into()),
            }],
        )
        .unwrap();
        let live = load_at(dir.path(), None, false).unwrap();
        assert_eq!(live.hooks.len(), 1);
        assert_eq!(live.hooks[0].event, "PreToolUse");
        assert_eq!(live.hooks[0].matcher.as_deref(), Some("bash"));
    }

    #[test]
    fn last_route_survives_builtin_orchestrator_default() {
        let dir = TempDir::new().unwrap();
        save_last_route(
            dir.path(),
            &LastRoute {
                connection: "openrouter".into(),
                model: "anthropic/claude-sonnet-4.6".into(),
                context_length: Some(200_000),
                input_per_million: Some(3.0),
                output_per_million: Some(15.0),
            },
        )
        .unwrap();
        let cfg = load_at(dir.path(), None, false).unwrap();
        // Built-in [orchestrator] is still grok-4.6; last.toml must win.
        assert_eq!(cfg.orchestrator.model.as_deref(), Some("grok-4.6"));
        let last = load_last_route(dir.path()).unwrap();
        let (conn, model) = resolve_route(&cfg, Some(&last), None, None);
        assert_eq!(conn, "openrouter");
        assert_eq!(model, "anthropic/claude-sonnet-4.6");
        assert_eq!(last.context_length, Some(200_000));
        assert_eq!(last.input_per_million, Some(3.0));
        assert_eq!(last.output_per_million, Some(15.0));
        let (conn, model) = resolve_route(&cfg, Some(&last), Some("spacexai"), None);
        assert_eq!(conn, "spacexai");
        assert_ne!(model, "anthropic/claude-sonnet-4.6");
    }

    #[test]
    fn write_default_connection_round_trip() {
        let dir = TempDir::new().unwrap();
        write_default_connection(dir.path(), "openrouter").unwrap();
        let cfg = load_at(dir.path(), None, false).unwrap();
        assert_eq!(cfg.default_connection, "openrouter");
        write_default_connection(dir.path(), "spacexai").unwrap();
        let cfg = load_at(dir.path(), None, false).unwrap();
        assert_eq!(cfg.default_connection, "spacexai");
    }

    #[test]
    fn store_secret_file_fallback() {
        let dir = TempDir::new().unwrap();
        store_secret_at(dir.path(), "ryter-test-conn", "xai-test-secret").unwrap();
        let got = fs::read_to_string(dir.path().join("keys/ryter-test-conn")).unwrap();
        assert_eq!(got.trim(), "xai-test-secret");
    }

    #[test]
    fn overlay_changes_default_connection() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("config.toml"),
            "default_connection = \"openrouter\"\n",
        )
        .unwrap();
        let cfg = load_at(dir.path(), None, false).unwrap();
        assert_eq!(cfg.default_connection, "openrouter");
        assert!(cfg.connections.contains_key("spacexai"));
    }

    #[test]
    fn untrusted_project_file_is_ignored() {
        let home = TempDir::new().unwrap();
        let proj = TempDir::new().unwrap();
        fs::create_dir_all(proj.path().join(".ryter")).unwrap();
        fs::write(
            proj.path().join(".ryter/config.toml"),
            "default_connection = \"openrouter\"\n",
        )
        .unwrap();
        let cfg = load_at(home.path(), Some(proj.path()), false).unwrap();
        assert_eq!(cfg.default_connection, "spacexai");
        let cfg = load_at(home.path(), Some(proj.path()), true).unwrap();
        assert_eq!(cfg.default_connection, "openrouter");
    }

    #[cfg(unix)]
    #[test]
    fn world_readable_api_key_is_rejected() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(
            f,
            "[connections.spacexai]\nkind = \"spacexai\"\nbase_url = \"https://api.x.ai/v1\"\napi_key = \"xai-secret\"\n"
        )
        .unwrap();
        f.flush().unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let err = load_at(dir.path(), None, false).unwrap_err();
        assert!(err.to_string().contains("chmod 600"));
    }

    #[test]
    fn pricing_overlay_from_toml() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("config.toml"),
            "[pricing.\"grok-4.6\"]\ninput_per_million = 9.0\noutput_per_million = 1.0\n",
        )
        .unwrap();
        let cfg = load_at(dir.path(), None, false).unwrap();
        let p = &cfg.pricing["grok-4.6"];
        assert_eq!(p.input_per_million, Some(9.0));
        assert_eq!(p.output_per_million, Some(1.0));
    }

    #[test]
    fn sandbox_overlay() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("config.toml"),
            "[sandbox]\nprofile = \"workspace\"\n",
        )
        .unwrap();
        let cfg = load_at(dir.path(), None, false).unwrap();
        assert_eq!(cfg.sandbox.profile, "workspace");
        assert_eq!(
            cfg.sandbox.profile().unwrap(),
            crate::sandbox::SandboxProfile::Workspace
        );
    }

    #[test]
    fn hooks_overlay_appends() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("config.toml"),
            "[[hooks]]\nevent = \"PreToolUse\"\ncommand = \"true\"\nmatcher = \"bash\"\n",
        )
        .unwrap();
        let cfg = load_at(dir.path(), None, false).unwrap();
        assert_eq!(cfg.hooks.len(), 1);
        assert_eq!(cfg.hooks[0].event, "PreToolUse");
        assert_eq!(cfg.hooks[0].matcher.as_deref(), Some("bash"));
    }

    #[test]
    fn resolve_secret_from_env_key() {
        let cfg = Config::default();
        let got = resolve_secret_with(&cfg, &ConnectionId::new("spacexai"), |k| {
            (k == "XAI_API_KEY").then(|| "from-env".into())
        })
        .unwrap();
        assert_eq!(got, "from-env");
    }
}
