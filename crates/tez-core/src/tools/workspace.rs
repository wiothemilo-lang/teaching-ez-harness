//! Tool cho chế độ Code: đọc/ghi file, liệt kê thư mục và chạy lệnh trong workspace.
//!
//! Mọi đường dẫn đều bị giới hạn trong workspace. Đây là lớp bảo vệ chính, không
//! phải sandbox của hệ điều hành: người dùng vẫn nên chạy tez bằng tài khoản
//! không có quyền cao.

use std::io::Read as _;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use serde_json::{json, Value};
use tez_llm::ToolSpec;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

use super::{Tool, ToolError, ToolOutput};

const MAX_READ_BYTES: u64 = 256 * 1024;
const MAX_OUTPUT_BYTES: usize = 8 * 1024;
const MAX_COMMAND_OUTPUT_BYTES: usize = 32 * 1024;
const MAX_ENTRIES: usize = 300;
const MAX_WALK_ENTRIES: usize = 50_000;
const MAX_GLOB_MATCHES: usize = 10_000;
static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(1);

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
    ///
    /// Không canonicalize cả đường dẫn vì file đích có thể chưa tồn tại. Thay
    /// vào đó canonicalize tổ tiên gần nhất còn tồn tại rồi ghép lại phần chưa
    /// tồn tại. Nhờ vậy symlink hỏng hoặc symlink trỏ ra ngoài đều bị chặn,
    /// thay vì âm thầm quay về đường dẫn chưa chuẩn hóa.
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

        let outside =
            || ToolError::InvalidArgs(format!("đường dẫn '{}' trỏ ra ngoài workspace", input));

        // Tìm tổ tiên gần nhất còn tồn tại, gom các thành phần chưa tồn tại.
        let mut existing = normalized.as_path();
        let mut missing: Vec<std::ffi::OsString> = Vec::new();
        loop {
            match existing.symlink_metadata() {
                Ok(metadata) => {
                    if metadata.file_type().is_symlink() {
                        // Symlink hỏng không canonicalize được: từ chối luôn,
                        // không được coi đường dẫn là hợp lệ.
                        let canonical = existing.canonicalize().map_err(|_| {
                            ToolError::InvalidArgs(format!(
                                "đường dẫn '{}' chứa liên kết hỏng",
                                input
                            ))
                        })?;
                        if !canonical.starts_with(&self.root) {
                            return Err(outside());
                        }
                        let mut resolved = canonical;
                        for part in missing.iter().rev() {
                            resolved.push(part);
                        }
                        return Ok(resolved);
                    }
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let name = existing.file_name().ok_or_else(&outside)?.to_os_string();
                    missing.push(name);
                    existing = existing.parent().ok_or_else(&outside)?;
                }
                Err(error) => {
                    return Err(ToolError::Execution(format!(
                        "không kiểm tra được đường dẫn '{}': {error}",
                        input
                    )))
                }
            }
        }

        let canonical = existing.canonicalize().map_err(|error| {
            ToolError::Execution(format!("không mở được đường dẫn '{}': {error}", input))
        })?;
        if !canonical.starts_with(&self.root) {
            return Err(outside());
        }
        let mut resolved = canonical;
        for part in missing.iter().rev() {
            resolved.push(part);
        }
        Ok(resolved)
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

/// Mở và đọc file text, chỉ chấp nhận file thường.
///
/// `O_NONBLOCK` quan trọng trên Unix: nếu đường dẫn bị đổi thành FIFO giữa
/// lúc kiểm tra và lúc mở, `open` vẫn trả về ngay thay vì treo worker.
async fn read_regular_text(path: &Path, max_bytes: u64) -> Result<String, String> {
    let mut options = tokio::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .await
        .map_err(|error| format!("không đọc được file: {error}"))?;
    let metadata = file
        .metadata()
        .await
        .map_err(|error| format!("không đọc được file: {error}"))?;
    if metadata.is_dir() {
        return Err("đây là thư mục; dùng list_files để xem nội dung".to_string());
    }
    if !metadata.is_file() {
        return Err("đây không phải file thường (FIFO/device/socket); từ chối đọc".to_string());
    }
    if metadata.len() > max_bytes {
        return Err(format!(
            "file quá lớn ({} byte, giới hạn {} byte)",
            metadata.len(),
            max_bytes
        ));
    }
    let mut bytes = Vec::new();
    file.take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| format!("không đọc được file: {error}"))?;
    if bytes.len() as u64 > max_bytes {
        return Err(format!("file quá lớn (giới hạn {} byte)", max_bytes));
    }
    String::from_utf8(bytes).map_err(|error| format!("không đọc được file dạng text: {error}"))
}

/// Bản sync của [`read_regular_text`] cho grep (chạy trên blocking pool).
fn read_regular_text_sync(path: &Path, max_bytes: u64) -> Option<String> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > max_bytes {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(max_bytes + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > max_bytes {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// Ghi file qua temp cùng thư mục rồi rename: không bao giờ truncate file gốc.
async fn write_atomic(path: &Path, content: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    let temp = parent.join(format!(
        ".{file_name}.tez-tmp-{}-{}",
        std::process::id(),
        NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed)
    ));
    let permissions = tokio::fs::metadata(path)
        .await
        .ok()
        .map(|metadata| metadata.permissions());

    let result = async {
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .await
            .map_err(|error| format!("không tạo được file tạm: {error}"))?;
        if let Some(permissions) = permissions {
            file.set_permissions(permissions)
                .await
                .map_err(|error| format!("không đặt được quyền file tạm: {error}"))?;
        }
        file.write_all(content.as_bytes())
            .await
            .map_err(|error| format!("không ghi được file: {error}"))?;
        file.sync_all()
            .await
            .map_err(|error| format!("không đồng bộ được file: {error}"))?;
        drop(file);
        tokio::fs::rename(&temp, path)
            .await
            .map_err(|error| format!("không thay được file: {error}"))
    }
    .await;

    if result.is_err() {
        let _ = tokio::fs::remove_file(&temp).await;
    }
    result
}

fn ensure_unchanged(before: &str, current: &str) -> Result<(), &'static str> {
    if before == current {
        Ok(())
    } else {
        Err("file đã thay đổi trong lúc sửa; hãy đọc lại file rồi thử lại")
    }
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
        let content = match read_regular_text(&path, MAX_READ_BYTES).await {
            Ok(content) => content,
            Err(error) => return Ok(ToolOutput::error(error)),
        };

        let offset = args
            .get("offset")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            .max(1);
        let offset = usize::try_from(offset).unwrap_or(usize::MAX);
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .map(|value| value.clamp(1, 2_000) as usize);
        if offset == 1 && limit.is_none() {
            return Ok(ToolOutput::ok(content));
        }

        let lines: Vec<&str> = content.lines().collect();
        if lines.is_empty() {
            return Ok(ToolOutput::ok("[file rỗng]"));
        }
        if offset > lines.len() {
            return Ok(ToolOutput::error(format!(
                "offset {offset} vượt quá số dòng ({})",
                lines.len()
            )));
        }
        let start = offset.saturating_sub(1);
        let end = limit
            .map(|limit| start.saturating_add(limit).min(lines.len()))
            .unwrap_or(lines.len());
        let slice = lines.get(start..end).unwrap_or_default().join("\n");
        Ok(ToolOutput::ok(format!(
            "[dòng {offset}-{end}/{}]\n{slice}",
            lines.len()
        )))
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

        match tokio::fs::metadata(&path).await {
            Ok(metadata) if metadata.is_dir() => {
                return Ok(ToolOutput::error("đây là thư mục; không thể ghi file"))
            }
            Ok(metadata) if !metadata.is_file() => {
                return Ok(ToolOutput::error(
                    "đường dẫn không phải file thường (FIFO/device/socket); từ chối ghi",
                ))
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Ok(ToolOutput::error(format!(
                    "không kiểm tra được file: {error}"
                )))
            }
        }

        match write_atomic(&path, content).await {
            Ok(()) => Ok(ToolOutput::ok(format!(
                "đã ghi {} byte vào {}",
                content.len(),
                self.workspace.display(&path)
            ))),
            Err(error) => Ok(ToolOutput::error(error)),
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
        let content = match read_regular_text(&path, MAX_READ_BYTES).await {
            Ok(content) => content,
            Err(error) => return Ok(ToolOutput::error(error)),
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
        // Đọc lại trước khi thay: nếu file đã đổi trong lúc ta tính toán,
        // không được ghi đè thay đổi của người khác.
        let current = match read_regular_text(&path, MAX_READ_BYTES).await {
            Ok(current) => current,
            Err(error) => return Ok(ToolOutput::error(error)),
        };
        if let Err(message) = ensure_unchanged(&content, &current) {
            return Ok(ToolOutput::error(message));
        }
        match write_atomic(&path, &updated).await {
            Ok(()) => Ok(ToolOutput::ok(format!(
                "đã thay {count} lần trong {}",
                self.workspace.display(&path)
            ))),
            Err(error) => Ok(ToolOutput::error(error)),
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
        // Không đi theo symlink: đích có thể nằm ngoài workspace.
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let is_dir = file_type.is_dir();
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

        // So khớp ngay trong lúc duyệt, không cắt 5000 file trước rồi mới
        // match (file khớp có thể nằm sau mốc cắt).
        let mut state = WalkState::default();
        let mut matches = Vec::new();
        let mut match_cap_hit = false;
        walk_files(&base, 0, &mut state, &mut |path| {
            let Some(rel) = path.strip_prefix(self.workspace.root()).ok() else {
                return true;
            };
            let rel = rel.to_string_lossy().replace('\\', "/");
            let Some(file_name) = path.file_name().map(|name| name.to_string_lossy()) else {
                return true;
            };
            if pattern.matches(&rel) || pattern.matches(file_name.as_ref()) {
                matches.push(rel);
                if matches.len() >= MAX_GLOB_MATCHES {
                    match_cap_hit = true;
                    return false;
                }
            }
            true
        });
        matches.sort();
        matches.dedup();
        if matches.is_empty() {
            let mut text = "(không có file khớp)".to_string();
            append_walk_truncation(&mut text, state.truncated);
            return Ok(ToolOutput::ok(text));
        }
        let total = matches.len();
        matches.truncate(limit);
        let mut text = matches.join("\n");
        if total > limit {
            text.push_str(&format!(
                "\n… (còn {} file khớp, tăng limit để xem thêm)",
                total - limit
            ));
        }
        if match_cap_hit {
            text.push_str(&format!(
                "\n… (đã dừng sau {MAX_GLOB_MATCHES} file khớp; kết quả có thể chưa đầy đủ)"
            ));
        }
        append_walk_truncation(&mut text, state.truncated);
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
        let mut state = WalkState::default();
        let mut files = Vec::new();
        match std::fs::metadata(&base) {
            Ok(metadata) if metadata.is_file() => files.push(base),
            Ok(metadata) if metadata.is_dir() => {
                walk_files(&base, 0, &mut state, &mut |path| {
                    files.push(path.to_path_buf());
                    true
                });
            }
            Ok(_) => {
                return Ok(ToolOutput::error(
                    "path không phải file thường (FIFO/device/socket)",
                ))
            }
            Err(_) => return Ok(ToolOutput::error("path không tồn tại")),
        }

        let mut lines = Vec::new();
        for file in files {
            if lines.len() >= limit {
                break;
            }
            // File ngoài workspace không được dán nhãn như file trong.
            let Ok(rel) = file.strip_prefix(self.workspace.root()) else {
                continue;
            };
            let rel = rel.to_string_lossy().replace('\\', "/");
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
            let Some(content) = read_regular_text_sync(&file, 512 * 1024) else {
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
            let mut text = "(không có dòng khớp)".to_string();
            append_walk_truncation(&mut text, state.truncated);
            return Ok(ToolOutput::ok(text));
        }
        let mut text = lines.join("\n");
        append_walk_truncation(&mut text, state.truncated);
        cap_text(&mut text);
        Ok(ToolOutput::ok(text))
    }
}

#[derive(Default)]
struct WalkState {
    visited: usize,
    truncated: bool,
    stopped: bool,
}

/// Duyệt file trong thư mục, bỏ thư mục build và gọi visitor cho từng file.
/// Visitor trả `false` để dừng sớm (ví dụ đã đủ số match).
fn walk_files<F>(dir: &Path, depth: usize, state: &mut WalkState, visit: &mut F)
where
    F: FnMut(&Path) -> bool,
{
    if depth > 12 || state.truncated || state.stopped {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut items: Vec<_> = entries.filter_map(Result::ok).collect();
    items.sort_by_key(|entry| entry.file_name());
    for entry in items {
        if state.truncated || state.stopped {
            return;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        // Không đi theo symlink: có thể dẫn tới file/thư mục ngoài workspace.
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            if matches!(
                name.as_str(),
                ".git" | "node_modules" | "target" | "dist" | ".venv" | "__pycache__"
            ) {
                continue;
            }
            walk_files(&path, depth + 1, state, visit);
        } else {
            if state.visited >= MAX_WALK_ENTRIES {
                state.truncated = true;
                return;
            }
            state.visited += 1;
            if !visit(&path) {
                state.stopped = true;
                return;
            }
        }
    }
}

fn append_walk_truncation(text: &mut String, truncated: bool) {
    if truncated {
        text.push_str(&format!(
            "\n… (đã quét tối đa {MAX_WALK_ENTRIES} file; kết quả có thể chưa đầy đủ)"
        ));
    }
}

/// Chạy một lệnh shell trong workspace.
pub struct RunCommand {
    workspace: Workspace,
    sandbox: Option<crate::sandbox::SandboxPolicy>,
}

impl RunCommand {
    pub fn new(workspace: Workspace) -> Self {
        Self {
            workspace,
            sandbox: None,
        }
    }

    /// Chạy lệnh trong sandbox OS (bubblewrap trên Linux).
    pub fn with_sandbox(workspace: Workspace, policy: crate::sandbox::SandboxPolicy) -> Self {
        Self {
            workspace,
            sandbox: Some(policy),
        }
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
            if let Some(policy) = self.sandbox.as_ref().filter(|policy| policy.enabled) {
                crate::sandbox::build_command(policy, self.workspace.root(), &cwd, command)
                    .map_err(ToolError::Execution)?
            } else {
                let mut process = tokio::process::Command::new("sh");
                process.args(["-c", command]);
                process
            }
        };

        process
            .current_dir(&cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        // Đưa cả cây lệnh vào một process group để timeout có thể dừng cả process cháu.
        #[cfg(unix)]
        process.process_group(0);

        let mut child = process
            .spawn()
            .map_err(|error| ToolError::Execution(format!("không chạy được lệnh: {error}")))?;
        let pid = child.id();
        // Nếu agent huỷ giữa chừng, future bị drop và `kill_on_drop` chỉ dừng
        // process trực tiếp. Guard này dừng cả process group (process cháu).
        let mut tree_guard = ProcessTreeGuard::new(pid);
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ToolError::Execution("không bắt được stdout".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| ToolError::Execution("không bắt được stderr".into()))?;
        let stdout_task = tokio::spawn(read_capped(stdout));
        let stderr_task = tokio::spawn(read_capped(stderr));

        let result =
            tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), child.wait()).await;

        match result {
            Ok(Ok(status)) => {
                // Process đã được reap; không giữ pid nữa để tránh kill nhầm
                // process khác nếu pid được tái sử dụng.
                tree_guard.disarm();
                // Đọc stdout/stderr song song. Process cháu có thể giữ một
                // trong hai pipe mở; nếu một reader timeout thì phải dừng cả
                // cây lệnh, không chỉ bỏ qua reader đó.
                let (stdout_result, stderr_result) =
                    tokio::join!(join_reader(stdout_task), join_reader(stderr_task));
                if stdout_result.is_none() || stderr_result.is_none() {
                    if let Some(pid) = pid {
                        kill_process_tree(pid);
                    }
                }
                let (stdout, stdout_total) = stdout_result.unwrap_or_default();
                let (stderr, stderr_total) = stderr_result.unwrap_or_default();
                let mut text = format!("exit code: {}", status.code().unwrap_or(-1));
                if stdout_total > stdout.len() {
                    text.push_str(&format!(
                        "\n… (stdout bị cắt: {} / {} byte)",
                        stdout.len(),
                        stdout_total
                    ));
                }
                if stderr_total > stderr.len() {
                    text.push_str(&format!(
                        "\n… (stderr bị cắt: {} / {} byte)",
                        stderr.len(),
                        stderr_total
                    ));
                }
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
                if status.success() {
                    Ok(ToolOutput::ok(text))
                } else {
                    Ok(ToolOutput::error(text))
                }
            }
            Ok(Err(error)) => {
                tree_guard.kill();
                Ok(ToolOutput::error(format!("lỗi khi chờ lệnh: {error}")))
            }
            Err(_) => {
                tree_guard.kill();
                let _ = child.kill().await;
                let _ = tokio::time::timeout(std::time::Duration::from_secs(2), async {
                    let _ = stdout_task.await;
                    let _ = stderr_task.await;
                })
                .await;
                Ok(ToolOutput::error(format!(
                    "lệnh vượt quá {timeout_secs}s và đã bị dừng"
                )))
            }
        }
    }
}

/// Đọc tối đa `MAX_COMMAND_OUTPUT_BYTES`, vẫn drain hết pipe để process không bị chặn.
async fn read_capped<R>(mut reader: R) -> (String, usize)
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let mut kept = Vec::new();
    let mut total = 0usize;
    let mut buffer = [0u8; 8192];
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                total += read;
                let room = MAX_COMMAND_OUTPUT_BYTES.saturating_sub(kept.len());
                if room > 0 {
                    kept.extend_from_slice(&buffer[..read.min(room)]);
                }
            }
        }
    }
    (String::from_utf8_lossy(&kept).into_owned(), total)
}

/// Process cháu có thể giữ pipe mở; không chờ nó vô hạn sau khi lệnh chính đã xong.
async fn join_reader(handle: tokio::task::JoinHandle<(String, usize)>) -> Option<(String, usize)> {
    match tokio::time::timeout(std::time::Duration::from_secs(2), handle).await {
        Ok(Ok(value)) => Some(value),
        _ => None,
    }
}

/// Dừng cả process group khi future bị huỷ (agent cancel hoặc timeout ngoài).
struct ProcessTreeGuard {
    pid: Option<u32>,
}

impl ProcessTreeGuard {
    fn new(pid: Option<u32>) -> Self {
        Self { pid }
    }

    /// Process đã được reap: không kill nữa để tránh pid được tái sử dụng.
    fn disarm(&mut self) {
        self.pid = None;
    }

    fn kill(&mut self) {
        if let Some(pid) = self.pid.take() {
            kill_process_tree(pid);
        }
    }
}

impl Drop for ProcessTreeGuard {
    fn drop(&mut self) {
        self.kill();
    }
}

#[cfg(unix)]
fn kill_process_tree(pid: u32) {
    // Child được spawn với process_group(0), nên pid cũng là pgid.
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
}

#[cfg(windows)]
fn kill_process_tree(pid: u32) {
    let _ = std::process::Command::new("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
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

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn sandbox_hides_app_data_dir() {
        if crate::sandbox::probe().is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let root = data.join("workspaces/default");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(data.join("master.key"), "SUPER_SECRET").unwrap();
        std::fs::write(root.join("visible.txt"), "VISIBLE").unwrap();

        let workspace = Workspace::new(&root).unwrap();
        let policy = crate::sandbox::SandboxPolicy {
            enabled: true,
            allow_network: false,
            data_dir: data.clone(),
        };
        let tool = RunCommand::with_sandbox(workspace, policy);
        let output = tool
            .execute(json!({
                "command": format!(
                    "cat '{}' 2>&1; cat '{}'",
                    data.join("master.key").display(),
                    root.join("visible.txt").display()
                )
            }))
            .await
            .unwrap();
        assert!(
            !output.content.contains("SUPER_SECRET"),
            "{}",
            output.content
        );
        assert!(output.content.contains("VISIBLE"), "{}", output.content);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancelling_command_kills_child_processes() {
        let (dir, workspace) = workspace();
        let marker = dir.path().join("child-survived");
        // Subshell giữ một process cháu sống lâu hơn shell trực tiếp. Nếu chỉ
        // shell bị kill, subshell vẫn tạo marker sau khi sleep kết thúc.
        let command = format!("(sleep 1; touch '{}') & wait", marker.display());
        let tool = RunCommand::new(workspace);
        let handle = tokio::spawn(async move {
            let _ = tool
                .execute(json!({"command": command, "timeout_secs": 30}))
                .await;
        });

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        handle.abort();
        let _ = handle.await;

        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        assert!(
            !marker.exists(),
            "process cháu vẫn chạy sau khi lệnh bị huỷ"
        );
    }

    #[cfg(unix)]
    fn make_symlink(target: &Path, link: &Path) {
        std::os::unix::fs::symlink(target, link).unwrap();
    }

    #[cfg(unix)]
    fn make_fifo(path: &Path) {
        let status = std::process::Command::new("mkfifo")
            .arg(path)
            .status()
            .expect("không chạy được mkfifo");
        assert!(status.success());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn blocks_write_through_dangling_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(dir.path().join("ws")).unwrap();
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        make_symlink(&outside.join("missing.txt"), &workspace.root().join("link"));

        let write = WriteFile::new(workspace);
        let output = write
            .execute(json!({"path": "link", "content": "escaped"}))
            .await;
        assert!(output.is_err(), "ghi qua symlink hỏng phải bị chặn");
        assert!(!outside.join("missing.txt").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn blocks_write_through_symlinked_directory() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(dir.path().join("ws")).unwrap();
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        make_symlink(&outside, &workspace.root().join("ext"));

        let write = WriteFile::new(workspace);
        let output = write
            .execute(json!({"path": "ext/new/f.txt", "content": "escaped"}))
            .await;
        assert!(output.is_err(), "ghi qua thư mục symlink phải bị chặn");
        assert!(!outside.join("new/f.txt").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn blocks_reading_symlink_outside() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(dir.path().join("ws")).unwrap();
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), "TOPSECRET").unwrap();
        make_symlink(
            &outside.join("secret.txt"),
            &workspace.root().join("secret.txt"),
        );

        let read = ReadFile::new(workspace);
        let output = read.execute(json!({"path": "secret.txt"})).await;
        assert!(output.is_err(), "đọc symlink ra ngoài phải bị chặn");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn walk_does_not_follow_symlinked_directory() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(dir.path().join("ws")).unwrap();
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), "TOPSECRET").unwrap();
        make_symlink(&outside, &workspace.root().join("ext"));

        let list = ListFiles::new(workspace.clone());
        let output = list.execute(json!({"depth": 3})).await.unwrap();
        assert!(!output.content.contains("secret.txt"));

        let grep = GrepFiles::new(workspace);
        let output = grep.execute(json!({"pattern": "TOPSECRET"})).await.unwrap();
        assert_eq!(output.content, "(không có dòng khớp)");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn allows_symlink_that_stays_inside_workspace() {
        let (_dir, workspace) = workspace();
        std::fs::write(workspace.root().join("real.txt"), "hello").unwrap();
        make_symlink(
            &workspace.root().join("real.txt"),
            &workspace.root().join("alias.txt"),
        );

        let read = ReadFile::new(workspace);
        let output = read.execute(json!({"path": "alias.txt"})).await.unwrap();
        assert_eq!(output.content, "hello");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn read_file_rejects_fifo_instead_of_blocking() {
        let (dir, workspace) = workspace();
        let fifo = dir.path().join("pipe");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("không chạy được mkfifo");
        assert!(status.success());

        let read = ReadFile::new(workspace);
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            read.execute(json!({"path": "pipe"})),
        )
        .await
        .expect("đọc FIFO không được treo")
        .unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("file thường"), "{}", output.content);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn caps_large_command_output() {
        let (_dir, workspace) = workspace();
        let command = RunCommand::new(workspace);
        let output = command
            .execute(json!({"command": "yes x | head -c 200000", "timeout_secs": 10}))
            .await
            .unwrap();
        assert!(!output.is_error, "{}", output.content);
        assert!(output.content.len() < 20_000, "output phải được cắt");
        assert!(output.content.contains("đã cắt bớt output"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_stops_background_children() {
        let (_dir, workspace) = workspace();
        let command = RunCommand::new(workspace);
        let started = std::time::Instant::now();
        let output = command
            .execute(json!({"command": "sleep 30 & wait", "timeout_secs": 1}))
            .await
            .unwrap();
        assert!(output.is_error);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(8),
            "timeout phải dừng cả cây lệnh, không chờ sleep 30"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stderr_reader_timeout_kills_background_child() {
        let (dir, workspace) = workspace();
        let command = RunCommand::new(workspace);
        let output = command
            .execute(json!({
                "command": "(sleep 3; echo alive > marker) 1>/dev/null & echo $! > child.pid",
                "timeout_secs": 10
            }))
            .await
            .unwrap();
        assert!(!output.is_error, "{}", output.content);
        assert!(dir.path().join("child.pid").exists());

        // Reader stderr timeout sau 2s; process con chỉ ghi marker ở giây thứ 3.
        // Chờ dư 1s để không đua với lúc process con thức dậy.
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        assert!(
            !dir.path().join("marker").exists(),
            "process con giữ stderr phải bị kill, không được sống tới lúc ghi marker"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn write_file_rejects_fifo() {
        use std::os::unix::fs::FileTypeExt;

        let (dir, workspace) = workspace();
        let fifo = dir.path().join("pipe");
        make_fifo(&fifo);

        let write = WriteFile::new(workspace);
        let output = write
            .execute(json!({"path": "pipe", "content": "x"}))
            .await
            .unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("file thường"), "{}", output.content);
        assert!(std::fs::symlink_metadata(&fifo)
            .unwrap()
            .file_type()
            .is_fifo());
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn edit_file_rejects_fifo_without_blocking() {
        let (dir, workspace) = workspace();
        let fifo = dir.path().join("pipe");
        make_fifo(&fifo);
        // Giữ một đầu O_RDWR để nếu guard bị bỏ, reader mở được FIFO rồi
        // block thay vì treo ở open().
        let keep_open = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&fifo)
            .unwrap();

        let edit = EditFile::new(workspace);
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            edit.execute(json!({"path": "pipe", "old_string": "a", "new_string": "b"})),
        )
        .await;
        drop(keep_open);

        let output = result.expect("edit_file không được mở FIFO").unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("file thường"), "{}", output.content);
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn grep_skips_fifo_without_blocking() {
        let (dir, workspace) = workspace();
        let fifo = dir.path().join("pipe");
        make_fifo(&fifo);
        let keep_open = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&fifo)
            .unwrap();

        let grep = GrepFiles::new(workspace);
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            grep.execute(json!({"pattern": "MATCH"})),
        )
        .await;
        drop(keep_open);

        let output = result.expect("grep không được mở FIFO").unwrap();
        assert_eq!(output.content, "(không có dòng khớp)");
    }

    #[tokio::test]
    async fn write_file_replaces_content_and_leaves_no_temp_files() {
        let (dir, workspace) = workspace();
        let write = WriteFile::new(workspace);
        write
            .execute(json!({"path": "a.txt", "content": "one"}))
            .await
            .unwrap();
        write
            .execute(json!({"path": "a.txt", "content": "two"}))
            .await
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "two"
        );
        let leftovers: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.contains("tez-tmp"))
            .collect();
        assert!(leftovers.is_empty(), "còn file tạm: {leftovers:?}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn write_file_preserves_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let (dir, workspace) = workspace();
        let path = dir.path().join("script.sh");
        std::fs::write(&path, "old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();

        let write = WriteFile::new(workspace);
        write
            .execute(json!({"path": "script.sh", "content": "new"}))
            .await
            .unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755);
    }

    #[test]
    fn edit_verification_detects_concurrent_change() {
        assert!(ensure_unchanged("same", "same").is_ok());
        assert!(ensure_unchanged("before", "after").is_err());
    }

    #[tokio::test]
    async fn glob_matches_files_beyond_old_walk_cap() {
        let (dir, workspace) = workspace();
        for index in 0..5_005 {
            std::fs::write(dir.path().join(format!("f{index:05}.txt")), "").unwrap();
        }
        std::fs::write(dir.path().join("zzz-target.rs"), "").unwrap();

        let glob = GlobFiles::new(workspace);
        let output = glob
            .execute(json!({"pattern": "*.rs", "limit": 10}))
            .await
            .unwrap();
        assert!(!output.is_error);
        assert!(
            output.content.contains("zzz-target.rs"),
            "{}",
            output.content
        );
    }

    #[tokio::test]
    async fn glob_reports_result_truncation() {
        let (dir, workspace) = workspace();
        for index in 0..350 {
            std::fs::write(dir.path().join(format!("f{index:05}.txt")), "").unwrap();
        }

        let glob = GlobFiles::new(workspace);
        let output = glob
            .execute(json!({"pattern": "*.txt", "limit": 300}))
            .await
            .unwrap();
        assert!(
            output.content.contains("còn 50 file khớp"),
            "{}",
            output.content
        );
    }

    #[tokio::test]
    async fn read_file_handles_empty_file_with_huge_offset() {
        let (_dir, workspace) = workspace();
        std::fs::write(workspace.root().join("empty.txt"), "").unwrap();

        let read = ReadFile::new(workspace);
        let output = read
            .execute(json!({"path": "empty.txt", "offset": u64::MAX, "limit": 2000}))
            .await
            .unwrap();
        assert!(!output.is_error, "{}", output.content);
        assert!(output.content.contains("rỗng"), "{}", output.content);
    }

    #[tokio::test]
    async fn read_file_rejects_offset_past_end() {
        let (_dir, workspace) = workspace();
        std::fs::write(workspace.root().join("one.txt"), "one\n").unwrap();

        let read = ReadFile::new(workspace);
        let output = read
            .execute(json!({"path": "one.txt", "offset": u64::MAX}))
            .await
            .unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("vượt quá số dòng"));
    }
}
