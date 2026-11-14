//! Permission pipeline: quyết định tool nào được chạy, tool nào cần người dùng duyệt.
//!
//! Thứ tự ba cổng giống mô hình harness của Claude Code:
//! 1. **Deny** — lệnh phá hủy bị chặn thẳng, không hỏi.
//! 2. **Ask** — lệnh có rủi ro cần người dùng xác nhận.
//! 3. **Allow** — phần còn lại chạy trực tiếp.
//!
//! Workspace tools đã tự chặn đường dẫn ngoài workspace; lớp này bổ sung
//! chính sách cho `run_command` và các tool có tác dụng phụ.

use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

static NEXT_APPROVAL_ID: AtomicU64 = AtomicU64::new(1);

/// Mức độ chặt của permission pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    /// Chỉ đọc: mọi tool ghi/chạy lệnh đều bị từ chối.
    ReadOnly,
    /// Hỏi trước mọi thao tác ghi file hoặc chạy lệnh.
    Strict,
    /// Hỏi với lệnh rủi ro, cho qua thao tác thường. Mặc định.
    #[default]
    Standard,
    /// Chỉ chặn deny-list; không hỏi.
    Trust,
}

impl PermissionMode {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "readonly" | "read_only" | "read-only" => Self::ReadOnly,
            "strict" => Self::Strict,
            "trust" | "trusted" => Self::Trust,
            _ => Self::Standard,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::Strict => "strict",
            Self::Standard => "standard",
            Self::Trust => "trust",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ReadOnly => "Chỉ đọc",
            Self::Strict => "Nghiêm ngặt",
            Self::Standard => "Tiêu chuẩn",
            Self::Trust => "Tin cậy",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::ReadOnly => "Chặn mọi thao tác ghi file và chạy lệnh.",
            Self::Strict => "Hỏi trước mỗi lần ghi file hoặc chạy lệnh.",
            Self::Standard => "Hỏi với lệnh rủi ro, cho qua thao tác thường.",
            Self::Trust => "Chỉ chặn lệnh phá hủy rõ ràng, không hỏi.",
        }
    }
}

/// Kết quả đánh giá một tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionDecision {
    Allow,
    Ask { reason: String },
    Deny { reason: String },
}

impl PermissionDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allow)
    }
}

/// Yêu cầu người dùng duyệt một tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub id: String,
    pub tool: String,
    pub args: Value,
    pub reason: String,
    pub session_id: Option<String>,
}

impl ApprovalRequest {
    pub fn new(
        tool: impl Into<String>,
        args: Value,
        reason: impl Into<String>,
        session_id: Option<String>,
    ) -> Self {
        Self {
            id: format!(
                "approval-{}",
                NEXT_APPROVAL_ID.fetch_add(1, Ordering::Relaxed)
            ),
            tool: tool.into(),
            args,
            reason: reason.into(),
            session_id,
        }
    }
}

/// Bộ duyệt tool call. Desktop dùng dialog + channel; CLI dùng terminal.
#[async_trait]
pub trait Approver: Send + Sync {
    async fn approve(&self, request: &ApprovalRequest) -> bool;
}

/// Luôn cho phép. Dùng cho chế độ `trust` và test.
pub struct AutoApprover;

#[async_trait]
impl Approver for AutoApprover {
    async fn approve(&self, _request: &ApprovalRequest) -> bool {
        true
    }
}

/// Luôn từ chối. Dùng khi không có kênh tương tác.
pub struct DenyApprover;

#[async_trait]
impl Approver for DenyApprover {
    async fn approve(&self, _request: &ApprovalRequest) -> bool {
        false
    }
}

/// Chính sách permission, độc lập với UI.
#[derive(Debug, Clone)]
pub struct PermissionPolicy {
    mode: PermissionMode,
}

impl Default for PermissionPolicy {
    fn default() -> Self {
        Self::new(PermissionMode::Standard)
    }
}

impl PermissionPolicy {
    pub fn new(mode: PermissionMode) -> Self {
        Self { mode }
    }

    pub fn mode(&self) -> PermissionMode {
        self.mode
    }

    /// Ba cổng: deny → ask → allow.
    pub fn evaluate(&self, tool: &str, args: &Value) -> PermissionDecision {
        if let Some(reason) = deny_reason(tool, args) {
            return PermissionDecision::Deny { reason };
        }

        if self.mode == PermissionMode::ReadOnly && is_mutating(tool) {
            return PermissionDecision::Deny {
                reason: "chế độ chỉ đọc: không cho phép ghi file hoặc chạy lệnh".to_string(),
            };
        }

        if let Some(reason) = ask_reason(tool, args) {
            return match self.mode {
                PermissionMode::Trust => PermissionDecision::Allow,
                _ => PermissionDecision::Ask { reason },
            };
        }

        if self.mode == PermissionMode::Strict && is_mutating(tool) {
            return PermissionDecision::Ask {
                reason: "chế độ nghiêm ngặt: cần duyệt mọi thay đổi".to_string(),
            };
        }

        PermissionDecision::Allow
    }
}

fn is_mutating(tool: &str) -> bool {
    matches!(
        tool,
        "write_file" | "edit_file" | "run_command" | "memory_save" | "memory_forget"
    ) || tool.starts_with("mcp__")
}

/// Chuẩn hóa lệnh để so khớp: bỏ quote, gộp khoảng trắng, lowercase.
fn normalize(command: &str) -> String {
    let cleaned: String = command
        .chars()
        .filter(|ch| *ch != '"' && *ch != '\'')
        .collect();
    cleaned
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

const DENY_PATTERNS: &[&str] = &[
    "rm -rf /",
    "rm -rf /*",
    "rm -fr /",
    "mkfs",
    "dd if=",
    "> /dev/sda",
    "> /dev/nvme",
    "shutdown",
    "reboot",
    "poweroff",
    "halt",
    ":(){",
    "chmod -r 777 /",
    "format c:",
    "del /f /s /q c:\\",
    "sudo rm -rf /",
];

fn deny_reason(tool: &str, args: &Value) -> Option<String> {
    if tool != "run_command" {
        return None;
    }
    let command = args.get("command").and_then(Value::as_str)?;
    let normalized = normalize(command);
    for pattern in DENY_PATTERNS {
        if normalized.contains(pattern) {
            return Some(format!("lệnh chứa mẫu bị chặn: '{pattern}'"));
        }
    }
    None
}

/// Regex nhận diện lệnh xóa ở đầu câu lệnh hoặc sau toán tử shell.
fn destructive_command(command: &str) -> bool {
    let pattern =
        Regex::new(r"(?i)(?:^|[;&|()\n]|\bsudo\s+)\s*(?:rm|rmdir|del)\b").expect("valid regex");
    pattern.is_match(command)
}

const ASK_PATTERNS: &[&str] = &[
    "git reset --hard",
    "git clean -fd",
    "git clean -xdf",
    "git push --force",
    "git push -f",
    "git checkout --",
    "drop table",
    "truncate table",
    "chmod",
    "chown",
    "kill ",
    "pkill",
    "systemctl",
    "docker rm",
    "docker rmi",
    "docker system prune",
    "npm install",
    "npm i ",
    "pnpm add",
    "pnpm install",
    "yarn add",
    "pip install",
    "pip3 install",
    "cargo install",
    "apt install",
    "apt-get install",
    "brew install",
    "curl ",
    "wget ",
    "ssh ",
    "scp ",
    "> /etc/",
];

fn ask_reason(tool: &str, args: &Value) -> Option<String> {
    if tool == "run_command" {
        let command = args.get("command").and_then(Value::as_str)?;
        let normalized = normalize(command);
        if destructive_command(command) {
            return Some("lệnh xóa file/thư mục".to_string());
        }
        if normalized.contains("curl") && normalized.contains("| sh") {
            return Some("tải và chạy script từ internet".to_string());
        }
        for pattern in ASK_PATTERNS {
            if normalized.contains(pattern) {
                let label = pattern.trim();
                return Some(format!("lệnh có thể thay đổi hệ thống: '{label}'"));
            }
        }
        return None;
    }

    if tool.starts_with("mcp__") {
        return Some("tool từ MCP server bên ngoài".to_string());
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn allows_routine_reads_and_writes() {
        let policy = PermissionPolicy::default();
        assert!(policy
            .evaluate("read_file", &json!({"path": "a"}))
            .is_allowed());
        assert!(policy
            .evaluate("write_file", &json!({"path": "a", "content": "x"}))
            .is_allowed());
    }

    #[test]
    fn denies_destructive_commands() {
        let policy = PermissionPolicy::default();
        for command in ["rm -rf /", "sudo rm -rf /tmp", "mkfs.ext4 /dev/sda"] {
            let decision = policy.evaluate("run_command", &json!({"command": command}));
            assert!(
                matches!(decision, PermissionDecision::Deny { .. }),
                "{command} -> {decision:?}"
            );
        }
    }

    #[test]
    fn asks_for_risky_commands_but_not_substrings() {
        let policy = PermissionPolicy::default();
        assert!(matches!(
            policy.evaluate("run_command", &json!({"command": "rm notes.txt"})),
            PermissionDecision::Ask { .. }
        ));
        assert!(policy
            .evaluate("run_command", &json!({"command": "echo model && ls"}))
            .is_allowed());
        assert!(matches!(
            policy.evaluate("run_command", &json!({"command": "git reset --hard HEAD"})),
            PermissionDecision::Ask { .. }
        ));
    }

    #[test]
    fn trust_skips_asks_but_keeps_deny_list() {
        let policy = PermissionPolicy::new(PermissionMode::Trust);
        assert!(policy
            .evaluate("run_command", &json!({"command": "rm notes.txt"}))
            .is_allowed());
        assert!(matches!(
            policy.evaluate("run_command", &json!({"command": "rm -rf /"})),
            PermissionDecision::Deny { .. }
        ));
    }

    #[test]
    fn read_only_blocks_mutations() {
        let policy = PermissionPolicy::new(PermissionMode::ReadOnly);
        assert!(policy
            .evaluate("read_file", &json!({"path": "a"}))
            .is_allowed());
        assert!(matches!(
            policy.evaluate("write_file", &json!({"path": "a", "content": "x"})),
            PermissionDecision::Deny { .. }
        ));
        assert!(matches!(
            policy.evaluate("run_command", &json!({"command": "ls"})),
            PermissionDecision::Deny { .. }
        ));
    }

    #[test]
    fn strict_asks_before_writes() {
        let policy = PermissionPolicy::new(PermissionMode::Strict);
        assert!(matches!(
            policy.evaluate("write_file", &json!({"path": "a", "content": "x"})),
            PermissionDecision::Ask { .. }
        ));
    }

    #[tokio::test]
    async fn auto_approver_allows() {
        let request = ApprovalRequest::new("run_command", json!({}), "test", None);
        assert!(AutoApprover.approve(&request).await);
        assert!(!DenyApprover.approve(&request).await);
    }
}
