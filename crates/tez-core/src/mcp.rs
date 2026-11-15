//! MCP client tối giản qua stdio hoặc Streamable HTTP (JSON-RPC 2.0).
//!
//! Hỗ trợ `initialize`, `tools/list`, `tools/call`. Server stdio là process con
//! được giữ sống trong lúc agent chạy; server HTTP dùng POST JSON-RPC và nhận
//! response JSON hoặc SSE. Tool MCP được đăng ký vào cùng
//! [`crate::tools::ToolRegistry`] với tool built-in.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tez_llm::ToolSpec;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

use crate::oauth::{self, OAuthConfig};
use crate::tools::{Tool, ToolError, ToolOutput};

const MCP_TIMEOUT: Duration = Duration::from_secs(20);
/// Một dòng JSON-RPC quá lớn có thể làm cạn RAM; chặn trước khi cấp phát.
const MAX_MCP_LINE_BYTES: usize = 4 * 1024 * 1024;
/// Kết quả tool MCP đưa vào context cũng cần có trần.
const MAX_MCP_OUTPUT_BYTES: usize = 64 * 1024;

#[derive(Debug, Error)]
pub enum McpError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("không spawn được MCP server: {0}")]
    Spawn(String),
    #[error("MCP server đóng kết nối")]
    Closed,
    #[error("MCP timeout")]
    Timeout,
    #[error("MCP HTTP: {0}")]
    Http(String),
    #[error("MCP OAuth: {0}")]
    Auth(String),
    #[error("MCP error: {0}")]
    Rpc(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Bỏ trống khi dùng `url` (Streamable HTTP).
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Chỉ có hiệu lực khi server đến từ cấu hình của user. Server tự khai báo
    /// `readOnlyHint` trong `tools/list` không được tin.
    #[serde(default, alias = "readOnly")]
    pub read_only: bool,
    /// Endpoint Streamable HTTP. Có `url` thì bỏ qua `command`/`args`.
    #[serde(default)]
    pub url: Option<String>,
    /// Header bổ sung cho HTTP, ví dụ Authorization. Không ghi log.
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Tên biến môi trường chứa bearer token; đọc lúc kết nối, không lưu token
    /// trong `mcp.json`.
    #[serde(default, alias = "bearerTokenEnv")]
    pub bearer_token_env: Option<String>,
    /// OAuth 2.1 + PKCE cho MCP HTTP. Token lưu ngoài `mcp.json`.
    #[serde(default)]
    pub oauth: Option<OAuthConfig>,
    /// Sandbox cho MCP stdio trên Linux. Mặc định bật trên Linux để server
    /// không đọc được secret trong data dir; server cần cache/network thì khai
    /// báo `writable_paths`/`allow_network`, hoặc tắt rõ ràng bằng
    /// `"enabled": false`. Trên macOS/Windows chưa có sandbox OS nên mặc định tắt.
    #[serde(default)]
    pub sandbox: McpSandboxSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpSandboxSettings {
    #[serde(default = "default_sandbox_enabled")]
    pub enabled: bool,
    /// MCP server thường cần mạng để tải package; mặc định chặn.
    #[serde(default)]
    pub allow_network: bool,
    /// Các thư mục bind read-write thêm, ví dụ `~/.npm`.
    #[serde(default)]
    pub writable_paths: Vec<String>,
}

fn default_sandbox_enabled() -> bool {
    cfg!(target_os = "linux")
}

impl Default for McpSandboxSettings {
    fn default() -> Self {
        Self {
            enabled: default_sandbox_enabled(),
            allow_network: false,
            writable_paths: Vec::new(),
        }
    }
}

fn default_true() -> bool {
    true
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpConfig {
    #[serde(default)]
    pub servers: HashMap<String, McpServerConfig>,
    /// Bật rõ ràng cho `.tez/mcp.json` do repo kiểm soát. Mặc định tắt.
    #[serde(default, skip_serializing_if = "is_false")]
    pub trusted: bool,
}

#[derive(Debug, Clone)]
pub struct ServerEntry {
    pub name: String,
    pub config: McpServerConfig,
    pub source: String,
}

#[derive(Debug, Clone)]
pub struct McpToolInfo {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    /// Hint do server khai báo. Chỉ dùng để hiển thị; không dùng cho permission.
    pub read_only_hint: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpServerStatus {
    pub name: String,
    pub source: String,
    pub ok: bool,
    pub tools: Vec<String>,
    pub error: Option<String>,
    pub oauth: bool,
    pub authorized: bool,
}

pub struct McpDiscovery {
    pub tools: Vec<McpTool>,
    /// Tool đọc resources/prompts của server (một tool/server, luôn read-only).
    pub extras: Vec<McpExtrasTool>,
    pub servers: Vec<McpServerStatus>,
    /// Tên tool MCP được user cấu hình `read_only`; dùng để miễn hỏi duyệt.
    pub read_only_tools: Vec<String>,
}

pub fn config_path(dir: impl AsRef<Path>) -> PathBuf {
    dir.as_ref().join("mcp.json")
}

/// File MCP riêng của dự án: `<root>/.tez/mcp.json`.
pub fn project_config_path(root: impl AsRef<Path>) -> PathBuf {
    root.as_ref().join(".tez").join("mcp.json")
}

pub fn load_config(dir: impl AsRef<Path>) -> Result<McpConfig, McpError> {
    let path = config_path(dir);
    if !path.exists() {
        return Ok(McpConfig::default());
    }
    let bytes = std::fs::read(path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

pub fn load_project_config(root: impl AsRef<Path>) -> Result<McpConfig, McpError> {
    let path = project_config_path(root);
    if !path.exists() {
        return Ok(McpConfig::default());
    }
    let bytes = std::fs::read(path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Nạp cấu hình nhưng không bao giờ làm hỏng phiên chat.
///
/// `mcp.json` toàn cục và `.tez/mcp.json` của dự án đều có thể hỏng, quá lớn
/// hoặc bị sửa bởi người khác. Khi đó chỉ cảnh báo và bỏ qua file, không trả
/// lỗi cho caller. Cấu hình sai vẫn hiện lỗi rõ trong màn hình Settings vì
/// phần đó dùng [`load_config`] nghiêm ngặt.
fn read_config_lenient(path: &Path) -> McpConfig {
    if !path.exists() {
        return McpConfig::default();
    }
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!(
                "[mcp] bỏ qua '{}': không đọc được file ({error})",
                path.display()
            );
            return McpConfig::default();
        }
    };
    match serde_json::from_slice(&bytes) {
        Ok(config) => config,
        Err(error) => {
            eprintln!(
                "[mcp] bỏ qua '{}': JSON không hợp lệ ({error})",
                path.display()
            );
            McpConfig::default()
        }
    }
}

fn load_config_lenient(dir: impl AsRef<Path>) -> McpConfig {
    read_config_lenient(&config_path(dir))
}

fn load_project_config_lenient(root: impl AsRef<Path>) -> McpConfig {
    read_config_lenient(&project_config_path(root))
}

fn env_truthy(name: &str) -> bool {
    std::env::var(name)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

/// Repo chỉ được tự chạy MCP server khi người dùng bật rõ ràng.
pub fn project_mcp_trusted(config: &McpConfig) -> bool {
    config.trusted || env_truthy("TEZ_TRUST_PROJECT_MCP")
}

/// `read_only` chỉ đáng tin khi đến từ cấu hình user, không phải từ server
/// tự khai báo hay từ repo/plugin.
fn trusted_read_only(source: &str, config: &McpServerConfig) -> bool {
    source == "user" && config.read_only
}

/// Ghi `mcp.json` atomically.
///
/// Ghi ra file tạm cùng thư mục rồi rename để lần đọc đồng thời không thấy
/// file dở dang. Nếu file cũ không parse được, giữ một bản backup trước khi
/// ghi đè để người dùng còn dữ liệu khôi phục.
pub fn save_config(dir: impl AsRef<Path>, config: &McpConfig) -> Result<(), McpError> {
    let path = config_path(dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    backup_if_corrupt(&path);

    let bytes = serde_json::to_vec_pretty(config)?;
    let temp = path.with_extension(format!("json.tmp.{}", std::process::id()));
    let result = (|| -> Result<(), McpError> {
        {
            let mut options = std::fs::OpenOptions::new();
            options.create(true).truncate(true).write(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temp)?;
            use std::io::Write;
            file.write_all(&bytes)?;
            file.sync_all()?;
        }
        restrict_permissions(&temp)?;
        std::fs::rename(&temp, &path)?;
        restrict_permissions(&path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

/// Đổi tên file hỏng thành `mcp.json.corrupt-<timestamp>-<pid>`.
fn backup_if_corrupt(path: &Path) {
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    if serde_json::from_slice::<McpConfig>(&bytes).is_ok() {
        return;
    }
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let backup = path.with_extension(format!("json.corrupt-{timestamp}-{}", std::process::id()));
    match std::fs::rename(path, &backup) {
        Ok(()) => eprintln!(
            "[mcp] file cấu hình hỏng đã được sao lưu tại '{}'",
            backup.display()
        ),
        Err(error) => eprintln!(
            "[mcp] không sao lưu được file cấu hình hỏng '{}': {error}",
            path.display()
        ),
    }
}

/// Gộp server từ `mcp.json` toàn cục và từ plugin. Server user định nghĩa được ưu tiên.
pub fn load_servers(dir: impl AsRef<Path>) -> Result<Vec<ServerEntry>, McpError> {
    load_servers_with_project(dir, None)
}

/// Gộp server toàn cục, server của dự án và server từ plugin.
///
/// Thứ tự ưu tiên khi trùng tên: dự án > user > plugin.
pub fn load_servers_with_project(
    dir: impl AsRef<Path>,
    project_root: Option<&Path>,
) -> Result<Vec<ServerEntry>, McpError> {
    let dir = dir.as_ref();
    let config = load_config_lenient(dir);
    let mut entries: Vec<ServerEntry> = config
        .servers
        .into_iter()
        .map(|(name, config)| ServerEntry {
            name,
            config,
            source: "user".to_string(),
        })
        .collect();

    if let Some(root) = project_root {
        let project = load_project_config_lenient(root);
        if !project.servers.is_empty() && !project_mcp_trusted(&project) {
            eprintln!(
                "[mcp] bỏ qua MCP server của dự án '{}': chưa được tin cậy. \
                 Thêm \"trusted\": true vào .tez/mcp.json hoặc đặt TEZ_TRUST_PROJECT_MCP=1.",
                root.display()
            );
        } else {
            for (name, mut config) in project.servers {
                if config.cwd.is_none() {
                    config.cwd = Some(root.to_string_lossy().to_string());
                }
                let entry = ServerEntry {
                    name: name.clone(),
                    config,
                    source: "project".to_string(),
                };
                if let Some(existing) = entries.iter_mut().find(|item| item.name == name) {
                    *existing = entry;
                } else {
                    entries.push(entry);
                }
            }
        }
    }

    let plugins =
        crate::plugins::load_all(dir).map_err(|error| McpError::Spawn(error.to_string()))?;
    for plugin in plugins {
        for (name, mut config) in plugin.manifest.mcp_servers {
            if entries.iter().any(|entry| entry.name == name) {
                continue;
            }
            if config.cwd.is_none() {
                config.cwd = Some(plugin.dir.to_string_lossy().to_string());
            }
            entries.push(ServerEntry {
                name,
                config,
                source: plugin.manifest.id.clone(),
            });
        }
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

/// Spawn các server đang bật, lấy danh sách tool và giữ process sống.
pub async fn discover(dir: impl AsRef<Path>) -> Result<McpDiscovery, McpError> {
    discover_with_project(dir, None).await
}

/// Như [`discover`] nhưng gộp thêm MCP server của dự án.
pub async fn discover_with_project(
    dir: impl AsRef<Path>,
    project_root: Option<&Path>,
) -> Result<McpDiscovery, McpError> {
    let dir = dir.as_ref().to_path_buf();
    let entries = load_servers_with_project(&dir, project_root)?;
    let mut tools = Vec::new();
    let mut extras = Vec::new();
    let mut servers = Vec::new();
    let mut read_only_tools = Vec::new();

    for entry in entries {
        let has_oauth = entry.config.oauth.is_some();
        let authorized = entry
            .config
            .oauth
            .as_ref()
            .is_some_and(|config| oauth::status(&dir, &entry.name, config).authorized);
        if !entry.config.enabled {
            servers.push(McpServerStatus {
                name: entry.name,
                source: entry.source,
                ok: false,
                tools: Vec::new(),
                error: Some("đang tắt".to_string()),
                oauth: has_oauth,
                authorized,
            });
            continue;
        }

        let read_only = trusted_read_only(&entry.source, &entry.config);
        match McpClient::connect(&entry.name, &entry.config, Some(&dir)).await {
            Ok((client, infos)) => {
                let tool_names = infos.iter().map(|info| info.name.clone()).collect();
                for info in infos {
                    let tool =
                        McpTool::new(entry.name.clone(), info, read_only, Arc::clone(&client));
                    if read_only {
                        read_only_tools.push(tool.spec().name);
                    }
                    tools.push(tool);
                }
                if client.supports("resources") || client.supports("prompts") {
                    let extra = McpExtrasTool::new(entry.name.clone(), Arc::clone(&client));
                    read_only_tools.push(extra.spec().name);
                    extras.push(extra);
                }
                servers.push(McpServerStatus {
                    name: entry.name,
                    source: entry.source,
                    ok: true,
                    tools: tool_names,
                    error: None,
                    oauth: has_oauth,
                    authorized,
                });
            }
            Err(error) => servers.push(McpServerStatus {
                name: entry.name,
                source: entry.source,
                ok: false,
                tools: Vec::new(),
                error: Some(error.to_string()),
                oauth: has_oauth,
                authorized,
            }),
        }
    }

    Ok(McpDiscovery {
        tools,
        extras,
        servers,
        read_only_tools,
    })
}

pub struct McpTool {
    server: String,
    raw_name: String,
    tool_name: String,
    description: String,
    parameters: Value,
    read_only: bool,
    client: Arc<McpClient>,
}

impl McpTool {
    fn new(server: String, info: McpToolInfo, read_only: bool, client: Arc<McpClient>) -> Self {
        let tool_name = sanitize_tool_name(&server, &info.name);
        let description = format!("[MCP: {}] {}", server, info.description.trim());
        Self {
            server,
            raw_name: info.name,
            tool_name,
            description,
            parameters: info.input_schema,
            read_only,
            client,
        }
    }

    pub fn server(&self) -> &str {
        &self.server
    }

    pub fn raw_name(&self) -> &str {
        &self.raw_name
    }

    /// Chỉ `true` khi user cấu hình `read_only`; hint của server bị bỏ qua.
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }
}

#[async_trait]
impl Tool for McpTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.tool_name.clone(),
            description: self.description.clone(),
            parameters: self.parameters.clone(),
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        self.client
            .call(&self.raw_name, args)
            .await
            .map_err(|error| ToolError::Execution(error.to_string()))
    }
}

/// Tool tổng hợp để đọc resources/prompts của một MCP server.
pub struct McpExtrasTool {
    server: String,
    tool_name: String,
    description: String,
    client: Arc<McpClient>,
}

impl McpExtrasTool {
    fn new(server: String, client: Arc<McpClient>) -> Self {
        let tool_name = sanitize_tool_name(&server, "extras");
        let description = format!(
            "[MCP: {server}] Đọc resources và prompts của server. \
             Dùng action=list_resources để liệt kê resource, read_resource với uri để đọc, \
             list_prompts để liệt kê prompt, get_prompt với name/arguments để lấy nội dung."
        );
        Self {
            server,
            tool_name,
            description,
            client,
        }
    }

    pub fn server(&self) -> &str {
        &self.server
    }

    pub fn is_read_only(&self) -> bool {
        true
    }
}

#[async_trait]
impl Tool for McpExtrasTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.tool_name.clone(),
            description: self.description.clone(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["list_resources", "read_resource", "list_prompts", "get_prompt"],
                        "description": "Hành động cần thực hiện"
                    },
                    "uri": {
                        "type": "string",
                        "description": "URI resource cho read_resource"
                    },
                    "name": {
                        "type": "string",
                        "description": "Tên prompt cho get_prompt"
                    },
                    "arguments": {
                        "type": "object",
                        "description": "Tham số prompt cho get_prompt",
                        "additionalProperties": true
                    }
                },
                "required": ["action"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArgs("thiếu trường 'action'".into()))?;
        self.client
            .extras(action, &args)
            .await
            .map_err(|error| ToolError::Execution(error.to_string()))
    }
}

#[async_trait]
trait McpTransport: Send {
    fn protocol_version(&self) -> &'static str;
    async fn request(&mut self, method: &str, params: Value) -> Result<Value, McpError>;
    async fn notify(&mut self, method: &str, params: Value) -> Result<(), McpError>;
}

pub struct McpClient {
    inner: Mutex<Box<dyn McpTransport>>,
    capabilities: Value,
}

impl McpClient {
    pub async fn connect(
        server: &str,
        config: &McpServerConfig,
        token_dir: Option<&Path>,
    ) -> Result<(Arc<Self>, Vec<McpToolInfo>), McpError> {
        let use_http = config
            .url
            .as_deref()
            .map(str::trim)
            .is_some_and(|url| !url.is_empty());
        let mut transport: Box<dyn McpTransport> = if use_http {
            Box::new(HttpConnection::connect(config, server, token_dir).await?)
        } else {
            Box::new(StdioConnection::connect(config, token_dir)?)
        };

        let protocol = transport.protocol_version();
        let initialize = transport
            .request(
                "initialize",
                json!({
                    "protocolVersion": protocol,
                    "capabilities": {},
                    "clientInfo": {
                        "name": "tez",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }),
            )
            .await?;
        let capabilities = initialize
            .get("capabilities")
            .cloned()
            .unwrap_or(Value::Null);
        transport
            .notify("notifications/initialized", json!({}))
            .await?;
        let result = transport.request("tools/list", json!({})).await?;
        let tools = parse_tools(&result);

        Ok((
            Arc::new(Self {
                inner: Mutex::new(transport),
                capabilities,
            }),
            tools,
        ))
    }

    /// Server khai báo capability, ví dụ `resources` hoặc `prompts`.
    pub fn supports(&self, capability: &str) -> bool {
        self.capabilities.get(capability).is_some()
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, McpError> {
        let mut connection = self.inner.lock().await;
        connection.request(method, params).await
    }

    pub async fn call(&self, tool: &str, args: Value) -> Result<ToolOutput, McpError> {
        let result = self
            .request(
                "tools/call",
                json!({
                    "name": tool,
                    "arguments": args
                }),
            )
            .await?;
        Ok(parse_call_result(&result))
    }

    /// Đọc resources/prompts qua một tool tổng hợp. Luôn chỉ đọc.
    pub async fn extras(&self, action: &str, args: &Value) -> Result<ToolOutput, McpError> {
        let result = match action {
            "list_resources" => self.request("resources/list", json!({})).await?,
            "read_resource" => {
                let uri = args
                    .get("uri")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| McpError::Rpc("thiếu 'uri' cho read_resource".into()))?;
                self.request("resources/read", json!({ "uri": uri }))
                    .await?
            }
            "list_prompts" => self.request("prompts/list", json!({})).await?,
            "get_prompt" => {
                let name = args
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| McpError::Rpc("thiếu 'name' cho get_prompt".into()))?;
                let arguments = args
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                self.request(
                    "prompts/get",
                    json!({ "name": name, "arguments": arguments }),
                )
                .await?
            }
            other => {
                return Err(McpError::Rpc(format!(
                    "action không hỗ trợ: {other} (dùng list_resources, read_resource, list_prompts, get_prompt)"
                )))
            }
        };

        Ok(format_extra_result(action, &result))
    }
}

/// Chỉ chuyển các biến môi trường cần thiết sang MCP server stdio.
fn inherit_safe_env(command: &mut Command) {
    command.env_clear();
    #[cfg(windows)]
    let keys = [
        "PATH",
        "PATHEXT",
        "SYSTEMROOT",
        "COMSPEC",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "TEMP",
        "TMP",
        "HOME",
        "USERNAME",
        "LANG",
    ];
    #[cfg(not(windows))]
    let keys = [
        "PATH", "HOME", "USER", "LOGNAME", "SHELL", "LANG", "LC_ALL", "TMPDIR",
    ];
    for key in keys {
        if let Ok(value) = std::env::var(key) {
            command.env(key, value);
        }
    }
}

/// Mở rộng `~` và đường dẫn tương đối cho `writable_paths` của sandbox MCP.
fn expand_user_path(value: &str, cwd: &Path) -> PathBuf {
    if value == "~" {
        return std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| cwd.to_path_buf());
    }
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

struct StdioConnection {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl StdioConnection {
    fn connect(config: &McpServerConfig, data_dir: Option<&Path>) -> Result<Self, McpError> {
        if config.command.trim().is_empty() {
            return Err(McpError::Spawn(
                "thiếu command và url cho MCP server".into(),
            ));
        }
        let mut command = if config.sandbox.enabled {
            let data_dir = data_dir.ok_or_else(|| {
                McpError::Spawn("sandbox MCP cần data dir của app để ẩn secret".into())
            })?;
            let cwd = config.cwd.as_deref().map(PathBuf::from).unwrap_or(
                std::env::current_dir().map_err(|error| {
                    McpError::Spawn(format!("không lấy được cwd hiện tại: {error}"))
                })?,
            );
            let cwd = if cwd.is_absolute() {
                cwd
            } else {
                std::env::current_dir()
                    .map_err(|error| {
                        McpError::Spawn(format!("không lấy được cwd hiện tại: {error}"))
                    })?
                    .join(cwd)
            };
            let writable: Vec<PathBuf> = config
                .sandbox
                .writable_paths
                .iter()
                .filter(|value| !value.trim().is_empty())
                .map(|value| expand_user_path(value.trim(), &cwd))
                .collect();
            let policy = crate::sandbox::SandboxPolicy {
                enabled: true,
                allow_network: config.sandbox.allow_network,
                data_dir: data_dir.to_path_buf(),
            };
            let mut command = crate::sandbox::build_mcp_command(
                &policy,
                &cwd,
                &writable,
                &config.command,
                &config.args,
            )
            .map_err(McpError::Spawn)?;
            command.envs(&config.env);
            command
        } else {
            let mut command = Command::new(&config.command);
            // Không để server kế thừa toàn bộ env của app (API key, token...).
            // Chỉ giữ các biến cần để chạy process; server cần gì thêm thì khai
            // báo trong `env` của `mcp.json`.
            inherit_safe_env(&mut command);
            command.args(&config.args).envs(&config.env);
            if let Some(cwd) = &config.cwd {
                command.current_dir(cwd);
            }
            command
        };
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);

        let mut child = command
            .spawn()
            .map_err(|error| McpError::Spawn(format!("{}: {error}", config.command)))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpError::Spawn("không mở được stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpError::Spawn("không mở được stdout".into()))?;

        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        })
    }
}

#[async_trait]
impl McpTransport for StdioConnection {
    fn protocol_version(&self) -> &'static str {
        "2024-11-05"
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value, McpError> {
        self.send_request(method, params).await
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<(), McpError> {
        self.send_notify(method, params).await
    }
}

impl Drop for StdioConnection {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

impl StdioConnection {
    async fn send_request(&mut self, method: &str, params: Value) -> Result<Value, McpError> {
        let id = self.next_id;
        self.next_id += 1;
        self.write(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        }))
        .await?;
        self.read_result(id).await
    }

    async fn send_notify(&mut self, method: &str, params: Value) -> Result<(), McpError> {
        self.write(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        }))
        .await
    }

    async fn write(&mut self, value: &Value) -> Result<(), McpError> {
        let mut line = serde_json::to_vec(value)?;
        line.push(b'\n');
        self.stdin.write_all(&line).await?;
        self.stdin.flush().await?;
        Ok(())
    }

    /// Đọc một dòng có giới hạn, không dùng `read_line` vô hạn.
    async fn read_line_capped(&mut self, max: usize) -> Result<Option<String>, McpError> {
        let mut bytes = Vec::new();
        loop {
            let available = self.stdout.fill_buf().await?;
            if available.is_empty() {
                if bytes.is_empty() {
                    return Ok(None);
                }
                return Ok(Some(String::from_utf8_lossy(&bytes).into_owned()));
            }
            if let Some(position) = available.iter().position(|&byte| byte == b'\n') {
                bytes.extend_from_slice(&available[..=position]);
                self.stdout.consume(position + 1);
                return Ok(Some(String::from_utf8_lossy(&bytes).into_owned()));
            }
            bytes.extend_from_slice(available);
            let consumed = available.len();
            self.stdout.consume(consumed);
            if bytes.len() > max {
                return Err(McpError::Rpc(format!(
                    "dòng JSON từ MCP server vượt quá {} KB",
                    max / 1024
                )));
            }
        }
    }

    async fn read_result(&mut self, id: u64) -> Result<Value, McpError> {
        let deadline = Instant::now() + MCP_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(McpError::Timeout);
            }

            let line =
                match tokio::time::timeout(remaining, self.read_line_capped(MAX_MCP_LINE_BYTES))
                    .await
                {
                    Ok(result) => result?,
                    Err(_) => return Err(McpError::Timeout),
                };
            let Some(line) = line else {
                return Err(McpError::Closed);
            };

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let value: Value = match serde_json::from_str(trimmed) {
                Ok(value) => value,
                Err(_) => continue,
            };
            if value.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = value.get("error") {
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                return Err(McpError::Rpc(message.to_string()));
            }
            return Ok(value.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}

/// Thay `${VAR}` trong header bằng biến môi trường. Thiếu biến là lỗi cấu hình,
/// không gửi nguyên placeholder lên server.
fn expand_env_value(value: &str) -> Result<String, McpError> {
    let mut output = String::new();
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        output.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            output.push_str("${");
            rest = after;
            continue;
        };
        let name = &after[..end];
        if name.is_empty()
            || !name
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
        {
            output.push_str("${");
            rest = after;
            continue;
        }
        let value = std::env::var(name)
            .map_err(|_| McpError::Spawn(format!("thiếu biến môi trường {name} cho MCP header")))?;
        output.push_str(&value);
        rest = &after[end + 1..];
    }
    output.push_str(rest);
    Ok(output)
}

/// Header HTTP sau khi mở rộng `${VAR}` và thêm bearer token từ biến môi trường.
fn resolve_http_headers(config: &McpServerConfig) -> Result<HashMap<String, String>, McpError> {
    let mut headers = HashMap::new();
    for (name, value) in &config.headers {
        headers.insert(name.clone(), expand_env_value(value)?);
    }
    if let Some(variable) = config
        .bearer_token_env
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let token = std::env::var(variable).map_err(|_| {
            McpError::Spawn(format!(
                "thiếu biến môi trường {variable} cho bearer token MCP"
            ))
        })?;
        let token = token.trim();
        if token.is_empty() {
            return Err(McpError::Spawn(format!(
                "biến môi trường {variable} cho bearer token MCP đang rỗng"
            )));
        }
        headers.insert("Authorization".to_string(), format!("Bearer {token}"));
    }
    Ok(headers)
}

struct HttpConnection {
    client: reqwest::Client,
    url: String,
    headers: HashMap<String, String>,
    session_id: Option<String>,
    next_id: u64,
    oauth: Option<OAuthConfig>,
    server: String,
    token_dir: Option<PathBuf>,
    get_stream: Option<tokio::task::JoinHandle<()>>,
}

impl HttpConnection {
    async fn connect(
        config: &McpServerConfig,
        server: &str,
        token_dir: Option<&Path>,
    ) -> Result<Self, McpError> {
        let raw = config
            .url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .ok_or_else(|| McpError::Spawn("thiếu url cho MCP HTTP".into()))?;
        let url = reqwest::Url::parse(raw)
            .map_err(|error| McpError::Spawn(format!("URL MCP không hợp lệ: {error}")))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(McpError::Spawn(
                "MCP HTTP chỉ hỗ trợ http/https".to_string(),
            ));
        }
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(8))
            .timeout(MCP_TIMEOUT)
            .user_agent("tez/0.1 (+local)")
            .build()
            .map_err(|error| McpError::Http(format!("không tạo được HTTP client: {error}")))?;
        let mut headers = resolve_http_headers(config)?;
        if let Some(oauth_config) = &config.oauth {
            let dir = token_dir.ok_or_else(|| {
                McpError::Auth(format!(
                    "server '{server}' dùng OAuth nhưng thiếu thư mục token"
                ))
            })?;
            let token = oauth::access_token(dir, server, oauth_config)
                .await
                .map_err(|error| McpError::Auth(error.to_string()))?;
            headers.insert(
                "Authorization".to_string(),
                format!("Bearer {}", token.trim()),
            );
        }
        Ok(Self {
            client,
            url: raw.to_string(),
            headers,
            session_id: None,
            next_id: 1,
            oauth: config.oauth.clone(),
            server: server.to_string(),
            token_dir: token_dir.map(Path::to_path_buf),
            get_stream: None,
        })
    }

    /// Mở GET SSE stream để nhận request/notification do server khởi tạo.
    /// Stream là best-effort: server không hỗ trợ, trả lỗi hoặc đóng kết nối
    /// đều không làm hỏng các request POST hiện có.
    fn start_get_stream(&mut self) {
        if self.get_stream.is_some() {
            return;
        }
        let client = self.client.clone();
        let url = self.url.clone();
        let headers = self.headers.clone();
        let session_id = self.session_id.clone();
        self.get_stream = Some(tokio::spawn(async move {
            run_get_stream(client, url, headers, session_id).await;
        }));
    }

    async fn send(&mut self, payload: &Value) -> Result<reqwest::Response, McpError> {
        let mut request = self
            .client
            .post(&self.url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(
                reqwest::header::ACCEPT,
                "application/json, text/event-stream",
            )
            .json(payload);
        for (name, value) in &self.headers {
            request = request.header(name, value);
        }
        if let Some(session) = &self.session_id {
            request = request.header("Mcp-Session-Id", session);
        }
        let response = request
            .send()
            .await
            .map_err(|error| McpError::Http(error.to_string()))?;
        if let Some(session) = response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
        {
            self.session_id = Some(session.to_string());
        }
        Ok(response)
    }

    async fn post(&mut self, payload: &Value) -> Result<reqwest::Response, McpError> {
        let mut response = self.send(payload).await?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            if let (Some(config), Some(dir)) = (self.oauth.clone(), self.token_dir.clone()) {
                if let Some(previous) = oauth::load_token(&dir, &self.server, &config) {
                    let refreshed = oauth::refresh_token(&dir, &self.server, &config, &previous)
                        .await
                        .map_err(|error| McpError::Auth(error.to_string()))?;
                    self.headers.insert(
                        "Authorization".to_string(),
                        format!("Bearer {}", refreshed.access_token.trim()),
                    );
                    response = self.send(payload).await?;
                }
            }
        }
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let preview: String = body.chars().take(500).collect();
            return Err(McpError::Rpc(format!("HTTP {status}: {preview}")));
        }
        Ok(response)
    }
}

#[async_trait]
impl McpTransport for HttpConnection {
    fn protocol_version(&self) -> &'static str {
        "2025-03-26"
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value, McpError> {
        let id = self.next_id;
        self.next_id += 1;
        let response = self
            .post(&json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": params
            }))
            .await?;
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        let bytes = response
            .bytes()
            .await
            .map_err(|error| McpError::Http(error.to_string()))?;
        if bytes.len() > MAX_MCP_LINE_BYTES {
            return Err(McpError::Rpc(format!(
                "response MCP vượt quá {} KB",
                MAX_MCP_LINE_BYTES / 1024
            )));
        }
        if content_type.contains("text/event-stream") {
            parse_sse_result(&bytes, id)
        } else {
            parse_json_result(&bytes, id)
        }
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<(), McpError> {
        let response = self
            .post(&json!({
                "jsonrpc": "2.0",
                "method": method,
                "params": params
            }))
            .await?;
        let _ = response.bytes().await;
        if method == "notifications/initialized" {
            self.start_get_stream();
        }
        Ok(())
    }
}

impl Drop for HttpConnection {
    fn drop(&mut self) {
        if let Some(task) = &self.get_stream {
            task.abort();
        }
    }
}

/// Đọc GET SSE stream và trả lời các request server khởi tạo.
async fn run_get_stream(
    client: reqwest::Client,
    url: String,
    headers: HashMap<String, String>,
    session_id: Option<String>,
) {
    let mut request = client
        .get(&url)
        .header(reqwest::header::ACCEPT, "text/event-stream")
        .header(reqwest::header::CACHE_CONTROL, "no-cache");
    for (name, value) in &headers {
        request = request.header(name, value);
    }
    if let Some(session) = &session_id {
        request = request.header("Mcp-Session-Id", session);
    }
    let response = match request.send().await {
        Ok(response) => response,
        Err(_) => return,
    };
    if !response.status().is_success() {
        return;
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !content_type.contains("text/event-stream") {
        return;
    }

    let mut stream = response.bytes_stream();
    let mut buffer: Vec<u8> = Vec::new();
    let mut data = String::new();
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else { break };
        buffer.extend_from_slice(&chunk);
        while let Some(position) = buffer.iter().position(|byte| *byte == b'\n') {
            let line_bytes: Vec<u8> = buffer.drain(..=position).collect();
            let line = String::from_utf8_lossy(&line_bytes);
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                if !data.is_empty() {
                    if let Ok(value) = serde_json::from_str::<Value>(data.trim_end()) {
                        handle_server_message(&client, &url, &headers, &session_id, value).await;
                    }
                    data.clear();
                }
                continue;
            }
            if let Some(rest) = line.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
                if data.len() > MAX_MCP_LINE_BYTES {
                    data.clear();
                }
            }
        }
        if buffer.len() > MAX_MCP_LINE_BYTES {
            buffer.clear();
        }
    }
}

/// Trả lời request do server khởi tạo. Client tez không khai báo sampling,
/// elicitation hay roots, nên chỉ hỗ trợ `ping` và trả lỗi rõ cho phần còn lại.
async fn handle_server_message(
    client: &reqwest::Client,
    url: &str,
    headers: &HashMap<String, String>,
    session_id: &Option<String>,
    value: Value,
) {
    let Some(method) = value.get("method").and_then(Value::as_str) else {
        return;
    };
    let Some(id) = value.get("id").cloned() else {
        return;
    };
    let payload = match method {
        "ping" => json!({"jsonrpc": "2.0", "id": id, "result": {}}),
        "roots/list" => json!({"jsonrpc": "2.0", "id": id, "result": {"roots": []}}),
        other => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32601,
                "message": format!("tez không hỗ trợ request '{other}' từ MCP server")
            }
        }),
    };
    let mut request = client
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(
            reqwest::header::ACCEPT,
            "application/json, text/event-stream",
        )
        .json(&payload);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    if let Some(session) = session_id {
        request = request.header("Mcp-Session-Id", session);
    }
    let _ = request.send().await;
}

fn parse_json_result(bytes: &[u8], id: u64) -> Result<Value, McpError> {
    let value: Value = serde_json::from_slice(bytes)?;
    rpc_result(&value, id)
}

fn parse_sse_result(bytes: &[u8], id: u64) -> Result<Value, McpError> {
    let text = String::from_utf8_lossy(bytes);
    let mut data_lines: Vec<String> = Vec::new();
    for line in text.lines().chain(std::iter::once("")) {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            if data_lines.is_empty() {
                continue;
            }
            let payload = data_lines.join("\n");
            data_lines.clear();
            let Ok(value) = serde_json::from_str::<Value>(&payload) else {
                continue;
            };
            if value.get("id").and_then(Value::as_u64) == Some(id) {
                return rpc_result(&value, id);
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.trim_start().to_string());
        }
    }
    Err(McpError::Rpc(format!("SSE không có response khớp id {id}")))
}

fn rpc_result(value: &Value, id: u64) -> Result<Value, McpError> {
    if value.get("id").and_then(Value::as_u64) != Some(id) {
        return Err(McpError::Rpc(format!("response MCP không khớp id {id}")));
    }
    if let Some(error) = value.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        return Err(McpError::Rpc(message.to_string()));
    }
    Ok(value.get("result").cloned().unwrap_or(Value::Null))
}

fn parse_tools(result: &Value) -> Vec<McpToolInfo> {
    result
        .get("tools")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let name = item.get("name")?.as_str()?.to_string();
                    let description = item
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let input_schema = item
                        .get("inputSchema")
                        .cloned()
                        .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
                    let read_only_hint = item
                        .get("annotations")
                        .and_then(|value| value.get("readOnlyHint"))
                        .and_then(Value::as_bool)
                        .or_else(|| item.get("readOnlyHint").and_then(Value::as_bool))
                        .unwrap_or(false);
                    Some(McpToolInfo {
                        name,
                        description,
                        input_schema,
                        read_only_hint,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn cap_output(mut content: String) -> String {
    if content.len() > MAX_MCP_OUTPUT_BYTES {
        let mut end = MAX_MCP_OUTPUT_BYTES;
        while !content.is_char_boundary(end) {
            end -= 1;
        }
        content.truncate(end);
        content.push_str("\n… (output MCP đã bị cắt bớt)");
    }
    content
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn format_extra_result(action: &str, result: &Value) -> ToolOutput {
    let content = match action {
        "read_resource" => format_resource_contents(result),
        "get_prompt" => format_prompt_messages(result),
        _ => pretty_json(result),
    };
    ToolOutput::ok(cap_output(content))
}

fn format_resource_contents(result: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(contents) = result.get("contents").and_then(Value::as_array) {
        for item in contents {
            let uri = item.get("uri").and_then(Value::as_str).unwrap_or("");
            if let Some(text) = item.get("text").and_then(Value::as_str) {
                parts.push(format!("{uri}\n{text}"));
            } else if let Some(blob) = item.get("blob").and_then(Value::as_str) {
                let mime = item
                    .get("mimeType")
                    .and_then(Value::as_str)
                    .unwrap_or("application/octet-stream");
                parts.push(format!("{uri}\n[blob {mime}, {} ký tự base64]", blob.len()));
            } else {
                parts.push(item.to_string());
            }
        }
    }
    if parts.is_empty() {
        pretty_json(result)
    } else {
        parts.join("\n\n")
    }
}

fn format_prompt_messages(result: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(messages) = result.get("messages").and_then(Value::as_array) {
        for message in messages {
            let role = message
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("message");
            let content = message
                .get("content")
                .map(prompt_content_text)
                .unwrap_or_default();
            parts.push(format!("{role}: {content}"));
        }
    }
    if parts.is_empty() {
        pretty_json(result)
    } else {
        parts.join("\n\n")
    }
}

fn prompt_content_text(content: &Value) -> String {
    if let Some(text) = content.as_str() {
        return text.to_string();
    }
    if let Some(text) = content.get("text").and_then(Value::as_str) {
        return text.to_string();
    }
    if let Some(items) = content.as_array() {
        let texts: Vec<String> = items
            .iter()
            .filter_map(|item| {
                item.get("text")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| item.as_str().map(str::to_string))
            })
            .collect();
        if !texts.is_empty() {
            return texts.join("\n");
        }
    }
    content.to_string()
}

fn parse_call_result(result: &Value) -> ToolOutput {
    let is_error = result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut parts = Vec::new();

    if let Some(content) = result.get("content").and_then(Value::as_array) {
        for item in content {
            match item.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        parts.push(text.to_string());
                    }
                }
                Some("image") => {
                    let mime = item
                        .get("mimeType")
                        .and_then(Value::as_str)
                        .unwrap_or("image");
                    parts.push(format!("[image: {mime}]"));
                }
                Some("resource") => {
                    if let Some(text) = item
                        .get("resource")
                        .and_then(|resource| resource.get("text"))
                        .and_then(Value::as_str)
                    {
                        parts.push(text.to_string());
                    }
                }
                _ => {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        parts.push(text.to_string());
                    }
                }
            }
        }
    }

    if parts.is_empty() {
        parts.push(result.to_string());
    }
    let content = cap_output(parts.join("\n"));
    if is_error {
        ToolOutput::error(content)
    } else {
        ToolOutput::ok(content)
    }
}

/// Tên tool MCP phải an toàn cho OpenAI function calling.
pub fn sanitize_tool_name(server: &str, tool: &str) -> String {
    let raw = format!("mcp__{server}__{tool}");
    let mut name: String = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if name.len() > 64 {
        name.truncate(64);
    }
    name
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) -> Result<(), McpError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) -> Result<(), McpError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_tool_names() {
        assert_eq!(
            sanitize_tool_name("demo", "read-file"),
            "mcp__demo__read-file"
        );
        assert_eq!(
            sanitize_tool_name("my server", "read.file"),
            "mcp__my_server__read_file"
        );
    }

    #[test]
    fn parses_read_only_hint() {
        let tools = parse_tools(&json!({
            "tools": [
                {"name": "read", "annotations": {"readOnlyHint": true}},
                {"name": "write", "annotations": {"readOnlyHint": false}},
                {"name": "legacy", "readOnly": true},
                {"name": "plain"}
            ]
        }));
        assert_eq!(tools.len(), 4);
        assert!(tools[0].read_only_hint);
        assert!(!tools[1].read_only_hint);
        assert!(!tools[2].read_only_hint);
        assert!(!tools[3].read_only_hint);
    }

    #[test]
    fn parses_text_and_error_results() {
        let ok = parse_call_result(&json!({
            "content": [{"type": "text", "text": "hello"}],
            "isError": false
        }));
        assert_eq!(ok.content, "hello");
        assert!(!ok.is_error);

        let error = parse_call_result(&json!({
            "content": [{"type": "text", "text": "boom"}],
            "isError": true
        }));
        assert!(error.is_error);
    }

    #[test]
    fn caps_large_tool_output() {
        let big = "x".repeat(100 * 1024);
        let output = parse_call_result(&json!({
            "content": [{"type": "text", "text": big}],
            "isError": false
        }));
        assert!(output.content.len() < 70 * 1024);
        assert!(output.content.contains("output MCP đã bị cắt bớt"));
    }

    #[test]
    fn config_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = McpConfig::default();
        config.servers.insert(
            "demo".into(),
            McpServerConfig {
                command: "python3".into(),
                args: vec!["server.py".into()],
                env: HashMap::new(),
                cwd: None,
                enabled: true,
                read_only: false,
                url: None,
                headers: HashMap::new(),
                bearer_token_env: None,
                oauth: None,
                sandbox: McpSandboxSettings::default(),
            },
        );
        save_config(dir.path(), &config).unwrap();
        let loaded = load_config(dir.path()).unwrap();
        assert_eq!(loaded.servers["demo"].command, "python3");
    }

    #[test]
    fn default_mcp_sandbox_follows_platform() {
        let settings = McpSandboxSettings::default();
        assert_eq!(settings.enabled, cfg!(target_os = "linux"));
        assert!(!settings.allow_network);
        assert!(settings.writable_paths.is_empty());
    }

    #[test]
    fn missing_sandbox_field_uses_secure_default() {
        let config: McpServerConfig = serde_json::from_str(r#"{"command":"python3"}"#).unwrap();
        assert_eq!(config.sandbox.enabled, cfg!(target_os = "linux"));
        assert!(!config.sandbox.allow_network);
    }

    #[test]
    fn partial_sandbox_keeps_secure_enabled_default() {
        let config: McpServerConfig =
            serde_json::from_str(r#"{"command":"python3","sandbox":{"allow_network":true}}"#)
                .unwrap();
        assert_eq!(config.sandbox.enabled, cfg!(target_os = "linux"));
        assert!(config.sandbox.allow_network);
    }

    #[test]
    fn malformed_global_config_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(config_path(dir.path()), "{not json").unwrap();

        let entries = load_servers_with_project(dir.path(), None).unwrap();

        assert!(entries.is_empty());
    }

    #[test]
    fn save_config_backs_up_corrupt_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_path(dir.path());
        std::fs::write(&path, "{not json").unwrap();

        save_config(dir.path(), &McpConfig::default()).unwrap();

        assert!(load_config(dir.path()).is_ok());
        let backups = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().contains(".corrupt-"))
            .count();
        assert_eq!(backups, 1);
    }

    #[test]
    fn project_servers_require_explicit_trust() {
        let data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();

        let mut config = McpConfig::default();
        config.servers.insert(
            "demo".into(),
            McpServerConfig {
                command: "global".into(),
                args: Vec::new(),
                env: HashMap::new(),
                cwd: None,
                enabled: true,
                read_only: false,
                url: None,
                headers: HashMap::new(),
                bearer_token_env: None,
                oauth: None,
                sandbox: McpSandboxSettings::default(),
            },
        );
        save_config(data.path(), &config).unwrap();

        std::fs::create_dir_all(project.path().join(".tez")).unwrap();
        std::fs::write(
            project.path().join(".tez").join("mcp.json"),
            r#"{"servers":{"demo":{"command":"project"},"extra":{"command":"extra"}}}"#,
        )
        .unwrap();

        let entries = load_servers_with_project(data.path(), Some(project.path())).unwrap();
        let demo = entries.iter().find(|entry| entry.name == "demo").unwrap();
        assert_eq!(demo.config.command, "global");
        assert_eq!(demo.source, "user");
        assert!(entries.iter().all(|entry| entry.name != "extra"));
    }

    #[test]
    fn project_servers_override_user_servers_when_trusted() {
        let data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();

        let mut config = McpConfig::default();
        config.servers.insert(
            "demo".into(),
            McpServerConfig {
                command: "global".into(),
                args: Vec::new(),
                env: HashMap::new(),
                cwd: None,
                enabled: true,
                read_only: false,
                url: None,
                headers: HashMap::new(),
                bearer_token_env: None,
                oauth: None,
                sandbox: McpSandboxSettings::default(),
            },
        );
        save_config(data.path(), &config).unwrap();

        std::fs::create_dir_all(project.path().join(".tez")).unwrap();
        std::fs::write(
            project.path().join(".tez").join("mcp.json"),
            r#"{"trusted":true,"servers":{"demo":{"command":"project"},"extra":{"command":"extra"}}}"#,
        )
        .unwrap();

        let entries = load_servers_with_project(data.path(), Some(project.path())).unwrap();
        let demo = entries.iter().find(|entry| entry.name == "demo").unwrap();
        assert_eq!(demo.config.command, "project");
        assert_eq!(demo.source, "project");
        assert_eq!(
            demo.config.cwd.as_deref(),
            Some(project.path().to_str().unwrap())
        );

        let extra = entries.iter().find(|entry| entry.name == "extra").unwrap();
        assert_eq!(extra.source, "project");
    }

    #[test]
    fn http_config_accepts_url_and_headers() {
        let config: McpServerConfig = serde_json::from_value(json!({
            "url": "https://mcp.example.com/mcp",
            "headers": {"Authorization": "Bearer test"},
            "enabled": true
        }))
        .unwrap();
        assert!(config.command.is_empty());
        assert_eq!(config.url.as_deref(), Some("https://mcp.example.com/mcp"));
        assert_eq!(config.headers["Authorization"], "Bearer test");
    }

    #[test]
    fn parses_json_and_sse_results() {
        let json_body = br#"{"jsonrpc":"2.0","id":1,"result":{"tools":[]}}"#;
        let value = parse_json_result(json_body, 1).unwrap();
        assert_eq!(value["tools"], json!([]));

        let sse_body =
            b"event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"ok\":true}}\n\n";
        let value = parse_sse_result(sse_body, 2).unwrap();
        assert_eq!(value["ok"], json!(true));

        assert!(parse_sse_result(sse_body, 3).is_err());
    }

    #[tokio::test]
    async fn http_transport_lists_and_calls_tools() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        async fn read_request(stream: &mut tokio::net::TcpStream) -> Option<Value> {
            let mut buffer = Vec::new();
            let mut chunk = [0u8; 2048];
            loop {
                let read = stream.read(&mut chunk).await.unwrap();
                if read == 0 {
                    break;
                }
                buffer.extend_from_slice(&chunk[..read]);
                let Some(header_end) = buffer.windows(4).position(|window| window == b"\r\n\r\n")
                else {
                    continue;
                };
                let header = String::from_utf8_lossy(&buffer[..header_end]);
                if header
                    .lines()
                    .next()
                    .is_some_and(|line| line.starts_with("GET "))
                {
                    return None;
                }
                let content_length = header
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        if name.eq_ignore_ascii_case("content-length") {
                            value.trim().parse::<usize>().ok()
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0);
                if buffer.len() >= header_end + 4 + content_length {
                    let body = &buffer[header_end + 4..header_end + 4 + content_length];
                    return Some(serde_json::from_slice(body).unwrap());
                }
            }
            panic!("HTTP request không có body JSON");
        }

        async fn write_response(
            stream: &mut tokio::net::TcpStream,
            status: &str,
            extra_headers: &str,
            body: &str,
        ) {
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra_headers}\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        }

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for _ in 0..5 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let Some(request) = read_request(&mut stream).await else {
                    write_response(&mut stream, "405 Method Not Allowed", "", "").await;
                    continue;
                };
                let method = request["method"].as_str().unwrap_or_default();
                match method {
                    "initialize" => {
                        let body = json!({
                            "jsonrpc": "2.0",
                            "id": request["id"],
                            "result": {
                                "protocolVersion": "2025-03-26",
                                "capabilities": {},
                                "serverInfo": {"name": "test", "version": "0"}
                            }
                        })
                        .to_string();
                        write_response(
                            &mut stream,
                            "200 OK",
                            "Mcp-Session-Id: test-session\r\n",
                            &body,
                        )
                        .await;
                    }
                    "notifications/initialized" => {
                        write_response(&mut stream, "202 Accepted", "", "").await;
                    }
                    "tools/list" => {
                        let body = json!({
                            "jsonrpc": "2.0",
                            "id": request["id"],
                            "result": {
                                "tools": [{
                                    "name": "echo",
                                    "description": "Echo",
                                    "inputSchema": {"type": "object", "properties": {}}
                                }]
                            }
                        })
                        .to_string();
                        write_response(&mut stream, "200 OK", "", &body).await;
                    }
                    "tools/call" => {
                        let body = json!({
                            "jsonrpc": "2.0",
                            "id": request["id"],
                            "result": {
                                "content": [{"type": "text", "text": "pong"}],
                                "isError": false
                            }
                        })
                        .to_string();
                        write_response(&mut stream, "200 OK", "", &body).await;
                    }
                    _ => write_response(&mut stream, "404 Not Found", "", "").await,
                }
            }
        });

        let config = McpServerConfig {
            command: String::new(),
            args: Vec::new(),
            env: HashMap::new(),
            cwd: None,
            enabled: true,
            read_only: false,
            url: Some(format!("http://{address}/mcp")),
            headers: HashMap::new(),
            bearer_token_env: None,
            oauth: None,
            sandbox: McpSandboxSettings::default(),
        };
        let (client, tools) = McpClient::connect("http", &config, None).await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "echo");

        let output = client.call("echo", json!({"text": "ping"})).await.unwrap();
        assert_eq!(output.content, "pong");
        assert!(!output.is_error);
        server.await.unwrap();
    }

    #[test]
    fn extras_format_resources_and_prompts() {
        let resource = json!({
            "contents": [
                {"uri": "file://a", "text": "hello"},
                {"uri": "file://b", "blob": "AAAA", "mimeType": "image/png"}
            ]
        });
        let output = format_extra_result("read_resource", &resource);
        assert!(output.content.contains("file://a\nhello"));
        assert!(output.content.contains("[blob image/png, 4 ký tự base64]"));

        let prompt = json!({
            "messages": [
                {"role": "user", "content": {"type": "text", "text": "hi"}},
                {"role": "assistant", "content": [{"type": "text", "text": "there"}]}
            ]
        });
        let output = format_extra_result("get_prompt", &prompt);
        assert_eq!(output.content, "user: hi\n\nassistant: there");
    }

    #[tokio::test]
    async fn http_transport_reads_resources_and_prompts() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        async fn read_request(stream: &mut tokio::net::TcpStream) -> Option<Value> {
            let mut buffer = Vec::new();
            let mut chunk = [0u8; 2048];
            loop {
                let read = stream.read(&mut chunk).await.unwrap();
                if read == 0 {
                    break;
                }
                buffer.extend_from_slice(&chunk[..read]);
                let Some(header_end) = buffer.windows(4).position(|window| window == b"\r\n\r\n")
                else {
                    continue;
                };
                let header = String::from_utf8_lossy(&buffer[..header_end]);
                if header
                    .lines()
                    .next()
                    .is_some_and(|line| line.starts_with("GET "))
                {
                    return None;
                }
                let content_length = header
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        if name.eq_ignore_ascii_case("content-length") {
                            value.trim().parse::<usize>().ok()
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0);
                if buffer.len() >= header_end + 4 + content_length {
                    let body = &buffer[header_end + 4..header_end + 4 + content_length];
                    return Some(serde_json::from_slice(body).unwrap());
                }
            }
            panic!("HTTP request không có body JSON");
        }

        async fn write_response(
            stream: &mut tokio::net::TcpStream,
            status: &str,
            extra_headers: &str,
            body: &str,
        ) {
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra_headers}\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        }

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for _ in 0..8 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let Some(request) = read_request(&mut stream).await else {
                    write_response(&mut stream, "405 Method Not Allowed", "", "").await;
                    continue;
                };
                let method = request["method"].as_str().unwrap_or_default();
                let result = match method {
                    "initialize" => {
                        let body = json!({
                            "jsonrpc": "2.0",
                            "id": request["id"],
                            "result": {
                                "protocolVersion": "2025-03-26",
                                "capabilities": {"resources": {}, "prompts": {}},
                                "serverInfo": {"name": "test", "version": "0"}
                            }
                        })
                        .to_string();
                        write_response(
                            &mut stream,
                            "200 OK",
                            "Mcp-Session-Id: test-session\r\n",
                            &body,
                        )
                        .await;
                        continue;
                    }
                    "notifications/initialized" => {
                        write_response(&mut stream, "202 Accepted", "", "").await;
                        continue;
                    }
                    "tools/list" => json!({"tools": []}),
                    "resources/list" => json!({
                        "resources": [{"uri": "file://a", "name": "A"}]
                    }),
                    "resources/read" => json!({
                        "contents": [{"uri": "file://a", "text": "hello"}]
                    }),
                    "prompts/list" => json!({
                        "prompts": [{"name": "greet"}]
                    }),
                    "prompts/get" => json!({
                        "messages": [{"role": "user", "content": {"type": "text", "text": "hi"}}]
                    }),
                    _ => json!({}),
                };
                let body = json!({
                    "jsonrpc": "2.0",
                    "id": request["id"],
                    "result": result
                })
                .to_string();
                write_response(&mut stream, "200 OK", "", &body).await;
            }
        });

        let config = McpServerConfig {
            command: String::new(),
            args: Vec::new(),
            env: HashMap::new(),
            cwd: None,
            enabled: true,
            read_only: false,
            url: Some(format!("http://{address}/mcp")),
            headers: HashMap::new(),
            bearer_token_env: None,
            oauth: None,
            sandbox: McpSandboxSettings::default(),
        };
        let (client, tools) = McpClient::connect("http", &config, None).await.unwrap();
        assert!(tools.is_empty());
        assert!(client.supports("resources"));
        assert!(client.supports("prompts"));
        assert!(!client.supports("sampling"));

        let extras = McpExtrasTool::new("http".into(), Arc::clone(&client));
        assert!(extras.is_read_only());
        assert_eq!(extras.spec().name, "mcp__http__extras");

        let output = extras
            .execute(json!({"action": "list_resources"}))
            .await
            .unwrap();
        assert!(output.content.contains("file://a"));

        let output = extras
            .execute(json!({"action": "read_resource", "uri": "file://a"}))
            .await
            .unwrap();
        assert!(output.content.contains("hello"));

        let output = extras
            .execute(json!({"action": "list_prompts"}))
            .await
            .unwrap();
        assert!(output.content.contains("greet"));

        let output = extras
            .execute(json!({"action": "get_prompt", "name": "greet"}))
            .await
            .unwrap();
        assert!(output.content.contains("user: hi"));

        server.await.unwrap();
    }

    #[tokio::test]
    async fn http_get_stream_answers_server_ping() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        async fn read_request(stream: &mut tokio::net::TcpStream) -> Option<Value> {
            let mut buffer = Vec::new();
            let mut chunk = [0u8; 2048];
            loop {
                let read = stream.read(&mut chunk).await.unwrap();
                if read == 0 {
                    break;
                }
                buffer.extend_from_slice(&chunk[..read]);
                let Some(header_end) = buffer.windows(4).position(|window| window == b"\r\n\r\n")
                else {
                    continue;
                };
                let header = String::from_utf8_lossy(&buffer[..header_end]);
                if header
                    .lines()
                    .next()
                    .is_some_and(|line| line.starts_with("GET "))
                {
                    return None;
                }
                let content_length = header
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        if name.eq_ignore_ascii_case("content-length") {
                            value.trim().parse::<usize>().ok()
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0);
                if buffer.len() >= header_end + 4 + content_length {
                    let body = &buffer[header_end + 4..header_end + 4 + content_length];
                    return Some(serde_json::from_slice(body).unwrap());
                }
            }
            panic!("HTTP request không có body JSON");
        }

        async fn write_json(stream: &mut tokio::net::TcpStream, status: &str, body: &str) {
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        }

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let server = tokio::spawn(async move {
            for _ in 0..5 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let Some(request) = read_request(&mut stream).await else {
                    let body = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":\"srv-1\",\"method\":\"ping\"}\n\n";
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    stream.write_all(response.as_bytes()).await.unwrap();
                    continue;
                };
                let method = request["method"].as_str().unwrap_or_default();
                match method {
                    "initialize" => {
                        let body = json!({
                            "jsonrpc": "2.0",
                            "id": request["id"],
                            "result": {
                                "protocolVersion": "2025-03-26",
                                "capabilities": {},
                                "serverInfo": {"name": "test", "version": "0"}
                            }
                        })
                        .to_string();
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nMcp-Session-Id: test-session\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        stream.write_all(response.as_bytes()).await.unwrap();
                    }
                    "notifications/initialized" => {
                        write_json(&mut stream, "202 Accepted", "").await;
                    }
                    "tools/list" => {
                        write_json(
                            &mut stream,
                            "200 OK",
                            &json!({"jsonrpc": "2.0", "id": request["id"], "result": {"tools": []}})
                                .to_string(),
                        )
                        .await;
                    }
                    _ => {
                        tx.send(request).unwrap();
                        write_json(&mut stream, "202 Accepted", "").await;
                    }
                }
            }
        });

        let config = McpServerConfig {
            command: String::new(),
            args: Vec::new(),
            env: HashMap::new(),
            cwd: None,
            enabled: true,
            read_only: false,
            url: Some(format!("http://{address}/mcp")),
            headers: HashMap::new(),
            bearer_token_env: None,
            oauth: None,
            sandbox: McpSandboxSettings::default(),
        };
        let (client, tools) = McpClient::connect("http", &config, None).await.unwrap();
        assert!(tools.is_empty());
        let response = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("không nhận được response cho ping")
            .expect("server đóng kênh");
        assert_eq!(response["id"], json!("srv-1"));
        assert_eq!(response["result"], json!({}));
        drop(client);
        server.await.unwrap();
    }

    #[test]
    fn malformed_project_config_is_ignored_not_fatal() {
        let data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(project.path().join(".tez")).unwrap();
        std::fs::write(project.path().join(".tez").join("mcp.json"), "{ not json").unwrap();

        let entries = load_servers_with_project(data.path(), Some(project.path())).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn server_read_only_hint_is_parsed_but_not_trusted() {
        let infos = parse_tools(&json!({
            "tools": [{
                "name": "write",
                "description": "ghi",
                "inputSchema": {"type": "object"},
                "annotations": {"readOnlyHint": true}
            }]
        }));
        assert!(infos[0].read_only_hint);

        let user_read_only = McpServerConfig {
            command: "demo".into(),
            args: Vec::new(),
            env: HashMap::new(),
            cwd: None,
            enabled: true,
            read_only: true,
            url: None,
            headers: HashMap::new(),
            bearer_token_env: None,
            oauth: None,
            sandbox: McpSandboxSettings::default(),
        };
        assert!(trusted_read_only("user", &user_read_only));
        assert!(!trusted_read_only("project", &user_read_only));
        assert!(!trusted_read_only("demo", &user_read_only));
    }

    fn http_config(headers: HashMap<String, String>, bearer: Option<&str>) -> McpServerConfig {
        McpServerConfig {
            command: String::new(),
            args: Vec::new(),
            env: HashMap::new(),
            cwd: None,
            enabled: true,
            read_only: false,
            url: Some("https://example.com/mcp".into()),
            headers,
            bearer_token_env: bearer.map(str::to_string),
            oauth: None,
            sandbox: McpSandboxSettings::default(),
        }
    }

    #[test]
    fn expands_env_values_in_http_headers() {
        std::env::set_var("TEZ_TEST_MCP_TOKEN", "abc");
        let mut headers = HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            "Bearer ${TEZ_TEST_MCP_TOKEN}".to_string(),
        );
        let resolved = resolve_http_headers(&http_config(headers, None)).unwrap();
        assert_eq!(
            resolved.get("Authorization").map(String::as_str),
            Some("Bearer abc")
        );
        std::env::remove_var("TEZ_TEST_MCP_TOKEN");
    }

    #[test]
    fn missing_env_header_is_an_error() {
        std::env::remove_var("TEZ_TEST_MCP_MISSING");
        let mut headers = HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            "Bearer ${TEZ_TEST_MCP_MISSING}".to_string(),
        );
        let error = resolve_http_headers(&http_config(headers, None)).unwrap_err();
        assert!(error.to_string().contains("TEZ_TEST_MCP_MISSING"));
    }

    #[test]
    fn bearer_token_env_sets_authorization() {
        std::env::set_var("TEZ_TEST_MCP_BEARER", "xyz");
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), "Bearer old".to_string());
        let resolved =
            resolve_http_headers(&http_config(headers, Some("TEZ_TEST_MCP_BEARER"))).unwrap();
        assert_eq!(
            resolved.get("Authorization").map(String::as_str),
            Some("Bearer xyz")
        );
        std::env::remove_var("TEZ_TEST_MCP_BEARER");
    }
}
