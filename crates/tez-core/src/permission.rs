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

/// Hành động của một rule do người dùng cấu hình.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleAction {
    Allow,
    Ask,
    Deny,
}

/// Một rule áp cho tool + đối tượng (đường dẫn hoặc lệnh).
///
/// `tool` nhận `*` hoặc tên chính xác. `pattern` là glob khớp path với tool
/// file, khớp command với `run_command`; rỗng hoặc `*` nghĩa là mọi đối tượng.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRule {
    pub action: RuleAction,
    pub tool: String,
    #[serde(default)]
    pub pattern: String,
}

impl PermissionRule {
    pub fn new(action: RuleAction, tool: impl Into<String>, pattern: impl Into<String>) -> Self {
        Self {
            action,
            tool: tool.into(),
            pattern: pattern.into(),
        }
    }

    fn matches(&self, tool: &str, target: &str) -> bool {
        if self.tool != "*" && self.tool != tool {
            return false;
        }
        let pattern = self.pattern.trim();
        if pattern.is_empty() || pattern == "*" {
            return true;
        }
        if target.is_empty() {
            return false;
        }
        glob::Pattern::new(pattern)
            .map(|glob| glob.matches(target))
            .unwrap_or(false)
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
    /// Diff hoặc mô tả thay đổi do lớp UI chuẩn bị (không bắt buộc).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
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
            preview: None,
        }
    }

    pub fn with_preview(mut self, preview: Option<String>) -> Self {
        self.preview = preview;
        self
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
    rules: Vec<PermissionRule>,
}

impl Default for PermissionPolicy {
    fn default() -> Self {
        Self::new(PermissionMode::Standard)
    }
}

impl PermissionPolicy {
    pub fn new(mode: PermissionMode) -> Self {
        Self {
            mode,
            rules: Vec::new(),
        }
    }

    /// Gắn rule người dùng. Rule không bao giờ vượt qua deny-list lệnh phá hủy.
    pub fn with_rules(mut self, rules: Vec<PermissionRule>) -> Self {
        self.rules = rules;
        self
    }

    pub fn mode(&self) -> PermissionMode {
        self.mode
    }

    pub fn rules(&self) -> &[PermissionRule] {
        &self.rules
    }

    /// Ba cổng: deny → ask → allow, với rule người dùng chen giữa.
    pub fn evaluate(&self, tool: &str, args: &Value) -> PermissionDecision {
        if let Some(reason) = deny_reason(tool, args) {
            return PermissionDecision::Deny { reason };
        }

        let target = rule_target(tool, args);
        if let Some(rule) = self
            .rules
            .iter()
            .find(|rule| rule.action == RuleAction::Deny && rule.matches(tool, &target))
        {
            return PermissionDecision::Deny {
                reason: format!("rule deny: {} {}", rule.tool, rule.pattern)
                    .trim()
                    .to_string(),
            };
        }

        if self.mode == PermissionMode::ReadOnly && is_mutating(tool) {
            return PermissionDecision::Deny {
                reason: "chế độ chỉ đọc: không cho phép ghi file hoặc chạy lệnh".to_string(),
            };
        }

        if self
            .rules
            .iter()
            .any(|rule| rule.action == RuleAction::Allow && rule.matches(tool, &target))
        {
            return PermissionDecision::Allow;
        }

        if let Some(reason) = ask_reason(tool, args) {
            return match self.mode {
                PermissionMode::Trust => PermissionDecision::Allow,
                _ => PermissionDecision::Ask { reason },
            };
        }

        if let Some(rule) = self
            .rules
            .iter()
            .find(|rule| rule.action == RuleAction::Ask && rule.matches(tool, &target))
        {
            return PermissionDecision::Ask {
                reason: format!("rule ask: {} {}", rule.tool, rule.pattern)
                    .trim()
                    .to_string(),
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

/// Đối tượng mà rule so khớp: path với tool file, command với `run_command`.
fn rule_target(tool: &str, args: &Value) -> String {
    if tool == "run_command" {
        return args
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
    }
    args.get("path")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
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

/// Tách lệnh thành token, bỏ quote/backslash để `r\m` vẫn nhận ra là `rm`.
fn shell_tokens(command: &str) -> Vec<String> {
    command
        .chars()
        .filter(|ch| !matches!(ch, '"' | '\'' | '\\'))
        .collect::<String>()
        .split(|ch: char| {
            ch.is_whitespace() || matches!(ch, ';' | '&' | '|' | '(' | ')' | '`' | '\n')
        })
        .filter(|token| !token.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn command_name(token: &str) -> &str {
    token.rsplit('/').next().unwrap_or(token)
}

/// Token có phải là lệnh thực sự được chạy không (đầu câu, sau shell/wrapper)?
/// Nhờ vậy `grep shutdown` hay `echo 'rm -rf /'` không bị chặn nhầm.
fn is_command_position(tokens: &[String], index: usize) -> bool {
    if index == 0 {
        return true;
    }
    let previous = tokens[index - 1].as_str();
    if matches!(
        previous,
        "sudo"
            | "doas"
            | "command"
            | "env"
            | "nohup"
            | "exec"
            | "xargs"
            | "time"
            | "nice"
            | "-c"
            | "sh"
            | "bash"
            | "zsh"
            | "dash"
            | "ash"
    ) {
        return true;
    }
    // `sudo -u root rm`, `env FOO=1 rm`, `xargs -0 rm`: wrapper ở gần phía trước.
    tokens[..index].iter().rev().take(4).any(|token| {
        matches!(
            token.as_str(),
            "sudo" | "doas" | "env" | "xargs" | "command" | "nohup"
        )
    })
}

const DELETE_COMMANDS: &[&str] = &["rm", "rmdir", "del", "erase", "rd", "srm", "shred"];

fn delete_command_position(tokens: &[String]) -> Option<usize> {
    tokens.iter().enumerate().find_map(|(index, token)| {
        (DELETE_COMMANDS.contains(&command_name(token)) && is_command_position(tokens, index))
            .then_some(index)
    })
}

/// `rm -rf /`, `rm -rf ~`, `rm -rf $HOME`... bị chặn thẳng, kể cả ở Trust.
fn catastrophic_delete(tokens: &[String]) -> bool {
    let Some(start) = delete_command_position(tokens) else {
        return false;
    };
    let args = &tokens[start + 1..];
    let flags: Vec<&str> = args
        .iter()
        .take_while(|token| token.starts_with('-'))
        .map(String::as_str)
        .collect();
    let targets: Vec<&str> = args
        .iter()
        .filter(|token| !token.starts_with('-'))
        .map(String::as_str)
        .collect();
    let recursive = flags
        .iter()
        .any(|flag| flag.contains('r') || *flag == "--recursive");
    let force = flags
        .iter()
        .any(|flag| flag.contains('f') || *flag == "--force");
    let root_target = targets.iter().any(|target| {
        *target == "/"
            || *target == "*"
            || *target == "~"
            || target.starts_with("~/")
            || *target == "$home"
            || *target == "${home}"
            || *target == "c:"
            || *target == "c:/"
            || [
                "/etc", "/usr", "/var", "/home", "/root", "/bin", "/boot", "/dev", "/lib", "/opt",
                "/sbin", "/srv", "/sys", "/proc",
            ]
            .iter()
            .any(|prefix| *target == *prefix || target.starts_with(&format!("{prefix}/")))
    });

    let windows_delete = matches!(command_name(&tokens[start]), "del" | "erase" | "rd");
    if windows_delete
        && root_target
        && args
            .iter()
            .any(|arg| matches!(arg.as_str(), "/s" | "/q" | "/f"))
    {
        return true;
    }

    // Giữ hành vi cũ: xóa đệ quy + ép buộc vào đường dẫn tuyệt đối là nguy hiểm.
    let absolute = targets.iter().any(|target| target.starts_with('/'));
    (root_target && (recursive || force || targets.contains(&"/")))
        || (recursive && force && absolute)
}

fn deny_reason(tool: &str, args: &Value) -> Option<String> {
    if tool != "run_command" {
        return None;
    }
    let command = args.get("command").and_then(Value::as_str)?;
    let tokens = shell_tokens(command);

    if catastrophic_delete(&tokens) {
        return Some("lệnh xóa toàn bộ thư mục gốc/ổ đĩa".to_string());
    }

    for (index, token) in tokens.iter().enumerate() {
        if !is_command_position(&tokens, index) {
            continue;
        }
        let name = command_name(token);
        if name == "mkfs" || name.starts_with("mkfs.") {
            return Some(format!("lệnh chứa mẫu bị chặn: '{name}'"));
        }
        if matches!(name, "shutdown" | "reboot" | "poweroff" | "halt") {
            return Some(format!("lệnh chứa mẫu bị chặn: '{name}'"));
        }
        if name == "dd" && tokens.iter().any(|token| token.starts_with("of=/dev/")) {
            return Some("lệnh ghi trực tiếp lên thiết bị".to_string());
        }
        if name == "chmod"
            && tokens.iter().any(|token| token == "777" || token == "0777")
            && tokens.iter().any(|token| token == "/")
        {
            return Some("lệnh đổi quyền toàn hệ thống".to_string());
        }
    }

    if tokens
        .iter()
        .any(|token| token.starts_with("/dev/sd") || token.starts_with("/dev/nvme"))
        && tokens.iter().any(|token| token == ">")
    {
        return Some("lệnh ghi trực tiếp lên thiết bị".to_string());
    }
    if command.replace(' ', "").contains(":(){") {
        return Some("fork bomb".to_string());
    }
    None
}

/// Nhận diện lệnh xóa kể cả khi bị bọc: `bash -c 'rm -rf ~'`, `/bin/rm`,
/// `command rm`, `r\m`, `sudo -u root rm`.
fn destructive_command(command: &str) -> bool {
    let tokens = shell_tokens(command);
    if delete_command_position(&tokens).is_some() {
        return true;
    }
    tokens
        .iter()
        .enumerate()
        .any(|(index, token)| command_name(token) == "find" && is_command_position(&tokens, index))
        && tokens.iter().any(|token| token == "-delete")
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

    if tool == "web_fetch" {
        return Some("truy cập mạng bên ngoài".to_string());
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
    fn denies_wrapped_catastrophic_commands() {
        let policy = PermissionPolicy::default();
        for command in [
            "bash -c 'rm -rf ~'",
            "/bin/rm -rf /",
            "command rm -rf /",
            "r\\m -rf /",
            "sudo -u root rm -rf /",
            "rm -rf $HOME",
            "rm -rf --no-preserve-root /",
        ] {
            let decision = policy.evaluate("run_command", &json!({"command": command}));
            assert!(
                matches!(decision, PermissionDecision::Deny { .. }),
                "{command} -> {decision:?}"
            );
        }
    }

    #[test]
    fn asks_for_wrapped_delete_commands() {
        let policy = PermissionPolicy::default();
        for command in [
            "bash -c 'rm notes.txt'",
            "/bin/rm notes.txt",
            "command rm notes.txt",
            "r\\m notes.txt",
        ] {
            let decision = policy.evaluate("run_command", &json!({"command": command}));
            assert!(
                matches!(decision, PermissionDecision::Ask { .. }),
                "{command} -> {decision:?}"
            );
        }
    }

    #[test]
    fn does_not_deny_dangerous_words_in_arguments() {
        let policy = PermissionPolicy::default();
        for command in [
            "grep shutdown logs.txt",
            "echo mkfs",
            "grep 'rm -rf /' notes.txt",
            "cat reboot.md",
        ] {
            let decision = policy.evaluate("run_command", &json!({"command": command}));
            assert!(
                !matches!(decision, PermissionDecision::Deny { .. }),
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

    #[test]
    fn user_rules_allow_ask_and_deny() {
        let policy = PermissionPolicy::default().with_rules(vec![
            PermissionRule::new(RuleAction::Allow, "run_command", "git status*"),
            PermissionRule::new(RuleAction::Ask, "run_command", "git push*"),
            PermissionRule::new(RuleAction::Deny, "write_file", "secrets/**"),
        ]);

        assert!(policy
            .evaluate("run_command", &json!({"command": "git status --short"}))
            .is_allowed());
        assert!(matches!(
            policy.evaluate("run_command", &json!({"command": "git push origin main"})),
            PermissionDecision::Ask { .. }
        ));
        assert!(matches!(
            policy.evaluate("write_file", &json!({"path": "secrets/key.txt"})),
            PermissionDecision::Deny { .. }
        ));
    }

    #[test]
    fn allow_rule_overrides_builtin_ask_but_not_catastrophic_deny() {
        let policy = PermissionPolicy::default().with_rules(vec![PermissionRule::new(
            RuleAction::Allow,
            "run_command",
            "*",
        )]);

        assert!(policy
            .evaluate("run_command", &json!({"command": "rm notes.txt"}))
            .is_allowed());
        assert!(matches!(
            policy.evaluate("run_command", &json!({"command": "rm -rf /"})),
            PermissionDecision::Deny { .. }
        ));
    }

    #[test]
    fn read_only_mode_wins_over_allow_rules() {
        let policy = PermissionPolicy::new(PermissionMode::ReadOnly)
            .with_rules(vec![PermissionRule::new(RuleAction::Allow, "*", "*")]);

        assert!(matches!(
            policy.evaluate("write_file", &json!({"path": "a", "content": "x"})),
            PermissionDecision::Deny { .. }
        ));
    }

    #[test]
    fn wildcard_tool_rule_matches_file_paths() {
        let policy = PermissionPolicy::new(PermissionMode::Strict)
            .with_rules(vec![PermissionRule::new(RuleAction::Allow, "*", "src/**")]);

        assert!(policy
            .evaluate("write_file", &json!({"path": "src/main.rs"}))
            .is_allowed());
        assert!(matches!(
            policy.evaluate("write_file", &json!({"path": "README.md"})),
            PermissionDecision::Ask { .. }
        ));
    }

    #[test]
    fn asks_before_network_fetch() {
        let policy = PermissionPolicy::default();
        assert!(matches!(
            policy.evaluate("web_fetch", &json!({"url": "https://example.com"})),
            PermissionDecision::Ask { .. }
        ));
        let trust = PermissionPolicy::new(PermissionMode::Trust);
        assert!(trust
            .evaluate("web_fetch", &json!({"url": "https://example.com"}))
            .is_allowed());
    }

    #[tokio::test]
    async fn auto_approver_allows() {
        let request = ApprovalRequest::new("run_command", json!({}), "test", None);
        assert!(AutoApprover.approve(&request).await);
        assert!(!DenyApprover.approve(&request).await);
    }
}
