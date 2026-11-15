use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tez_auth::providers::{self, ProviderBook, ProviderEntry, ProviderInput};
use tez_core::agent::{Agent, AgentConfig, RunContext, StopReason};
use tez_core::compaction::{session_artifact_dir, CompactReport, Compactor};
use tez_core::context::{ContextBuilder, ContextInput};
use tez_core::harness::{self, HarnessConfig};
use tez_core::hooks::{self};
use tez_core::mcp::{self, McpServerConfig};
use tez_core::memory::{self, MemoryForget, MemorySave, MemorySearch, MemoryStore};
use tez_core::modes::Mode;
use tez_core::permission::{
    ApprovalRequest, Approver, AutoApprover, PermissionMode, PermissionPolicy,
};
use tez_core::plugins;
use tez_core::skills::{self, SkillRegistry};
use tez_core::subagent::{SubagentRuntime, TaskTool};
use tez_core::subjects;
use tez_core::todo::{TodoItem, TodoState, TodoWrite};
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
use tez_llm::{
    ChatRequest as LlmChatRequest, ChatResponse, LlmProvider, Message, OpenAiCompatible,
};
use tez_store::{NewMessage, Store, UsageRecord};

const SAMPLE_PLUGIN_JSON: &str =
    include_str!("../../../../examples/plugins/study-toolkit/plugin.json");
const SAMPLE_PLUGIN_SERVER: &str =
    include_str!("../../../../examples/plugins/study-toolkit/server.py");
const SAMPLE_PLUGIN_SKILL: &str =
    include_str!("../../../../examples/plugins/study-toolkit/skills/exam-strategy/SKILL.md");

/// Dọn session mới nếu lượt chat lỗi giữa đường (key sai, mạng, MCP...).
struct NewSessionGuard {
    db: PathBuf,
    id: String,
    armed: bool,
}

impl Drop for NewSessionGuard {
    fn drop(&mut self) {
        if self.armed {
            if let Ok(store) = Store::open(&self.db) {
                let _ = store.abandon_session(&self.id);
            }
        }
    }
}

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

#[derive(Debug, Deserialize)]
struct DraftExtensionRequest {
    kind: String,
    prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DraftFile {
    path: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct DraftExtensionResult {
    kind: String,
    files: Vec<DraftFile>,
    notes: String,
}

#[derive(Debug, Deserialize)]
struct SaveExtensionRequest {
    kind: String,
    files: Vec<DraftFile>,
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
    todos: Vec<TodoItem>,
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
    todos: Vec<TodoItem>,
    compactions: CompactReport,
    permission_denials: usize,
    stop_reason: String,
}

/// Kênh chờ người dùng bấm duyệt trong desktop.
#[derive(Default)]
struct ApprovalBroker {
    pending: Mutex<HashMap<String, tokio::sync::oneshot::Sender<bool>>>,
}

impl ApprovalBroker {
    fn register(&self, id: &str) -> tokio::sync::oneshot::Receiver<bool> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(id.to_string(), tx);
        }
        rx
    }

    fn respond(&self, id: &str, approved: bool) -> bool {
        let sender = self
            .pending
            .lock()
            .ok()
            .and_then(|mut pending| pending.remove(id));
        match sender {
            Some(tx) => tx.send(approved).is_ok(),
            None => false,
        }
    }

    fn cancel(&self, id: &str) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(id);
        }
    }
}

/// Cho phép UI dừng lượt agent đang chạy.
#[derive(Default)]
struct StopBroker {
    current: Mutex<Option<tokio::sync::watch::Sender<bool>>>,
}

impl StopBroker {
    fn register(&self) -> tokio::sync::watch::Receiver<bool> {
        let (tx, rx) = tokio::sync::watch::channel(false);
        if let Ok(mut current) = self.current.lock() {
            *current = Some(tx);
        }
        rx
    }

    fn clear(&self) {
        if let Ok(mut current) = self.current.lock() {
            current.take();
        }
    }

    fn stop(&self) -> bool {
        match self.current.lock().ok().and_then(|current| current.as_ref().cloned()) {
            Some(tx) => tx.send(true).is_ok(),
            None => false,
        }
    }
}

#[derive(Clone, Serialize)]
struct ChatDelta {
    session_id: String,
    text: String,
}

struct DesktopApprover {
    app: AppHandle,
    broker: Arc<ApprovalBroker>,
}

#[async_trait]
impl Approver for DesktopApprover {
    async fn approve(&self, request: &ApprovalRequest) -> bool {
        let receiver = self.broker.register(&request.id);
        if let Err(error) = self.app.emit("tez://approval-request", request) {
            eprintln!("[permission] không gửi được yêu cầu duyệt: {error}");
            self.broker.cancel(&request.id);
            return false;
        }
        match tokio::time::timeout(Duration::from_secs(180), receiver).await {
            Ok(Ok(approved)) => approved,
            Ok(Err(_)) => false,
            Err(_) => {
                self.broker.cancel(&request.id);
                false
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct RespondApprovalRequest {
    id: String,
    approved: bool,
}

#[derive(Debug, Deserialize)]
struct MemoryNameRequest {
    name: String,
}

#[derive(Debug, Serialize)]
struct MemoryInfo {
    name: String,
    description: String,
    kind: String,
    body: String,
    path: String,
}

#[derive(Debug, Serialize)]
struct HarnessInfo {
    config: HarnessConfig,
    path: String,
    memory_dir: String,
    audit_log: String,
    memory: Vec<MemoryInfo>,
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
    if api_key.trim().is_empty() && tez_auth::connection::requires_api_key(&entry.provider) {
        anyhow::bail!(
            "Chưa có API key cho '{}'. Mở Cài đặt để nhập key, hoặc dùng provider \
             ollama/custom nếu endpoint không cần key.",
            entry.provider
        );
    }

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
    // Mở DB trước khi ghi provider: nếu DB lỗi thì không báo "lưu thất bại"
    // trong khi providers.json đã được ghi.
    let store = Store::open(dir.join("tez.db")).map_err(|error| error.to_string())?;
    store
        .ensure_profile("default", "Học sinh")
        .map_err(|error| error.to_string())?;
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
    let message = tez_llm::ping(&provider, &saved.model)
        .await
        .map_err(|error| match &error {
            tez_llm::LlmError::Api { status, .. } if *status == 401 || *status == 403 => {
                anyhow::anyhow!(
                    "API key bị provider từ chối (HTTP {status}). Kiểm tra lại key trong Cài đặt."
                )
            }
            tez_llm::LlmError::Api { status, .. } if *status == 429 => {
                anyhow::anyhow!("Provider đang giới hạn tốc độ (HTTP 429). Chờ rồi thử lại.")
            }
            tez_llm::LlmError::Http(err) if err.is_timeout() => {
                anyhow::anyhow!("Provider không phản hồi kịp (timeout). Kiểm tra mạng/base_url.")
            }
            _ => anyhow::anyhow!(error),
        })?;
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

fn extension_system_prompt(kind: &str) -> String {
    match kind {
        "skill" => r#"Bạn là chuyên gia viết skill cho AI agent local-first.
Hãy tạo một skill Markdown ngắn gọn, có thể dùng ngay.
Chỉ trả về JSON hợp lệ, không markdown, không giải thích:
{"files":[{"path":"skills/<slug>/SKILL.md","content":"---\nname: <tên>\ndescription: <mô tả ngắn>\nsubjects: all\ntriggers: <từ khóa, phân tách bằng dấu phẩy>\n---\n<chỉ dẫn thực tế, 5-12 dòng>"}]}
Quy tắc: slug chỉ gồm chữ thường, số và dấu gạch ngang; nội dung là chỉ dẫn cho agent, không phải lời chào; không bịa tool không tồn tại."#
            .to_string(),
        "mcp" => r#"Bạn là chuyên gia cấu hình MCP server.
Hãy đề xuất một MCP server stdio thực tế, cài được bằng command phổ biến (npx, uvx, python3...).
Chỉ trả về JSON hợp lệ, không markdown, không giải thích:
{"name":"<tên ngắn>","command":"<lệnh>","args":["..."],"env":{},"cwd":null,"enabled":true}
Không bịa package không tồn tại; nếu cần đường dẫn, dùng placeholder rõ ràng. Không đưa secret vào env."#
            .to_string(),
        "plugin" => r#"Bạn là chuyên gia đóng gói extension cho AI agent.
Hãy tạo một plugin gồm manifest và một skill Markdown.
Chỉ trả về JSON hợp lệ, không markdown, không giải thích:
{"files":[
  {"path":"plugins/<slug>/plugin.json","content":"{\"id\":\"<slug>\",\"name\":\"<tên>\",\"version\":\"0.1.0\",\"description\":\"<mô tả>\",\"skills\":[\"skills/<slug>\"],\"mcpServers\":{}}"},
  {"path":"plugins/<slug>/skills/<slug>/SKILL.md","content":"---\nname: <tên>\ndescription: <mô tả>\nsubjects: all\ntriggers: <từ khóa>\n---\n<chỉ dẫn>"}
]}
Quy tắc: slug chỉ gồm chữ thường, số và dấu gạch ngang; manifest phải là JSON hợp lệ; không bịa tool."#
            .to_string(),
        _ => String::new(),
    }
}

fn strip_code_fence(raw: &str) -> String {
    let mut text = raw.trim();
    if text.starts_with("```") {
        if let Some(newline) = text.find('\n') {
            text = &text[newline + 1..];
        }
        if let Some(end) = text.rfind("```") {
            text = &text[..end];
        }
    }
    text.trim().to_string()
}

fn parse_draft_files(raw: &str) -> Result<Vec<DraftFile>, String> {
    let text = strip_code_fence(raw);
    let start = text
        .find(['{', '['])
        .ok_or_else(|| "AI không trả về JSON. Thử lại với mô tả cụ thể hơn.".to_string())?;
    let end = text
        .rfind(['}', ']'])
        .ok_or_else(|| "AI trả về JSON chưa hoàn chỉnh. Thử lại.".to_string())?;
    let slice = &text[start..=end];

    #[derive(Deserialize)]
    struct Envelope {
        files: Vec<DraftFile>,
    }

    if let Ok(envelope) = serde_json::from_str::<Envelope>(slice) {
        if !envelope.files.is_empty() {
            return Ok(envelope.files);
        }
    }
    if let Ok(files) = serde_json::from_str::<Vec<DraftFile>>(slice) {
        if !files.is_empty() {
            return Ok(files);
        }
    }
    if let Ok(file) = serde_json::from_str::<DraftFile>(slice) {
        return Ok(vec![file]);
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(slice) {
        if value.get("name").is_some() && value.get("command").is_some() {
            return Ok(vec![DraftFile {
                path: "mcp.json".to_string(),
                content: serde_json::to_string_pretty(&value)
                    .map_err(|error| format!("không serialize được MCP: {error}"))?,
            }]);
        }
    }

    Err("AI trả về JSON không đúng định dạng. Thử lại với mô tả ngắn và rõ hơn.".to_string())
}

fn safe_extension_path(kind: &str, raw: &str) -> Result<PathBuf, String> {
    let path = Path::new(raw.trim());
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(format!("đường dẫn không hợp lệ: {raw}"));
    }
    let expected = match kind {
        "skill" => "skills",
        "plugin" => "plugins",
        _ => return Err(format!("loại extension không hỗ trợ: {kind}")),
    };
    let mut components = path.components();
    let first = match components.next() {
        Some(Component::Normal(value)) => value.to_string_lossy().to_string(),
        _ => return Err(format!("đường dẫn không hợp lệ: {raw}")),
    };
    if first != expected {
        return Err(format!("file phải nằm trong {expected}/: {raw}"));
    }
    for component in components {
        if !matches!(component, Component::Normal(_)) {
            return Err(format!("đường dẫn không được chứa '..': {raw}"));
        }
    }
    if kind == "skill" && !path.ends_with("SKILL.md") {
        return Err(format!("skill phải là file SKILL.md: {raw}"));
    }
    Ok(path.to_path_buf())
}

#[tauri::command]
async fn draft_extension(request: DraftExtensionRequest) -> Result<DraftExtensionResult, String> {
    let kind = request.kind.trim().to_lowercase();
    if !matches!(kind.as_str(), "skill" | "mcp" | "plugin") {
        return Err(format!("loại extension không hỗ trợ: {kind}"));
    }
    let prompt = request.prompt.trim();
    if prompt.is_empty() {
        return Err("Hãy mô tả extension bạn muốn tạo.".into());
    }

    let dir = data_dir().map_err(friendly)?;
    let connection = resolve_connection(&dir, None).map_err(friendly)?;
    let provider = OpenAiCompatible::new(
        &connection.provider_id,
        &connection.base_url,
        &connection.api_key,
    );
    let response = provider
        .chat(LlmChatRequest {
            model: connection.model.clone(),
            messages: vec![
                Message::system(extension_system_prompt(&kind)),
                Message::user(prompt),
            ],
            tools: Vec::new(),
            temperature: Some(0.3),
            top_p: Some(0.95),
            max_tokens: Some(4_096),
        })
        .await
        .map_err(|error| error.to_string())?;
    let content = match response {
        ChatResponse::Text { content, .. } => content,
        ChatResponse::ToolCalls { content, .. } => content.unwrap_or_default(),
    };
    let files = parse_draft_files(&content)?;
    Ok(DraftExtensionResult {
        kind,
        files,
        notes: "Bản nháp do AI tạo. Kiểm tra nội dung trước khi lưu.".to_string(),
    })
}

#[tauri::command]
fn save_extension(request: SaveExtensionRequest) -> Result<Vec<String>, String> {
    let kind = request.kind.trim().to_lowercase();
    if !matches!(kind.as_str(), "skill" | "mcp" | "plugin") {
        return Err(format!("loại extension không hỗ trợ: {kind}"));
    }
    if request.files.is_empty() {
        return Err("Không có file nào để lưu.".into());
    }
    if request.files.len() > 20 {
        return Err("Quá nhiều file trong một lần lưu.".into());
    }

    let dir = data_dir().map_err(friendly)?;

    if kind == "mcp" {
        #[derive(Deserialize)]
        struct DraftMcp {
            name: String,
            command: String,
            #[serde(default)]
            args: Vec<String>,
            #[serde(default)]
            env: HashMap<String, String>,
            #[serde(default)]
            cwd: Option<String>,
            #[serde(default = "default_true")]
            enabled: bool,
        }

        let file = request
            .files
            .iter()
            .find(|file| file.path.ends_with("mcp.json"))
            .or_else(|| request.files.first())
            .ok_or_else(|| "Thiếu file cấu hình MCP.".to_string())?;
        let draft: DraftMcp = serde_json::from_str(&file.content)
            .map_err(|error| format!("MCP JSON không hợp lệ: {error}"))?;
        if draft.name.trim().is_empty() || draft.command.trim().is_empty() {
            return Err("MCP cần có name và command.".into());
        }
        let mut config = mcp::load_config(&dir).map_err(|error| error.to_string())?;
        config.servers.insert(
            draft.name.trim().to_string(),
            McpServerConfig {
                command: draft.command.trim().to_string(),
                args: draft.args,
                env: draft.env,
                cwd: draft.cwd.filter(|value| !value.trim().is_empty()),
                enabled: draft.enabled,
            },
        );
        mcp::save_config(&dir, &config).map_err(|error| error.to_string())?;
        return Ok(vec![format!("mcp:{}", draft.name.trim())]);
    }

    let mut written = Vec::new();
    for file in &request.files {
        if file.content.chars().count() > 200_000 {
            return Err(format!("file quá lớn: {}", file.path));
        }
        let relative = safe_extension_path(&kind, &file.path)?;
        let full = dir.join(&relative);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("không tạo được thư mục: {error}"))?;
        }
        std::fs::write(&full, &file.content)
            .map_err(|error| format!("không ghi được {}: {error}", full.display()))?;
        written.push(full.display().to_string());
    }
    Ok(written)
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

    let todos = store
        .load_todos(&request.session_id)
        .map_err(|error| error.to_string())?
        .and_then(|json| serde_json::from_str::<Vec<TodoItem>>(&json).ok())
        .unwrap_or_default();

    Ok(SessionDetail {
        id: request.session_id,
        messages,
        todos,
    })
}

#[tauri::command]
async fn chat(
    app: AppHandle,
    broker: State<'_, Arc<ApprovalBroker>>,
    stop: State<'_, Arc<StopBroker>>,
    request: ChatRequest,
) -> Result<ChatReply, String> {
    let cancel = stop.register();
    let result = chat_inner(app, Arc::clone(&broker), request, cancel).await;
    stop.clear();
    result.map_err(|error| match error.downcast_ref::<tez_llm::LlmError>() {
            Some(tez_llm::LlmError::Api { status, .. }) if *status == 401 || *status == 403 => {
                format!(
                    "API key bị provider từ chối (HTTP {status}). Kiểm tra lại key trong Cài đặt."
                )
            }
            Some(tez_llm::LlmError::Api { status, .. }) if *status == 429 => {
                format!("Provider đang giới hạn tốc độ (HTTP {status}). Chờ rồi thử lại.")
            }
            _ => friendly(error),
        })
}

#[tauri::command]
fn stop_chat(stop: State<'_, Arc<StopBroker>>) -> bool {
    stop.stop()
}

async fn chat_inner(
    app: AppHandle,
    broker: Arc<ApprovalBroker>,
    request: ChatRequest,
    cancel: tokio::sync::watch::Receiver<bool>,
) -> anyhow::Result<ChatReply> {
    let stream_app = app.clone();
    let dir = data_dir()?;
    let mode = Mode::parse(request.mode.as_deref().unwrap_or("chat"));
    let workspace = match mode {
        Mode::Code => Some(resolve_workspace(request.workspace.as_deref())?),
        Mode::Chat => None,
    };
    let connection = resolve_connection(&dir, request.provider_id.as_deref())?;

    let config = harness::load(&dir);
    let policy = Arc::new(PermissionPolicy::new(config.permission_mode));
    let approver: Arc<dyn Approver> = if config.permission_mode == PermissionMode::Trust {
        Arc::new(AutoApprover)
    } else {
        Arc::new(DesktopApprover {
            app,
            broker: Arc::clone(&broker),
        })
    };
    let hooks = Arc::new(hooks::build_registry(Some(&dir), &config.hooks));
    let memory_store = MemoryStore::new(dir.join("memory"));

    let store = Store::open(dir.join("tez.db"))?;
    store.ensure_profile("default", "Người dùng")?;

    let (session_id, history, todo_json, created) = match request.session_id.as_deref() {
        Some(id) => {
            if !store.session_exists(id)? {
                anyhow::bail!("session '{id}' không tồn tại");
            }
            store.resume_session(id)?;
            let history = store
                .load_messages(id, 40)?
                .into_iter()
                .filter_map(|message| match message.role.as_str() {
                    "user" => Some(Message::user(message.content)),
                    "assistant" => Some(Message::assistant(message.content)),
                    _ => None,
                })
                .collect();
            (id.to_string(), history, store.load_todos(id)?, false)
        }
        None => (
            store.create_session_mode("default", None, None, mode.as_str())?,
            Vec::new(),
            None,
            true,
        ),
    };
    let (delta_tx, mut delta_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let emitter_session = session_id.clone();
    let emitter = tokio::spawn(async move {
        while let Some(text) = delta_rx.recv().await {
            let _ = stream_app.emit(
                "tez://chat-delta",
                ChatDelta {
                    session_id: emitter_session.clone(),
                    text,
                },
            );
        }
    });

    let mut session_guard = created.then(|| NewSessionGuard {        db: dir.join("tez.db"),
        id: session_id.clone(),
        armed: true,
    });

    let todos = Arc::new(TodoState::new());
    if let Some(json) = todo_json {
        if let Ok(items) = serde_json::from_str::<Vec<TodoItem>>(&json) {
            let _ = todos.replace(items);
        }
    }

    let hits = store.search_chunks(&request.message, None, 4)?;
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
    let matched = registry.matching_query(&request.message, 2);
    let skill_snippets: Vec<String> = matched.iter().map(|skill| skills::render(skill)).collect();

    let recalled = if config.memory.enabled && config.memory.auto_recall {
        let records = memory_store.search(&request.message, config.memory.recall_limit);
        memory::render_recalled(&records)
    } else {
        Vec::new()
    };
    let todo_snapshot = todos.snapshot();

    let workspace_label = workspace
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    let messages = ContextBuilder.build(ContextInput {
        recent: &recent,
        excerpts: &excerpts,
        skills: &skill_snippets,
        memory: &recalled,
        todos: &todo_snapshot,
        mode,
        workspace: Some(&workspace_label),
        instructions: &config.system_prompt,
        language: &config.language,
    });

    let provider: Arc<dyn LlmProvider> = Arc::new(OpenAiCompatible::new(
        &connection.provider_id,
        &connection.base_url,
        &connection.api_key,
    ));

    let mut tools = match (&mode, workspace.as_deref()) {
        (Mode::Code, Some(root)) => code_registry(root)?,
        _ => full_registry(),
    };
    tools.register(MemorySave::new(memory_store.clone()));
    tools.register(MemorySearch::new(memory_store.clone()));
    tools.register(MemoryForget::new(memory_store.clone()));
    tools.register(TodoWrite::new(Arc::clone(&todos)));
    let discovery = mcp::discover(&dir).await?;
    for tool in discovery.tools {
        tools.register(tool);
    }

    if mode == Mode::Code {
        let runtime = Arc::new(SubagentRuntime {
            provider: Arc::clone(&provider),
            model: connection.model.clone(),
            tools: tools.clone(),
            policy: Arc::clone(&policy),
            approver: Arc::clone(&approver),
            hooks: Arc::clone(&hooks),
            max_steps: config.subagent_max_steps,
            compaction: config.compaction.clone(),
        });
        tools.register(TaskTool::new(runtime));
    }

    if !config.disabled_tools.is_empty() {
        tools = tools.without(&config.disabled_tools);
    }

    let artifact_dir = session_artifact_dir(&dir, &session_id);
    let compactor = Compactor::new(config.compaction.clone(), Some(artifact_dir.clone()));
    let agent = Agent::new(provider, tools)
        .with_config(AgentConfig {
            max_steps: config.max_steps,
            max_tokens: config.max_tokens,
            temperature: config.temperature,
            top_p: config.top_p,
            max_tool_output: config.max_tool_output,
            ..AgentConfig::default()
        })
        .with_policy(Arc::clone(&policy))
        .with_approver(Arc::clone(&approver))
        .with_hooks(Arc::clone(&hooks))
        .with_compactor(compactor)
        .with_todos(Arc::clone(&todos))
        .with_stream(delta_tx)
        .with_cancel(cancel);
    let started = Instant::now();
    let outcome = agent
        .run_with(
            &connection.model,
            messages,
            RunContext::new(&session_id, Some(&artifact_dir), &request.message),
        )
        .await?;
    drop(agent);
    let _ = emitter.await;
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
    if let Ok(items_json) = serde_json::to_string(&outcome.todos) {
        store.save_todos(&session_id, &items_json)?;
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

    if let Some(guard) = session_guard.as_mut() {
        guard.armed = false;
    }

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
        todos: outcome.todos,
        compactions: outcome.compactions,
        permission_denials: outcome.permission_denials,
        stop_reason: match outcome.stop_reason {
            StopReason::Completed => "completed",
            StopReason::MaxSteps => "max_steps",
            StopReason::Stopped => "stopped",
        }
        .to_string(),
    })
}

#[tauri::command]
fn respond_approval(
    broker: State<'_, Arc<ApprovalBroker>>,
    request: RespondApprovalRequest,
) -> bool {
    broker.respond(&request.id, request.approved)
}

fn harness_info(dir: &Path) -> HarnessInfo {
    let config = harness::load(dir);
    let memory = MemoryStore::new(dir.join("memory"))
        .records()
        .into_iter()
        .map(|record| MemoryInfo {
            name: record.name,
            description: record.description,
            kind: record.kind,
            body: record.body,
            path: record.path.display().to_string(),
        })
        .collect();
    HarnessInfo {
        config,
        path: harness::config_path(dir).display().to_string(),
        memory_dir: dir.join("memory").display().to_string(),
        audit_log: dir.join("audit.jsonl").display().to_string(),
        memory,
    }
}

#[tauri::command]
fn get_harness() -> Result<HarnessInfo, String> {
    let dir = data_dir().map_err(friendly)?;
    Ok(harness_info(&dir))
}

#[tauri::command]
fn save_harness(config: HarnessConfig) -> Result<HarnessInfo, String> {
    let dir = data_dir().map_err(friendly)?;
    harness::save(&dir, &config)?;
    Ok(harness_info(&dir))
}

#[tauri::command]
fn delete_memory(request: MemoryNameRequest) -> Result<bool, String> {
    let dir = data_dir().map_err(friendly)?;
    MemoryStore::new(dir.join("memory")).forget(&request.name)
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
        .manage(Arc::new(ApprovalBroker::default()))
        .manage(Arc::new(StopBroker::default()))
        .invoke_handler(tauri::generate_handler![
            app_status,
            chat,
            stop_chat,
            respond_approval,
            get_harness,
            save_harness,
            delete_memory,
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
            draft_extension,
            save_extension,
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
