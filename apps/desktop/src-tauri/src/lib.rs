use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tez_auth::providers::{self, ProviderBook, ProviderEntry, ProviderInput};
use tez_core::agent::Agent;
use tez_core::context::{ContextBuilder, ContextInput};
use tez_core::mcp::{self, McpServerConfig};
use tez_core::modes::Mode;
use tez_core::plugins;
use tez_core::skills::{self, SkillRegistry};
use tez_core::subjects::{self, SubjectPack};
use tez_core::tools::balance_equation::BalanceEquation;
use tez_core::tools::calculator::Calculator;
use tez_core::tools::check_grammar::CheckGrammar;
use tez_core::tools::check_step::CheckStep;
use tez_core::tools::convert_units::ConvertUnits;
use tez_core::tools::molar_mass::MolarMass;
use tez_core::tools::physics_formula::PhysicsFormula;
use tez_core::tools::plot_function::PlotFunction;
use tez_core::tools::readability::Readability;
use tez_core::tools::{code_registry, ToolRegistry};
use tez_llm::{LlmProvider, Message, OpenAiCompatible};
use tez_store::{NewMessage, Store, UsageRecord};

const SAMPLE_PLUGIN_JSON: &str =
    include_str!("../../../../examples/plugins/study-toolkit/plugin.json");
const SAMPLE_PLUGIN_SERVER: &str =
    include_str!("../../../../examples/plugins/study-toolkit/server.py");
const SAMPLE_PLUGIN_SKILL: &str =
    include_str!("../../../../examples/plugins/study-toolkit/skills/exam-strategy/SKILL.md");

#[derive(Debug, Deserialize)]
struct SaveProviderRequest {
    id: Option<String>,
    name: String,
    provider: String,
    base_url: Option<String>,
    model: Option<String>,
    api_key: Option<String>,
    #[serde(default)]
    set_active: bool,
}

#[derive(Debug, Deserialize)]
struct SaveMcpRequest {
    name: String,
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: HashMap<String, String>,
    cwd: Option<String>,
    #[serde(default = "default_true")]
    enabled: bool,
}

fn default_true() -> bool {
    true
}

struct ResolvedConnection {
    id: String,
    provider_id: String,
    provider_name: String,
    base_url: String,
    model: String,
    api_key: String,
}

#[derive(Debug, Serialize)]
struct SubjectInfo {
    id: &'static str,
    name: &'static str,
    strategy: &'static str,
    tools: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
struct ConnectionInfo {
    id: String,
    name: String,
    provider: String,
    model: String,
    base_url: String,
}

#[derive(Debug, Serialize)]
struct ProviderInfo {
    id: String,
    name: String,
    provider: String,
    base_url: String,
    model: String,
    has_key: bool,
    active: bool,
}

#[derive(Debug, Serialize)]
struct ExtensionSummary {
    skills: usize,
    mcp_servers: usize,
    plugins: usize,
}

#[derive(Debug, Serialize)]
struct AppStatus {
    connection: Option<ConnectionInfo>,
    providers: Vec<ProviderInfo>,
    subjects: Vec<SubjectInfo>,
    sessions: i64,
    data_dir: String,
    default_workspace: String,
    version: &'static str,
    extensions: ExtensionSummary,
}

#[derive(Debug, Deserialize)]
struct ChatRequest {
    #[serde(default)]
    subject: String,
    #[serde(default)]
    grade: String,
    message: String,
    session_id: Option<String>,
    provider_id: Option<String>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    workspace: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SessionRequest {
    session_id: String,
}

#[derive(Debug, Serialize)]
struct StoredMessageInfo {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct SessionDetail {
    id: String,
    messages: Vec<StoredMessageInfo>,
}

#[derive(Debug, Serialize)]
struct ConnectionTest {
    ok: bool,
    message: String,
    latency_ms: u64,
}

#[derive(Debug, Serialize)]
struct ToolTrace {
    tool: String,
    args: String,
    output: String,
    ok: bool,
    duration_ms: u64,
}

#[derive(Debug, Serialize)]
struct ChatReply {
    session_id: String,
    text: String,
    tools: Vec<ToolTrace>,
    input_tokens: u64,
    output_tokens: u64,
    provider: String,
    model: String,
}

#[derive(Debug, Serialize)]
struct SessionInfo {
    id: String,
    subject: Option<String>,
    mode: String,
    state: String,
    messages: i64,
    started_at: String,
    summary: Option<String>,
}

#[derive(Debug, Serialize)]
struct SkillInfo {
    id: String,
    name: String,
    description: String,
    subjects: Vec<String>,
    triggers: Vec<String>,
    body: String,
    source: String,
}

#[derive(Debug, Serialize)]
struct McpServerInfo {
    name: String,
    source: String,
    command: String,
    args: Vec<String>,
    cwd: Option<String>,
    env: HashMap<String, String>,
    enabled: bool,
}

#[derive(Debug, Serialize)]
struct PluginInfo {
    id: String,
    name: String,
    version: String,
    description: String,
    path: String,
    skills: usize,
    mcp_servers: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ExtensionPaths {
    data_dir: String,
    skills_dir: String,
    plugins_dir: String,
    mcp_config: String,
}

#[derive(Debug, Serialize)]
struct ExtensionsInfo {
    skills: Vec<SkillInfo>,
    mcp: Vec<McpServerInfo>,
    plugins: Vec<PluginInfo>,
    paths: ExtensionPaths,
}

#[derive(Debug, Serialize)]
struct McpTest {
    ok: bool,
    tools: Vec<String>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct DownloadUpdateResult {
    path: String,
    version: String,
}

#[derive(Debug, Serialize)]
struct WorkspaceInfo {
    path: String,
    exists: bool,
}

fn data_dir() -> anyhow::Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("dev", "teaching-ez", "teaching-ez")
        .ok_or_else(|| anyhow::anyhow!("không xác định được thư mục dữ liệu"))?;
    let dir = dirs.data_dir().to_path_buf();
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn default_workspace() -> anyhow::Result<PathBuf> {
    Ok(data_dir()?.join("workspaces").join("default"))
}

/// Chuẩn hóa workspace cho chế độ Code; tạo thư mục nếu chưa có.
fn resolve_workspace(input: Option<&str>) -> anyhow::Result<PathBuf> {
    let path = match input.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => PathBuf::from(value),
        None => default_workspace()?,
    };
    std::fs::create_dir_all(&path)?;
    Ok(path.canonicalize().unwrap_or(path))
}

fn open_path_in_os(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    let opener = "xdg-open";
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(target_os = "windows")]
    let opener = "explorer";

    std::process::Command::new(opener)
        .arg(path)
        .spawn()
        .map(|_| ())
}

fn resolve_connection(dir: &Path, id: Option<&str>) -> anyhow::Result<ResolvedConnection> {
    let book = providers::load(dir)?;
    let entry = match id {
        Some(id) => book
            .find(id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("provider '{id}' không tồn tại"))?,
        None => book.active().cloned().ok_or_else(|| {
            anyhow::anyhow!("Chưa kết nối provider. Mở Cài đặt để thêm API key hoặc chạy: tez connect --provider groq")
        })?,
    };
    let api_key = providers::decrypt_key(dir, &entry)?;

    Ok(ResolvedConnection {
        id: entry.id,
        provider_id: entry.provider,
        provider_name: entry.name,
        base_url: entry.base_url,
        model: entry.model,
        api_key,
    })
}

fn full_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register(Calculator);
    registry.register(CheckStep);
    registry.register(ConvertUnits);
    registry.register(Readability);
    registry.register(MolarMass);
    registry.register(BalanceEquation);
    registry.register(PlotFunction);
    registry.register(PhysicsFormula);
    registry.register(CheckGrammar);
    registry
}

fn registry_for(pack: &SubjectPack) -> ToolRegistry {
    full_registry().filtered(pack.tools)
}

fn subject_infos() -> Vec<SubjectInfo> {
    subjects::ALL
        .iter()
        .map(|pack| SubjectInfo {
            id: pack.id,
            name: pack.name,
            strategy: pack.strategy,
            tools: pack.tools.to_vec(),
        })
        .collect()
}

fn friendly(error: anyhow::Error) -> String {
    format!("{error:#}")
}

fn provider_info(dir: &Path, book: &ProviderBook, entry: &ProviderEntry) -> ProviderInfo {
    let has_key = providers::decrypt_key(dir, entry)
        .map(|key| !key.is_empty())
        .unwrap_or(false);
    ProviderInfo {
        id: entry.id.clone(),
        name: entry.name.clone(),
        provider: entry.provider.clone(),
        base_url: entry.base_url.clone(),
        model: entry.model.clone(),
        has_key,
        active: book.active.as_deref() == Some(entry.id.as_str()),
    }
}

fn plugin_info(plugin: &plugins::Plugin) -> PluginInfo {
    let mut mcp_servers: Vec<String> = plugin.manifest.mcp_servers.keys().cloned().collect();
    mcp_servers.sort();
    PluginInfo {
        id: plugin.manifest.id.clone(),
        name: plugin.manifest.name.clone(),
        version: plugin.manifest.version.clone(),
        description: plugin.manifest.description.clone(),
        path: plugin.dir.display().to_string(),
        skills: plugin.manifest.skills.len(),
        mcp_servers,
    }
}

#[tauri::command]
fn app_status() -> Result<AppStatus, String> {
    let dir = data_dir().map_err(friendly)?;
    let book = providers::load(&dir).map_err(|error| error.to_string())?;
    let connection = book.active().map(|entry| ConnectionInfo {
        id: entry.id.clone(),
        name: entry.name.clone(),
        provider: entry.provider.clone(),
        model: entry.model.clone(),
        base_url: entry.base_url.clone(),
    });
    let providers = book
        .providers
        .iter()
        .map(|entry| provider_info(&dir, &book, entry))
        .collect();

    let sessions = Store::open(dir.join("tez.db"))
        .and_then(|store| store.session_count())
        .unwrap_or(0);
    let skills = SkillRegistry::load_with_plugins(&dir)
        .map(|registry| registry.all().len())
        .unwrap_or(0);
    let mcp_servers = mcp::load_servers(&dir)
        .map(|servers| servers.len())
        .unwrap_or(0);
    let plugins = plugins::load_all(&dir)
        .map(|items| items.len())
        .unwrap_or(0);

    Ok(AppStatus {
        connection,
        providers,
        subjects: subject_infos(),
        sessions,
        data_dir: dir.display().to_string(),
        default_workspace: default_workspace()
            .map(|path| path.display().to_string())
            .unwrap_or_default(),
        version: env!("CARGO_PKG_VERSION"),
        extensions: ExtensionSummary {
            skills,
            mcp_servers,
            plugins,
        },
    })
}

#[tauri::command]
fn list_providers() -> Result<Vec<ProviderInfo>, String> {
    let dir = data_dir().map_err(friendly)?;
    let book = providers::load(&dir).map_err(|error| error.to_string())?;
    Ok(book
        .providers
        .iter()
        .map(|entry| provider_info(&dir, &book, entry))
        .collect())
}

#[tauri::command]
fn save_provider(request: SaveProviderRequest) -> Result<ProviderInfo, String> {
    let dir = data_dir().map_err(friendly)?;
    let entry = providers::upsert(
        &dir,
        ProviderInput {
            id: request.id.as_deref(),
            name: &request.name,
            provider: &request.provider,
            base_url: request.base_url.as_deref(),
            model: request.model.as_deref(),
            api_key: request.api_key.as_deref(),
            set_active: request.set_active,
        },
    )
    .map_err(|error| error.to_string())?;

    let store = Store::open(dir.join("tez.db")).map_err(|error| error.to_string())?;
    store
        .ensure_profile("default", "Học sinh")
        .map_err(|error| error.to_string())?;

    let book = providers::load(&dir).map_err(|error| error.to_string())?;
    Ok(provider_info(&dir, &book, &entry))
}

#[tauri::command]
fn delete_provider(id: String) -> Result<bool, String> {
    let dir = data_dir().map_err(friendly)?;
    providers::remove(&dir, &id).map_err(|error| error.to_string())
}

#[tauri::command]
fn activate_provider(id: String) -> Result<ProviderInfo, String> {
    let dir = data_dir().map_err(friendly)?;
    let entry = providers::set_active(&dir, &id).map_err(|error| error.to_string())?;
    let book = providers::load(&dir).map_err(|error| error.to_string())?;
    Ok(provider_info(&dir, &book, &entry))
}

#[tauri::command]
async fn test_provider(id: Option<String>) -> Result<ConnectionTest, String> {
    test_provider_inner(id.as_deref()).await.map_err(friendly)
}

async fn test_provider_inner(id: Option<&str>) -> anyhow::Result<ConnectionTest> {
    let dir = data_dir()?;
    let saved = resolve_connection(&dir, id)?;
    let provider = OpenAiCompatible::new(&saved.provider_id, &saved.base_url, &saved.api_key);
    let started = Instant::now();
    let message = tez_llm::ping(&provider, &saved.model).await?;
    let latency_ms = started.elapsed().as_millis() as u64;

    Ok(ConnectionTest {
        ok: true,
        message: message.trim().to_string(),
        latency_ms,
    })
}

#[tauri::command]
async fn test_connection() -> Result<ConnectionTest, String> {
    test_provider(None).await
}

#[tauri::command]
fn list_extensions() -> Result<ExtensionsInfo, String> {
    let dir = data_dir().map_err(friendly)?;
    let registry = SkillRegistry::load_with_plugins(&dir).map_err(|error| error.to_string())?;
    let skills = registry
        .all()
        .iter()
        .map(|skill| SkillInfo {
            id: skill.id.clone(),
            name: skill.name.clone(),
            description: skill.description.clone(),
            subjects: skill.subjects.clone(),
            triggers: skill.triggers.clone(),
            body: skill.body.clone(),
            source: skill.source.clone(),
        })
        .collect();

    let mcp = mcp::load_servers(&dir)
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|entry| McpServerInfo {
            name: entry.name,
            source: entry.source,
            command: entry.config.command,
            args: entry.config.args,
            cwd: entry.config.cwd,
            env: entry.config.env,
            enabled: entry.config.enabled,
        })
        .collect();

    let plugins = plugins::load_all(&dir)
        .map_err(|error| error.to_string())?
        .iter()
        .map(plugin_info)
        .collect();

    Ok(ExtensionsInfo {
        skills,
        mcp,
        plugins,
        paths: ExtensionPaths {
            data_dir: dir.display().to_string(),
            skills_dir: dir.join("skills").display().to_string(),
            plugins_dir: dir.join("plugins").display().to_string(),
            mcp_config: mcp::config_path(&dir).display().to_string(),
        },
    })
}

#[tauri::command]
fn save_mcp_server(request: SaveMcpRequest) -> Result<(), String> {
    if request.name.trim().is_empty() {
        return Err("Tên MCP server không được để trống.".into());
    }
    if request.command.trim().is_empty() {
        return Err("Command không được để trống.".into());
    }

    let dir = data_dir().map_err(friendly)?;
    let mut config = mcp::load_config(&dir).map_err(|error| error.to_string())?;
    config.servers.insert(
        request.name.trim().to_string(),
        McpServerConfig {
            command: request.command.trim().to_string(),
            args: request.args,
            env: request.env,
            cwd: request.cwd.filter(|value| !value.trim().is_empty()),
            enabled: request.enabled,
        },
    );
    mcp::save_config(&dir, &config).map_err(|error| error.to_string())
}

#[tauri::command]
fn delete_mcp_server(name: String) -> Result<bool, String> {
    let dir = data_dir().map_err(friendly)?;
    let mut config = mcp::load_config(&dir).map_err(|error| error.to_string())?;
    if config.servers.remove(&name).is_none() {
        return Ok(false);
    }
    mcp::save_config(&dir, &config).map_err(|error| error.to_string())?;
    Ok(true)
}

#[tauri::command]
async fn test_mcp_server(name: String) -> Result<McpTest, String> {
    let dir = data_dir().map_err(friendly)?;
    let servers = mcp::load_servers(&dir).map_err(|error| error.to_string())?;
    let entry = servers
        .into_iter()
        .find(|entry| entry.name == name)
        .ok_or_else(|| format!("MCP server '{name}' không tồn tại"))?;

    match mcp::McpClient::connect(&entry.name, &entry.config).await {
        Ok((_client, tools)) => Ok(McpTest {
            ok: true,
            tools: tools.into_iter().map(|tool| tool.name).collect(),
            error: None,
        }),
        Err(error) => Ok(McpTest {
            ok: false,
            tools: Vec::new(),
            error: Some(error.to_string()),
        }),
    }
}

#[tauri::command]
fn install_sample_plugin() -> Result<PluginInfo, String> {
    let dir = data_dir().map_err(friendly)?;
    let root = dir.join("plugins").join("study-toolkit");
    std::fs::create_dir_all(root.join("skills").join("exam-strategy"))
        .map_err(|error| error.to_string())?;
    std::fs::write(root.join("plugin.json"), SAMPLE_PLUGIN_JSON)
        .map_err(|error| error.to_string())?;
    std::fs::write(root.join("server.py"), SAMPLE_PLUGIN_SERVER)
        .map_err(|error| error.to_string())?;
    std::fs::write(
        root.join("skills").join("exam-strategy").join("SKILL.md"),
        SAMPLE_PLUGIN_SKILL,
    )
    .map_err(|error| error.to_string())?;

    let plugins = plugins::load_all(&dir).map_err(|error| error.to_string())?;
    let plugin = plugins
        .iter()
        .find(|plugin| plugin.manifest.id == "study-toolkit")
        .ok_or_else(|| "không nạp được plugin mẫu".to_string())?;
    Ok(plugin_info(plugin))
}

#[tauri::command]
fn open_extension_path(kind: String) -> Result<String, String> {
    let dir = data_dir().map_err(friendly)?;
    let path = match kind.as_str() {
        "skills" => {
            let path = dir.join("skills");
            std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;
            path
        }
        "plugins" => {
            let path = dir.join("plugins");
            std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;
            path
        }
        "data" => dir.clone(),
        "mcp" => {
            let path = mcp::config_path(&dir);
            if !path.exists() {
                mcp::save_config(&dir, &mcp::McpConfig::default())
                    .map_err(|error| error.to_string())?;
            }
            path
        }
        _ => return Err(format!("loại đường dẫn không hỗ trợ: {kind}")),
    };

    #[cfg(target_os = "linux")]
    let opener = "xdg-open";
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(target_os = "windows")]
    let opener = "explorer";

    std::process::Command::new(opener)
        .arg(&path)
        .spawn()
        .map_err(|error| format!("không mở được {}: {error}", path.display()))?;
    Ok(path.display().to_string())
}

#[tauri::command]
fn list_sessions(limit: Option<usize>) -> Result<Vec<SessionInfo>, String> {
    let dir = data_dir().map_err(friendly)?;
    let store = Store::open(dir.join("tez.db")).map_err(|error| error.to_string())?;
    let rows = store
        .list_sessions(limit.unwrap_or(20))
        .map_err(|error| error.to_string())?;

    Ok(rows
        .into_iter()
        .map(|row| SessionInfo {
            id: row.id,
            subject: row.subject,
            mode: row.mode,
            state: row.state,
            messages: row.messages,
            started_at: row.started_at,
            summary: row.summary,
        })
        .collect())
}

#[tauri::command]
fn load_session(request: SessionRequest) -> Result<SessionDetail, String> {
    let dir = data_dir().map_err(friendly)?;
    let store = Store::open(dir.join("tez.db")).map_err(|error| error.to_string())?;
    if !store
        .session_exists(&request.session_id)
        .map_err(|error| error.to_string())?
    {
        return Err(format!("session '{}' không tồn tại", request.session_id));
    }

    let messages = store
        .load_messages(&request.session_id, 200)
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|message| StoredMessageInfo {
            role: message.role,
            content: message.content,
        })
        .collect();

    Ok(SessionDetail {
        id: request.session_id,
        messages,
    })
}

#[tauri::command]
async fn chat(request: ChatRequest) -> Result<ChatReply, String> {
    chat_inner(request).await.map_err(friendly)
}

async fn chat_inner(request: ChatRequest) -> anyhow::Result<ChatReply> {
    let dir = data_dir()?;
    let mode = Mode::parse(request.mode.as_deref().unwrap_or("chat"));
    let pack = subjects::resolve_or_general(&request.subject);
    let workspace = match mode {
        Mode::Code => Some(resolve_workspace(request.workspace.as_deref())?),
        Mode::Chat => None,
    };
    let connection = resolve_connection(&dir, request.provider_id.as_deref())?;

    let store = Store::open(dir.join("tez.db"))?;
    store.ensure_profile("default", "Học sinh")?;

    let (session_id, history) = match request.session_id.as_deref() {
        Some(id) => {
            if !store.session_exists(id)? {
                anyhow::bail!("session '{id}' không tồn tại");
            }
            store.resume_session(id)?;
            let history = store
                .load_messages(id, 20)?
                .into_iter()
                .filter_map(|message| match message.role.as_str() {
                    "user" => Some(Message::user(message.content)),
                    "assistant" => Some(Message::assistant(message.content)),
                    _ => None,
                })
                .collect();
            (id.to_string(), history)
        }
        None => (
            store.create_session_mode("default", None, Some(pack.id), mode.as_str())?,
            Vec::new(),
        ),
    };

    let subject_filter = if pack.is_general() {
        None
    } else {
        Some(pack.id)
    };
    let hits = store.search_chunks(&request.message, subject_filter, 4)?;
    let excerpts: Vec<String> = hits
        .iter()
        .map(|hit| format!("[{}] {}", hit.title, hit.snippet))
        .collect();

    store.append_message(NewMessage {
        session_id: &session_id,
        role: "user",
        content: &request.message,
        model: None,
        tokens_in: 0,
        tokens_out: 0,
    })?;
    drop(store);

    let mut recent = history;
    recent.push(Message::user(request.message.clone()));

    let registry = SkillRegistry::load_with_plugins(&dir)?;
    let matched = registry.matching(pack.id, &request.message, 2);
    let skill_snippets: Vec<String> = matched.iter().map(|skill| skills::render(skill)).collect();

    let objectives: Vec<String> = Vec::new();
    let workspace_label = workspace
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    let messages = ContextBuilder.build(ContextInput {
        subject: pack,
        grade: &request.grade,
        objectives: &objectives,
        excerpts: &excerpts,
        student_summary: None,
        state: "practice",
        recent: &recent,
        skills: &skill_snippets,
        mode,
        workspace: Some(&workspace_label),
    });

    let provider: Arc<dyn LlmProvider> = Arc::new(OpenAiCompatible::new(
        &connection.provider_id,
        &connection.base_url,
        &connection.api_key,
    ));

    let mut tools = match (&mode, workspace.as_deref()) {
        (Mode::Code, Some(root)) => code_registry(root)?,
        _ => registry_for(pack),
    };
    let discovery = mcp::discover(&dir).await?;
    for tool in discovery.tools {
        tools.register(tool);
    }

    let agent = Agent::new(provider, tools);
    let started = Instant::now();
    let outcome = agent.run(&connection.model, messages).await?;
    let latency_ms = started.elapsed().as_millis() as u64;

    let store = Store::open(dir.join("tez.db"))?;
    store.append_message(NewMessage {
        session_id: &session_id,
        role: "assistant",
        content: &outcome.text,
        model: Some(&connection.model),
        tokens_in: outcome.usage.input_tokens,
        tokens_out: outcome.usage.output_tokens,
    })?;
    for step in &outcome.steps {
        store.record_tool_call(
            &session_id,
            &step.tool,
            &step.args,
            &step.output,
            step.ok,
            step.duration_ms,
        )?;
    }
    store.record_usage(UsageRecord {
        profile_id: "default",
        connection_id: Some(&connection.id),
        provider_id: &connection.provider_id,
        model_id: &connection.model,
        task: mode.as_str(),
        tokens_in: outcome.usage.input_tokens,
        tokens_out: outcome.usage.output_tokens,
        latency_ms: Some(latency_ms),
        status: "ok",
    })?;
    store.resume_session(&session_id)?;
    drop(store);

    Ok(ChatReply {
        session_id,
        text: outcome.text,
        tools: outcome
            .steps
            .iter()
            .map(|step| ToolTrace {
                tool: step.tool.clone(),
                args: step.args.to_string(),
                output: step.output.clone(),
                ok: step.ok,
                duration_ms: step.duration_ms,
            })
            .collect(),
        input_tokens: outcome.usage.input_tokens,
        output_tokens: outcome.usage.output_tokens,
        provider: connection.provider_name,
        model: connection.model,
    })
}

#[tauri::command]
async fn check_update() -> Result<tez_update::UpdateInfo, String> {
    tez_update::check(env!("CARGO_PKG_VERSION"))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn download_update() -> Result<DownloadUpdateResult, String> {
    let info = tez_update::check(env!("CARGO_PKG_VERSION"))
        .await
        .map_err(|error| error.to_string())?;
    if !info.available {
        return Err("Bạn đang dùng bản mới nhất.".into());
    }
    let asset = tez_update::pick_asset(&info, std::env::consts::OS)
        .ok_or_else(|| "Bản phát hành không có file phù hợp với hệ điều hành này.".to_string())?;
    let dir = data_dir().map_err(friendly)?.join("updates");
    let path = tez_update::download(asset, &dir)
        .await
        .map_err(|error| error.to_string())?;
    Ok(DownloadUpdateResult {
        path: path.display().to_string(),
        version: info.latest,
    })
}

#[tauri::command]
fn launch_update(path: String) -> Result<(), String> {
    let path = PathBuf::from(&path);
    if !path.exists() {
        return Err(format!(
            "không tìm thấy file cập nhật: {path}",
            path = path.display()
        ));
    }
    open_path_in_os(&path).map_err(|error| format!("không mở được bản cập nhật: {error}"))
}

#[tauri::command]
fn open_external(url: String) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("chỉ hỗ trợ liên kết http(s)".into());
    }
    open_path_in_os(Path::new(&url)).map_err(|error| format!("không mở được liên kết: {error}"))
}

#[tauri::command]
fn open_workspace(path: Option<String>) -> Result<WorkspaceInfo, String> {
    let resolved = resolve_workspace(path.as_deref()).map_err(friendly)?;
    open_path_in_os(&resolved).map_err(|error| format!("không mở được workspace: {error}"))?;
    Ok(WorkspaceInfo {
        path: resolved.display().to_string(),
        exists: true,
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            app_status,
            chat,
            list_sessions,
            load_session,
            list_providers,
            save_provider,
            delete_provider,
            activate_provider,
            test_provider,
            test_connection,
            list_extensions,
            save_mcp_server,
            delete_mcp_server,
            test_mcp_server,
            install_sample_plugin,
            open_extension_path,
            check_update,
            download_update,
            launch_update,
            open_external,
            open_workspace
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
