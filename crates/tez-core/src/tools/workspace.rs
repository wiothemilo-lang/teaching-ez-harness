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
            description: "Đọc một file text trong workspace. Luôn dùng trước khi sửa file.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Đường dẫn tương đối trong workspace" }
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
            Ok(content) => Ok(ToolOutput::ok(content)),
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
