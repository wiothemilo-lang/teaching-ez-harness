//! Tool cho chế độ Code: đọc/ghi file, liệt kê thư mục và chạy lệnh trong workspace.
//!
//! Mọi đường dẫn đều bị giới hạn trong workspace. Đây là lớp bảo vệ chính, không
//! phải sandbox của hệ điều hành: người dùng vẫn nên chạy tez bằng tài khoản
//! không có quyền cao.

use std::path::{Component, Path, PathBuf};
use std::process::Stdio;

use async_trait::async_trait;
use serde_json::{json, Value};
use tez_llm::ToolSpec;

use super::{Tool, ToolError, ToolOutput};

const MAX_READ_BYTES: u64 = 256 * 1024;
const MAX_OUTPUT_BYTES: usize = 8 * 1024;
const MAX_ENTRIES: usize = 300;

/// Thư mục làm việc đã chuẩn hóa, dùng chung cho các tool Code.
#[derive(Debug, Clone)]
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, ToolError> {
        let root = root.as_ref();
        std::fs::create_dir_all(root).map_err(|error| {
            ToolError::Execution(format!(
                "không tạo được workspace {}: {error}",
                root.display()
            ))
        })?;
        let root = root.canonicalize().map_err(|error| {
            ToolError::Execution(format!(
                "không mở được workspace {}: {error}",
                root.display()
            ))
        })?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Ghép đường dẫn người dùng nhập với workspace và chặn thoát ra ngoài.
    fn resolve(&self, input: &str) -> Result<PathBuf, ToolError> {
        let raw = Path::new(input.trim());
        let joined = if raw.is_absolute() {
            raw.to_path_buf()
        } else {
            self.root.join(raw)
        };
        let normalized = normalize(&joined);

        if !normalized.starts_with(&self.root) {
            return Err(ToolError::InvalidArgs(format!(
                "đường dẫn '{}' nằm ngoài workspace",
                input
            )));
        }

        // Chống symlink trỏ ra ngoài: canonicalize phần đã tồn tại.
        if let Ok(canonical) = normalized.canonicalize() {
            if !canonical.starts_with(&self.root) {
                return Err(ToolError::InvalidArgs(format!(
                    "đường dẫn '{}' trỏ ra ngoài workspace",
                    input
                )));
            }
            return Ok(canonical);
        }

        Ok(normalized)
    }

    fn display(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .map(|value| {
                if value.as_os_str().is_empty() {
                    ".".to_string()
                } else {
                    value.display().to_string()
                }
            })
            .unwrap_or_else(|_| path.display().to_string())
    }
}

/// Chuẩn hóa `.`/`..` mà không cần đường dẫn tồn tại.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn required_str<'a>(args: &'a Value, field: &str) -> Result<&'a str, ToolError> {
    args.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::InvalidArgs(format!("thiếu trường '{field}'")))
}

fn optional_str<'a>(args: &'a Value, field: &str) -> Option<&'a str> {
    args.get(field).and_then(Value::as_str)
}

fn cap_text(text: &mut String) {
    if text.len() <= MAX_OUTPUT_BYTES {
        return;
    }
    let mut end = MAX_OUTPUT_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text.push_str("\n… (đã cắt bớt output)");
}

/// Đọc nội dung file dạng text.
pub struct ReadFile {
    workspace: Workspace,
}

impl ReadFile {
    pub fn new(workspace: Workspace) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl Tool for ReadFile {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read_file".into(),
            description: "Đọc một file text trong workspace. Luôn dùng trước khi sửa file. Có thể truyền offset/limit để đọc một đoạn theo dòng.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Đường dẫn tương đối trong workspace" },
                    "offset": { "type": "integer", "minimum": 1, "description": "Dòng bắt đầu (1-based)" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 2000, "description": "Số dòng tối đa" }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let path = self.workspace.resolve(required_str(&args, "path")?)?;
        let metadata = match tokio::fs::metadata(&path).await {
            Ok(metadata) => metadata,
            Err(error) => return Ok(ToolOutput::error(format!("không đọc được file: {error}"))),
        };
        if metadata.is_dir() {
            return Ok(ToolOutput::error(
                "đây là thư mục; dùng list_files để xem nội dung",
            ));
        }
        if metadata.len() > MAX_READ_BYTES {
            return Ok(ToolOutput::error(format!(
                "file quá lớn ({} byte, giới hạn {} byte)",
                metadata.len(),
                MAX_READ_BYTES
            )));
        }

        match tokio::fs::read_to_string(&path).await {
            Ok(content) => {
                let offset = args
                    .get("offset")
                    .and_then(Value::as_u64)
                    .unwrap_or(1)
                    .max(1) as usize;
                let limit = args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .map(|value| value.clamp(1, 2_000) as usize);
                if offset == 1 && limit.is_none() {
                    return Ok(ToolOutput::ok(content));
                }
                let lines: Vec<&str> = content.lines().collect();
                if offset > lines.len() && !lines.is_empty() {
                    return Ok(ToolOutput::error(format!(
                        "offset {offset} vượt quá số dòng ({})",
                        lines.len()
                    )));
                }
                let end = limit
                    .map(|limit| (offset - 1 + limit).min(lines.len()))
                    .unwrap_or(lines.len());
                let slice = lines
                    .get(offset.saturating_sub(1)..end)
                    .unwrap_or_default()
                    .join("\n");
                Ok(ToolOutput::ok(format!(
                    "[dòng {offset}-{end}/{}]\n{slice}",
                    lines.len()
                )))
            }
            Err(error) => Ok(ToolOutput::error(format!(
                "không đọc được file dạng text: {error}"
            ))),
        }
    }
}

/// Ghi đè nội dung file, tạo thư mục cha nếu cần.
pub struct WriteFile {
    workspace: Workspace,
}

impl WriteFile {
    pub fn new(workspace: Workspace) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl Tool for WriteFile {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "write_file".into(),
            description:
                "Ghi toàn bộ nội dung vào một file trong workspace, tạo thư mục cha nếu cần.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Đường dẫn tương đối trong workspace" },
                    "content": { "type": "string", "description": "Toàn bộ nội dung file sau khi ghi" }
                },
                "required": ["path", "content"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let path = self.workspace.resolve(required_str(&args, "path")?)?;
        let content = required_str(&args, "content")?;

        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|error| {
                ToolError::Execution(format!(
                    "không tạo được thư mục {}: {error}",
                    parent.display()
                ))
            })?;
        }

        match tokio::fs::write(&path, content).await {
            Ok(()) => Ok(ToolOutput::ok(format!(
                "đã ghi {} byte vào {}",
                content.len(),
                self.workspace.display(&path)
            ))),
            Err(error) => Ok(ToolOutput::error(format!("không ghi được file: {error}"))),
        }
    }
}

/// Thay thế chính xác một đoạn text trong file; bắt buộc khớp duy nhất.
pub struct EditFile {
    workspace: Workspace,
}

impl EditFile {
    pub fn new(workspace: Workspace) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl Tool for EditFile {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "edit_file".into(),
            description: "Thay một đoạn text chính xác trong file. Mặc định đoạn cũ phải xuất hiện đúng một lần; dùng replace_all nếu muốn thay mọi lần. An toàn hơn write_file vì không ghi đè cả file."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Đường dẫn tương đối trong workspace" },
                    "old_string": { "type": "string", "description": "Đoạn text cũ cần thay, giữ nguyên thụt lề" },
                    "new_string": { "type": "string", "description": "Đoạn text mới" },
                    "replace_all": { "type": "boolean", "description": "Thay mọi lần xuất hiện" }
                },
                "required": ["path", "old_string", "new_string"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let path = self.workspace.resolve(required_str(&args, "path")?)?;
        let old_string = required_str(&args, "old_string")?;
        let new_string = required_str(&args, "new_string")?;
        let replace_all = args
            .get("replace_all")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        if old_string.is_empty() {
            return Ok(ToolOutput::error("old_string không được rỗng"));
        }
        let metadata = match tokio::fs::metadata(&path).await {
            Ok(metadata) => metadata,
            Err(error) => return Ok(ToolOutput::error(format!("không đọc được file: {error}"))),
        };
        if metadata.len() > MAX_READ_BYTES {
            return Ok(ToolOutput::error(format!(
                "file quá lớn để sửa an toàn ({} byte, giới hạn {} byte)",
                metadata.len(),
                MAX_READ_BYTES
            )));
        }

        let content = match tokio::fs::read_to_string(&path).await {
            Ok(content) => content,
            Err(error) => {
                return Ok(ToolOutput::error(format!(
                    "không đọc được file dạng text: {error}"
                )))
            }
        };
        let count = content.matches(old_string).count();
        if count == 0 {
            return Ok(ToolOutput::error(
                "không tìm thấy old_string trong file; hãy đọc lại file và dùng đúng ngữ cảnh",
            ));
        }
        if count > 1 && !replace_all {
            return Ok(ToolOutput::error(format!(
                "old_string xuất hiện {count} lần; thêm ngữ cảnh để khớp duy nhất hoặc đặt replace_all=true"
            )));
        }

        let updated = if replace_all {
            content.replace(old_string, new_string)
        } else {
            content.replacen(old_string, new_string, 1)
        };
        match tokio::fs::write(&path, updated).await {
            Ok(()) => Ok(ToolOutput::ok(format!(
                "đã thay {count} lần trong {}",
                self.workspace.display(&path)
            ))),
            Err(error) => Ok(ToolOutput::error(format!("không ghi được file: {error}"))),
        }
    }
}

/// Liệt kê file/thư mục trong workspace, bỏ qua thư mục sinh build.
pub struct ListFiles {
    workspace: Workspace,
}

impl ListFiles {
    pub fn new(workspace: Workspace) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl Tool for ListFiles {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "list_files".into(),
            description: "Liệt kê file và thư mục trong workspace (tối đa 3 tầng, bỏ node_modules/target/.git)."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Thư mục bắt đầu, mặc định '.'" },
                    "depth": { "type": "integer", "minimum": 1, "maximum": 4 }
                },
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let start = self
            .workspace
            .resolve(optional_str(&args, "path").unwrap_or("."))?;
        if !start.is_dir() {
            return Ok(ToolOutput::error("đường dẫn không phải thư mục"));
        }
        let depth = args
            .get("depth")
            .and_then(Value::as_u64)
            .unwrap_or(2)
            .clamp(1, 4) as usize;

        let mut lines = Vec::new();
        walk(&self.workspace, &start, depth, &mut lines);
        if lines.is_empty() {
            return Ok(ToolOutput::ok("(thư mục rỗng)"));
        }
        let mut text = lines.join("\n");
        cap_text(&mut text);
        Ok(ToolOutput::ok(text))
    }
}

fn walk(workspace: &Workspace, dir: &Path, depth: usize, out: &mut Vec<String>) {
    if out.len() >= MAX_ENTRIES {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    let mut items: Vec<_> = entries.filter_map(Result::ok).collect();
    items.sort_by_key(|entry| entry.file_name());

    for entry in items {
        if out.len() >= MAX_ENTRIES {
            return;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if matches!(
            name.as_str(),
            ".git" | "node_modules" | "target" | "dist" | ".venv" | "__pycache__"
        ) {
            continue;
        }
        let path = entry.path();
        let is_dir = path.is_dir();
        let mut line = workspace.display(&path);
        if is_dir {
            line.push('/');
        }
        out.push(line);
        if is_dir && depth > 1 {
            walk(workspace, &path, depth - 1, out);
        }
    }
}

/// Tìm file theo glob pattern, ví dụ `src/**/*.rs`.
pub struct GlobFiles {
    workspace: Workspace,
}

impl GlobFiles {
    pub fn new(workspace: Workspace) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl Tool for GlobFiles {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "glob".into(),
            description: "Tìm file trong workspace theo glob pattern (`*`, `**`, `?`, `[...]`), ví dụ `src/**/*.ts`. Bỏ qua node_modules/target/.git."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Glob pattern tương đối với workspace" },
                    "path": { "type": "string", "description": "Thư mục bắt đầu, mặc định '.'" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 300 }
                },
                "required": ["pattern"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let pattern_text = required_str(&args, "pattern")?;
        let pattern = glob::Pattern::new(pattern_text)
            .map_err(|error| ToolError::InvalidArgs(format!("glob không hợp lệ: {error}")))?;
        let base = self
            .workspace
            .resolve(optional_str(&args, "path").unwrap_or("."))?;
        if !base.is_dir() {
            return Ok(ToolOutput::error("path không phải thư mục"));
        }
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(200)
            .clamp(1, 300) as usize;

        let mut files = Vec::new();
        walk_files(&base, 0, &mut files);
        let mut matches: Vec<String> = files
            .into_iter()
            .filter_map(|path| {
                let rel = path
                    .strip_prefix(self.workspace.root())
                    .ok()?
                    .to_string_lossy()
                    .replace('\\', "/");
                let file_name = path.file_name()?.to_string_lossy();
                (pattern.matches(&rel) || pattern.matches(file_name.as_ref())).then_some(rel)
            })
            .collect();
        matches.sort();
        matches.dedup();
        if matches.is_empty() {
            return Ok(ToolOutput::ok("(không có file khớp)"));
        }
        let total = matches.len();
        matches.truncate(limit);
        let mut text = matches.join("\n");
        if total > limit {
            text.push_str(&format!(
                "\n… (còn {} file, tăng limit để xem thêm)",
                total - limit
            ));
        }
        cap_text(&mut text);
        Ok(ToolOutput::ok(text))
    }
}

/// Tìm text trong file bằng regex.
pub struct GrepFiles {
    workspace: Workspace,
}

impl GrepFiles {
    pub fn new(workspace: Workspace) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl Tool for GrepFiles {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "grep".into(),
            description: "Tìm dòng khớp regex trong các file text của workspace. Trả về `đường_dẫn:dòng: nội_dung`. Dùng để định vị symbol trước khi đọc file."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Regex cần tìm" },
                    "path": { "type": "string", "description": "Thư mục/file bắt đầu, mặc định '.'" },
                    "include": { "type": "string", "description": "Glob lọc file, ví dụ `*.rs` hoặc `src/**/*.ts`" },
                    "ignore_case": { "type": "boolean", "description": "Không phân biệt hoa thường" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 300 }
                },
                "required": ["pattern"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let pattern_text = required_str(&args, "pattern")?;
        let ignore_case = args
            .get("ignore_case")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let regex = regex::RegexBuilder::new(pattern_text)
            .case_insensitive(ignore_case)
            .build()
            .map_err(|error| ToolError::InvalidArgs(format!("regex không hợp lệ: {error}")))?;
        let include = args
            .get("include")
            .and_then(Value::as_str)
            .map(glob::Pattern::new)
            .transpose()
            .map_err(|error| {
                ToolError::InvalidArgs(format!("include glob không hợp lệ: {error}"))
            })?;
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(100)
            .clamp(1, 300) as usize;

        let base = self
            .workspace
            .resolve(optional_str(&args, "path").unwrap_or("."))?;
        let mut files = Vec::new();
        if base.is_file() {
            files.push(base);
        } else if base.is_dir() {
            walk_files(&base, 0, &mut files);
        } else {
            return Ok(ToolOutput::error("path không tồn tại"));
        }

        let mut lines = Vec::new();
        for file in files {
            if lines.len() >= limit {
                break;
            }
            let rel = file
                .strip_prefix(self.workspace.root())
                .unwrap_or(&file)
                .to_string_lossy()
                .replace('\\', "/");
            if let Some(include) = &include {
                let file_name = file.file_name().map(|value| value.to_string_lossy());
                let matched = include.matches(&rel)
                    || file_name
                        .as_deref()
                        .map(|name| include.matches(name))
                        .unwrap_or(false);
                if !matched {
                    continue;
                }
            }
            let Ok(metadata) = std::fs::metadata(&file) else {
                continue;
            };
            if metadata.len() > 512 * 1024 {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&file) else {
                continue;
            };
            for (index, line) in content.lines().enumerate() {
                if lines.len() >= limit {
                    break;
                }
                if regex.is_match(line) {
                    let mut preview: String = line.trim().chars().take(240).collect();
                    if line.trim().chars().count() > 240 {
                        preview.push('…');
                    }
                    lines.push(format!("{rel}:{}: {preview}", index + 1));
                }
            }
        }

        if lines.is_empty() {
            return Ok(ToolOutput::ok("(không có dòng khớp)"));
        }
        let mut text = lines.join("\n");
        cap_text(&mut text);
        Ok(ToolOutput::ok(text))
    }
}

/// Duyệt file text trong thư mục, bỏ thư mục build và giới hạn số lượng.
fn walk_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 12 || out.len() >= 5_000 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut items: Vec<_> = entries.filter_map(Result::ok).collect();
    items.sort_by_key(|entry| entry.file_name());
    for entry in items {
        if out.len() >= 5_000 {
            return;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if matches!(
                name.as_str(),
                ".git" | "node_modules" | "target" | "dist" | ".venv" | "__pycache__"
            ) {
                continue;
            }
            walk_files(&path, depth + 1, out);
        } else {
            out.push(path);
        }
    }
}

/// Chạy một lệnh shell trong workspace.
pub struct RunCommand {
    workspace: Workspace,
}

impl RunCommand {
    pub fn new(workspace: Workspace) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl Tool for RunCommand {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "run_command".into(),
            description: "Chạy lệnh shell trong workspace và trả về exit code, stdout, stderr. Dùng để build/test."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "Lệnh cần chạy" },
                    "cwd": { "type": "string", "description": "Thư mục con để chạy lệnh" },
                    "timeout_secs": { "type": "integer", "minimum": 1, "maximum": 120 }
                },
                "required": ["command"],
                "additionalProperties": false
            }),
        }
    }

    fn timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(130)
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let command = required_str(&args, "command")?;
        if command.trim().is_empty() {
            return Err(ToolError::InvalidArgs("lệnh rỗng".into()));
        }
        let cwd = self
            .workspace
            .resolve(optional_str(&args, "cwd").unwrap_or("."))?;
        if !cwd.is_dir() {
            return Ok(ToolOutput::error("cwd không phải thư mục"));
        }
        let timeout_secs = args
            .get("timeout_secs")
            .and_then(Value::as_u64)
            .unwrap_or(20)
            .clamp(1, 120);

        #[cfg(windows)]
        let mut process = {
            let mut process = tokio::process::Command::new("cmd");
            process.args(["/C", command]);
            process
        };
        #[cfg(not(windows))]
        let mut process = {
            let mut process = tokio::process::Command::new("sh");
            process.args(["-c", command]);
            process
        };

        process
            .current_dir(&cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let child = process
            .spawn()
            .map_err(|error| ToolError::Execution(format!("không chạy được lệnh: {error}")))?;

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            child.wait_with_output(),
        )
        .await;

        match result {
            Ok(Ok(output)) => {
                let mut text = format!("exit code: {}", output.status.code().unwrap_or(-1));
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                if !stdout.trim().is_empty() {
                    text.push_str("\n--- stdout ---\n");
                    text.push_str(stdout.trim_end());
                }
                if !stderr.trim().is_empty() {
                    text.push_str("\n--- stderr ---\n");
                    text.push_str(stderr.trim_end());
                }
                if stdout.trim().is_empty() && stderr.trim().is_empty() {
                    text.push_str("\n(không có output)");
                }
                cap_text(&mut text);
                if output.status.success() {
                    Ok(ToolOutput::ok(text))
                } else {
                    Ok(ToolOutput::error(text))
                }
            }
            Ok(Err(error)) => Ok(ToolOutput::error(format!("lỗi khi chờ lệnh: {error}"))),
            Err(_) => Ok(ToolOutput::error(format!(
                "lệnh vượt quá {timeout_secs}s và đã bị dừng"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> (tempfile::TempDir, Workspace) {
        let dir = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        (dir, workspace)
    }

    #[tokio::test]
    async fn writes_then_reads_file() {
        let (_dir, workspace) = workspace();
        let write = WriteFile::new(workspace.clone());
        let output = write
            .execute(json!({"path": "src/main.rs", "content": "fn main() {}"}))
            .await
            .unwrap();
        assert!(!output.is_error);

        let read = ReadFile::new(workspace);
        let output = read.execute(json!({"path": "src/main.rs"})).await.unwrap();
        assert_eq!(output.content, "fn main() {}");
    }

    #[tokio::test]
    async fn blocks_path_traversal() {
        let (_dir, workspace) = workspace();
        let read = ReadFile::new(workspace);
        let output = read.execute(json!({"path": "../../etc/passwd"})).await;
        assert!(output.is_err());
    }

    #[tokio::test]
    async fn lists_files_and_skips_noise() {
        let (_dir, workspace) = workspace();
        std::fs::create_dir_all(workspace.root().join("src")).unwrap();
        std::fs::create_dir_all(workspace.root().join("node_modules")).unwrap();
        std::fs::write(workspace.root().join("src/lib.rs"), "").unwrap();

        let list = ListFiles::new(workspace);
        let output = list.execute(json!({"depth": 2})).await.unwrap();
        assert!(output.content.contains("src/"));
        assert!(output.content.contains("src/lib.rs"));
        assert!(!output.content.contains("node_modules"));
    }

    #[tokio::test]
    async fn runs_command_in_workspace() {
        let (_dir, workspace) = workspace();
        let tool = RunCommand::new(workspace);
        #[cfg(windows)]
        let command = "echo hello";
        #[cfg(not(windows))]
        let command = "printf hello";
        let output = tool.execute(json!({"command": command})).await.unwrap();
        assert!(!output.is_error, "{}", output.content);
        assert!(output.content.contains("hello"));
    }
}
