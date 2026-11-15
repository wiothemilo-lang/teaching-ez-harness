//! MCP client tối giản qua stdio (JSON-RPC 2.0).
//!
//! Hỗ trợ đủ cho MVP: `initialize`, `tools/list`, `tools/call`. Mỗi server là
//! một process con được giữ sống trong lúc agent chạy; tool MCP được đăng ký
//! vào cùng [`crate::tools::ToolRegistry`] với tool built-in.

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
    #[error("MCP error: {0}")]
    Rpc(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpConfig {
    #[serde(default)]
    pub servers: HashMap<String, McpServerConfig>,
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
        let project = load_project_config(root)?;
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

        match McpClient::connect(&entry.name, &entry.config).await {
            Ok((client, infos)) => {
                let tool_names = infos.iter().map(|info| info.name.clone()).collect();
                for info in infos {
                    tools.push(McpTool::new(entry.name.clone(), info, Arc::clone(&client)));
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

    Ok(McpDiscovery { tools, servers })
}

pub struct McpTool {
    server: String,
    raw_name: String,
    tool_name: String,
    description: String,
    parameters: Value,
    client: Arc<McpClient>,
}

impl McpTool {
    fn new(server: String, info: McpToolInfo, client: Arc<McpClient>) -> Self {
        let tool_name = sanitize_tool_name(&server, &info.name);
        let description = format!("[MCP: {}] {}", server, info.description.trim());
        Self {
            server,
            raw_name: info.name,
            tool_name,
            description,
            parameters: info.input_schema,
            client,
        }
    }

    pub fn server(&self) -> &str {
        &self.server
    }

    pub fn raw_name(&self) -> &str {
        &self.raw_name
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

pub struct McpClient {
    inner: Mutex<McpConnection>,
}

struct McpConnection {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl McpClient {
    pub async fn connect(
        _server: &str,
        config: &McpServerConfig,
    ) -> Result<(Arc<Self>, Vec<McpToolInfo>), McpError> {
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

        let mut connection = McpConnection {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        };

        connection
            .request(
                "initialize",
                json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {
                        "name": "tez",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }),
            )
            .await?;
        connection
            .notify("notifications/initialized", json!({}))
            .await?;
        let result = connection.request("tools/list", json!({})).await?;
        let tools = parse_tools(&result);

        Ok((
            Arc::new(Self {
                inner: Mutex::new(connection),
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

impl Drop for McpConnection {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

impl McpConnection {
    async fn request(&mut self, method: &str, params: Value) -> Result<Value, McpError> {
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

    async fn notify(&mut self, method: &str, params: Value) -> Result<(), McpError> {
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
                    Some(McpToolInfo {
                        name,
                        description,
                        input_schema,
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
            },
        );
        save_config(dir.path(), &config).unwrap();
        let loaded = load_config(dir.path()).unwrap();
        assert_eq!(loaded.servers["demo"].command, "python3");
    }

    #[test]
    fn project_servers_override_user_servers() {
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
        assert_eq!(demo.config.command, "project");
        assert_eq!(demo.source, "project");
        assert_eq!(
            demo.config.cwd.as_deref(),
            Some(project.path().to_str().unwrap())
        );

        let extra = entries.iter().find(|entry| entry.name == "extra").unwrap();
        assert_eq!(extra.source, "project");
    }
}
