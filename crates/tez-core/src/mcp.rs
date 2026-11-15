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
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tez_llm::ToolSpec;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

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
}

pub struct McpDiscovery {
    pub tools: Vec<McpTool>,
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

/// Nạp cấu hình dự án nhưng không bao giờ làm hỏng phiên chat.
///
/// `.tez/mcp.json` do repo kiểm soát nên có thể hỏng, quá lớn hoặc bị sửa bởi
/// người khác. Khi đó chỉ cảnh báo và bỏ qua file, không trả lỗi cho caller.
fn load_project_config_lenient(root: impl AsRef<Path>) -> McpConfig {
    let path = project_config_path(root);
    if !path.exists() {
        return McpConfig::default();
    }
    let bytes = match std::fs::read(&path) {
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

pub fn save_config(dir: impl AsRef<Path>, config: &McpConfig) -> Result<(), McpError> {
    let path = config_path(dir);
    std::fs::write(&path, serde_json::to_vec_pretty(config)?)?;
    restrict_permissions(&path)?;
    Ok(())
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
    let config = load_config(dir)?;
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
    let entries = load_servers_with_project(dir, project_root)?;
    let mut tools = Vec::new();
    let mut servers = Vec::new();
    let mut read_only_tools = Vec::new();

    for entry in entries {
        if !entry.config.enabled {
            servers.push(McpServerStatus {
                name: entry.name,
                source: entry.source,
                ok: false,
                tools: Vec::new(),
                error: Some("đang tắt".to_string()),
            });
            continue;
        }

        let read_only = trusted_read_only(&entry.source, &entry.config);
        match McpClient::connect(&entry.name, &entry.config).await {
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
                servers.push(McpServerStatus {
                    name: entry.name,
                    source: entry.source,
                    ok: true,
                    tools: tool_names,
                    error: None,
                });
            }
            Err(error) => servers.push(McpServerStatus {
                name: entry.name,
                source: entry.source,
                ok: false,
                tools: Vec::new(),
                error: Some(error.to_string()),
            }),
        }
    }

    Ok(McpDiscovery {
        tools,
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

#[async_trait]
trait McpTransport: Send {
    fn protocol_version(&self) -> &'static str;
    async fn request(&mut self, method: &str, params: Value) -> Result<Value, McpError>;
    async fn notify(&mut self, method: &str, params: Value) -> Result<(), McpError>;
}

pub struct McpClient {
    inner: Mutex<Box<dyn McpTransport>>,
}

impl McpClient {
    pub async fn connect(
        _server: &str,
        config: &McpServerConfig,
    ) -> Result<(Arc<Self>, Vec<McpToolInfo>), McpError> {
        let use_http = config
            .url
            .as_deref()
            .map(str::trim)
            .is_some_and(|url| !url.is_empty());
        let mut transport: Box<dyn McpTransport> = if use_http {
            Box::new(HttpConnection::connect(config).await?)
        } else {
            Box::new(StdioConnection::connect(config)?)
        };

        let protocol = transport.protocol_version();
        transport
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
        transport
            .notify("notifications/initialized", json!({}))
            .await?;
        let result = transport.request("tools/list", json!({})).await?;
        let tools = parse_tools(&result);

        Ok((
            Arc::new(Self {
                inner: Mutex::new(transport),
            }),
            tools,
        ))
    }

    pub async fn call(&self, tool: &str, args: Value) -> Result<ToolOutput, McpError> {
        let mut connection = self.inner.lock().await;
        let result = connection
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
}

struct StdioConnection {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl StdioConnection {
    fn connect(config: &McpServerConfig) -> Result<Self, McpError> {
        if config.command.trim().is_empty() {
            return Err(McpError::Spawn(
                "thiếu command và url cho MCP server".into(),
            ));
        }
        let mut command = Command::new(&config.command);
        command
            .args(&config.args)
            .envs(&config.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        if let Some(cwd) = &config.cwd {
            command.current_dir(cwd);
        }

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

struct HttpConnection {
    client: reqwest::Client,
    url: String,
    headers: HashMap<String, String>,
    session_id: Option<String>,
    next_id: u64,
}

impl HttpConnection {
    async fn connect(config: &McpServerConfig) -> Result<Self, McpError> {
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
        Ok(Self {
            client,
            url: raw.to_string(),
            headers: config.headers.clone(),
            session_id: None,
            next_id: 1,
        })
    }

    async fn post(&mut self, payload: &Value) -> Result<reqwest::Response, McpError> {
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
        Ok(())
    }
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
    let mut content = parts.join("\n");
    if content.len() > MAX_MCP_OUTPUT_BYTES {
        let mut end = MAX_MCP_OUTPUT_BYTES;
        while !content.is_char_boundary(end) {
            end -= 1;
        }
        content.truncate(end);
        content.push_str("\n… (output MCP đã bị cắt bớt)");
    }
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
            },
        );
        save_config(dir.path(), &config).unwrap();
        let loaded = load_config(dir.path()).unwrap();
        assert_eq!(loaded.servers["demo"].command, "python3");
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

        async fn read_request(stream: &mut tokio::net::TcpStream) -> Value {
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
                    return serde_json::from_slice(body).unwrap();
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
            for _ in 0..4 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let request = read_request(&mut stream).await;
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
        };
        let (client, tools) = McpClient::connect("http", &config).await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "echo");

        let output = client.call("echo", json!({"text": "ping"})).await.unwrap();
        assert_eq!(output.content, "pong");
        assert!(!output.is_error);
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
        };
        assert!(trusted_read_only("user", &user_read_only));
        assert!(!trusted_read_only("project", &user_read_only));
        assert!(!trusted_read_only("demo", &user_read_only));
    }
}
