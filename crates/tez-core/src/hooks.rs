//! Hook system: gắn hành vi vào vòng lặp agent mà không sửa vòng lặp.
//!
//! Bốn điểm móc:
//! - `UserPromptSubmit` — sau khi nhận câu hỏi, trước khi gọi model.
//! - `PreToolUse` — trước khi chạy tool; trả về `block` để chặn.
//! - `PostToolUse` — sau khi chạy tool; có thể thay output.
//! - `Stop` — trước khi kết thúc; trả về `continue_with` để chạy tiếp.
//!
//! Hook built-in: audit JSONL, che bí mật trong output. Người dùng có thể thêm
//! hook shell trong `harness.json`.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    UserPromptSubmit,
    PreToolUse,
    PostToolUse,
    Stop,
}

impl HookEvent {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_lowercase().replace('-', "_").as_str() {
            "user_prompt_submit" | "userpromptsubmit" => Some(Self::UserPromptSubmit),
            "pre_tool_use" | "pretooluse" => Some(Self::PreToolUse),
            "post_tool_use" | "posttooluse" => Some(Self::PostToolUse),
            "stop" => Some(Self::Stop),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserPromptSubmit => "user_prompt_submit",
            Self::PreToolUse => "pre_tool_use",
            Self::PostToolUse => "post_tool_use",
            Self::Stop => "stop",
        }
    }
}

/// Dữ liệu hook nhận được. Owned để dễ đi qua `.await`.
#[derive(Debug, Clone, Serialize)]
pub struct HookContext {
    pub event: HookEvent,
    pub session_id: Option<String>,
    pub request: Option<String>,
    pub tool: Option<String>,
    pub args: Option<Value>,
    pub output: Option<String>,
}

impl HookContext {
    pub fn event(event: HookEvent) -> Self {
        Self {
            event,
            session_id: None,
            request: None,
            tool: None,
            args: None,
            output: None,
        }
    }

    pub fn session(mut self, session_id: Option<&str>) -> Self {
        self.session_id = session_id.map(ToOwned::to_owned);
        self
    }

    pub fn request(mut self, request: Option<&str>) -> Self {
        self.request = request.map(ToOwned::to_owned);
        self
    }

    pub fn tool(mut self, tool: &str, args: &Value) -> Self {
        self.tool = Some(tool.to_string());
        self.args = Some(args.clone());
        self
    }

    pub fn output(mut self, output: &str) -> Self {
        self.output = Some(output.to_string());
        self
    }
}

#[derive(Debug, Default, Clone)]
pub struct HookResult {
    /// Chặn tool call (PreToolUse).
    pub block: Option<String>,
    /// Thay output (PostToolUse).
    pub output: Option<String>,
    /// Buộc agent chạy tiếp với message này (Stop).
    pub continue_with: Option<String>,
    /// Ghi chú cho log/UI.
    pub notes: Vec<String>,
}

impl HookResult {
    fn note(message: impl Into<String>) -> Self {
        Self {
            notes: vec![message.into()],
            ..Self::default()
        }
    }
}

#[async_trait]
pub trait Hook: Send + Sync {
    fn name(&self) -> &str;
    fn events(&self) -> &[HookEvent];
    async fn run(&self, ctx: &HookContext) -> HookResult;
}

/// Hook chạy một lệnh shell, nhận JSON qua stdin và trả JSON qua stdout.
///
/// stdout hợp lệ:
/// ```json
/// {"block": "lý do", "output": "output mới", "continue": "message", "note": "..."}
/// ```
/// Exit code khác 0 được coi là chặn ở `PreToolUse`.
pub struct CommandHook {
    name: String,
    event: HookEvent,
    command: String,
    timeout: Duration,
}

impl CommandHook {
    pub fn new(event: HookEvent, command: impl Into<String>, timeout_secs: u64) -> Self {
        let command = command.into();
        Self {
            name: format!("command:{}", command.chars().take(40).collect::<String>()),
            event,
            command,
            timeout: Duration::from_secs(timeout_secs.clamp(1, 60)),
        }
    }
}

#[async_trait]
impl Hook for CommandHook {
    fn name(&self) -> &str {
        &self.name
    }

    fn events(&self) -> &[HookEvent] {
        std::slice::from_ref(&self.event)
    }

    async fn run(&self, ctx: &HookContext) -> HookResult {
        let payload = match serde_json::to_vec(ctx) {
            Ok(payload) => payload,
            Err(error) => return HookResult::note(format!("hook serialize lỗi: {error}")),
        };

        #[cfg(windows)]
        let mut process = {
            let mut process = tokio::process::Command::new("cmd");
            process.args(["/C", &self.command]);
            process
        };
        #[cfg(not(windows))]
        let mut process = {
            let mut process = tokio::process::Command::new("sh");
            process.args(["-c", &self.command]);
            process
        };

        process
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = match process.spawn() {
            Ok(child) => child,
            Err(error) => return HookResult::note(format!("hook không chạy được: {error}")),
        };

        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(&payload).await;
        }

        let output = match tokio::time::timeout(self.timeout, child.wait_with_output()).await {
            Ok(Ok(output)) => output,
            Ok(Err(error)) => return HookResult::note(format!("hook lỗi: {error}")),
            Err(_) => {
                return HookResult::note(format!(
                    "hook vượt quá {}s và đã bị dừng",
                    self.timeout.as_secs()
                ))
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

        if !output.status.success() {
            let reason = if stderr.is_empty() {
                format!("hook '{}' thoát với mã lỗi", self.name)
            } else {
                stderr
            };
            return HookResult {
                block: Some(reason),
                ..HookResult::default()
            };
        }

        if stdout.is_empty() {
            return HookResult::default();
        }

        match serde_json::from_str::<Value>(&stdout) {
            Ok(value) => {
                let mut result = HookResult {
                    block: value
                        .get("block")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    output: value
                        .get("output")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    continue_with: value
                        .get("continue")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    notes: Vec::new(),
                };
                if let Some(note) = value.get("note").and_then(Value::as_str) {
                    result.notes.push(note.to_string());
                }
                result
            }
            Err(_) => HookResult::note(stdout),
        }
    }
}

/// Ghi audit log JSONL: mỗi tool call và lần kết thúc phiên một dòng.
pub struct AuditHook {
    path: PathBuf,
}

impl AuditHook {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

#[async_trait]
impl Hook for AuditHook {
    fn name(&self) -> &str {
        "audit"
    }

    fn events(&self) -> &[HookEvent] {
        &[
            HookEvent::PreToolUse,
            HookEvent::PostToolUse,
            HookEvent::Stop,
        ]
    }

    async fn run(&self, ctx: &HookContext) -> HookResult {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        let line = json!({
            "ts": timestamp,
            "event": ctx.event.as_str(),
            "session_id": ctx.session_id,
            "tool": ctx.tool,
            "args": ctx.args,
            "output": ctx.output.as_ref().map(|value| {
                let mut preview: String = value.chars().take(500).collect();
                if value.chars().count() > 500 {
                    preview.push('…');
                }
                preview
            }),
        });

        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            use std::io::Write;
            let _ = writeln!(file, "{line}");
        }
        HookResult::default()
    }
}

/// Che các mẫu giống API key/token trước khi đưa output về model.
pub struct SecretRedactionHook {
    patterns: Vec<(Regex, &'static str)>,
}

impl Default for SecretRedactionHook {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretRedactionHook {
    pub fn new() -> Self {
        let patterns = vec![
            (
                Regex::new(r"sk-[A-Za-z0-9_\-]{16,}").expect("valid regex"),
                "sk-***",
            ),
            (
                Regex::new(r"gh[pousr]_[A-Za-z0-9]{20,}").expect("valid regex"),
                "gh*_***",
            ),
            (
                Regex::new(r"AKIA[0-9A-Z]{16}").expect("valid regex"),
                "AKIA***",
            ),
            (
                Regex::new(r"xox[baprs]-[A-Za-z0-9\-]{10,}").expect("valid regex"),
                "xox*-***",
            ),
            (
                Regex::new(
                    r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----",
                )
                .expect("valid regex"),
                "[PRIVATE KEY REDACTED]",
            ),
        ];
        Self { patterns }
    }
}

#[async_trait]
impl Hook for SecretRedactionHook {
    fn name(&self) -> &str {
        "secret-redaction"
    }

    fn events(&self) -> &[HookEvent] {
        &[HookEvent::PostToolUse]
    }

    async fn run(&self, ctx: &HookContext) -> HookResult {
        let Some(output) = &ctx.output else {
            return HookResult::default();
        };
        let mut redacted = output.clone();
        let mut changed = false;
        for (pattern, replacement) in &self.patterns {
            if pattern.is_match(&redacted) {
                redacted = pattern.replace_all(&redacted, *replacement).to_string();
                changed = true;
            }
        }
        if changed {
            HookResult {
                output: Some(redacted),
                notes: vec!["đã che bí mật trong output tool".to_string()],
                ..HookResult::default()
            }
        } else {
            HookResult::default()
        }
    }
}

/// Cấu hình hook trong `harness.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookConfig {
    pub event: String,
    pub command: String,
    #[serde(default = "default_hook_timeout")]
    pub timeout_secs: u64,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_hook_timeout() -> u64 {
    5
}

fn default_true() -> bool {
    true
}

impl HookConfig {
    pub fn to_hook(&self) -> Option<CommandHook> {
        if !self.enabled {
            return None;
        }
        let event = HookEvent::parse(&self.event)?;
        Some(CommandHook::new(
            event,
            self.command.clone(),
            self.timeout_secs,
        ))
    }
}

#[derive(Default, Clone)]
pub struct HookRegistry {
    hooks: Vec<Arc<dyn Hook>>,
}

impl HookRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<H: Hook + 'static>(&mut self, hook: H) {
        self.hooks.push(Arc::new(hook));
    }

    pub fn register_shared(&mut self, hook: Arc<dyn Hook>) {
        self.hooks.push(hook);
    }

    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    pub fn names(&self) -> Vec<String> {
        self.hooks
            .iter()
            .map(|hook| hook.name().to_string())
            .collect()
    }

    async fn run_event(&self, ctx: &HookContext) -> HookResult {
        let mut combined = HookResult::default();
        for hook in &self.hooks {
            if !hook.events().contains(&ctx.event) {
                continue;
            }
            let result = hook.run(ctx).await;
            combined.notes.extend(result.notes);
            if result.block.is_some() {
                combined.block = result.block;
                return combined;
            }
            if result.output.is_some() {
                combined.output = result.output;
            }
            if result.continue_with.is_some() {
                combined.continue_with = result.continue_with;
            }
        }
        combined
    }

    pub async fn pre_tool(&self, ctx: &HookContext) -> HookResult {
        self.run_event(ctx).await
    }

    pub async fn post_tool(&self, ctx: &HookContext) -> HookResult {
        self.run_event(ctx).await
    }

    pub async fn user_prompt(&self, ctx: &HookContext) -> HookResult {
        self.run_event(ctx).await
    }

    pub async fn stop(&self, ctx: &HookContext) -> HookResult {
        self.run_event(ctx).await
    }
}

/// Xây registry từ config, kèm audit + redaction mặc định.
pub fn build_registry(data_dir: Option<&Path>, configs: &[HookConfig]) -> HookRegistry {
    let mut registry = HookRegistry::new();
    if let Some(dir) = data_dir {
        registry.register(AuditHook::new(dir.join("audit.jsonl")));
    }
    registry.register(SecretRedactionHook::new());
    for config in configs {
        if let Some(hook) = config.to_hook() {
            registry.register(hook);
        }
    }
    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    struct BlockHook;

    #[async_trait]
    impl Hook for BlockHook {
        fn name(&self) -> &str {
            "block"
        }

        fn events(&self) -> &[HookEvent] {
            &[HookEvent::PreToolUse]
        }

        async fn run(&self, _ctx: &HookContext) -> HookResult {
            HookResult {
                block: Some("không được phép".into()),
                ..HookResult::default()
            }
        }
    }

    #[tokio::test]
    async fn pre_tool_hook_can_block() {
        let mut registry = HookRegistry::new();
        registry.register(BlockHook);
        let ctx = HookContext::event(HookEvent::PreToolUse)
            .tool("run_command", &json!({"command": "ls"}));
        let result = registry.pre_tool(&ctx).await;
        assert_eq!(result.block.as_deref(), Some("không được phép"));
    }

    #[tokio::test]
    async fn redacts_api_keys() {
        let hook = SecretRedactionHook::new();
        let ctx = HookContext::event(HookEvent::PostToolUse)
            .output("key=sk-abcdefghijklmnopqrstuvwx and ghp_abcdefghijklmnopqrstuvwx");
        let result = hook.run(&ctx).await;
        let output = result.output.unwrap();
        assert!(!output.contains("sk-abcdefghijklmnopqrstuvwx"));
        assert!(!output.contains("ghp_abcdefghijklmnopqrstuvwx"));
        assert!(output.contains("sk-***"));
    }

    #[test]
    fn parses_hook_events() {
        assert_eq!(HookEvent::parse("PreToolUse"), Some(HookEvent::PreToolUse));
        assert_eq!(
            HookEvent::parse("post-tool-use"),
            Some(HookEvent::PostToolUse)
        );
        assert_eq!(HookEvent::parse("nope"), None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn command_hook_reads_json_and_blocks() {
        let hook = CommandHook::new(
            HookEvent::PreToolUse,
            "cat >/dev/null; printf '{\"block\":\"blocked by script\"}'",
            5,
        );
        let ctx = HookContext::event(HookEvent::PreToolUse)
            .tool("run_command", &json!({"command": "ls"}));
        let result = hook.run(&ctx).await;
        assert_eq!(result.block.as_deref(), Some("blocked by script"));
    }
}
