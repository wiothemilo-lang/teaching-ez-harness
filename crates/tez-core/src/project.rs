//! Ngữ cảnh theo dự án: chỉ dẫn đặt cạnh workspace.
//!
//! Thứ tự đọc tại thư mục gốc của workspace:
//! `AGENTS.md`, `CLAUDE.md`, `.tez/AGENTS.md`, `.tez/CLAUDE.md`.
//! Nội dung được gộp vào system prompt như chỉ dẫn của dự án; yêu cầu trực
//! tiếp của người dùng vẫn được ưu tiên khi mâu thuẫn.
//!
//! Skill và MCP theo dự án dùng thư mục `.tez/` (xem [`skills`](crate::skills)
//! và [`mcp`](crate::mcp)).

use std::path::{Path, PathBuf};

/// Tổng dung lượng tối đa của chỉ dẫn dự án được nạp (byte).
pub const MAX_INSTRUCTIONS_BYTES: usize = 24 * 1024;

const INSTRUCTION_FILES: [&str; 4] = ["AGENTS.md", "CLAUDE.md", ".tez/AGENTS.md", ".tez/CLAUDE.md"];

#[derive(Debug, Clone)]
pub struct InstructionFile {
    pub path: PathBuf,
    pub content: String,
}

#[derive(Debug, Clone, Default)]
pub struct ProjectContext {
    pub root: Option<PathBuf>,
    pub instructions: Vec<InstructionFile>,
}

impl ProjectContext {
    /// Nạp chỉ dẫn dự án từ `root`. Thư mục không tồn tại trả về context rỗng.
    pub fn load(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref().to_path_buf();
        if !root.is_dir() {
            return Self::default();
        }

        let mut remaining = MAX_INSTRUCTIONS_BYTES;
        let mut instructions = Vec::new();
        for relative in INSTRUCTION_FILES {
            if remaining == 0 {
                break;
            }
            let path = root.join(relative);
            if !path.is_file() {
                continue;
            }
            let Ok(raw) = std::fs::read_to_string(&path) else {
                continue;
            };
            let content = truncate(raw.trim(), remaining);
            if content.is_empty() {
                continue;
            }
            remaining = remaining.saturating_sub(content.len());
            instructions.push(InstructionFile { path, content });
        }

        Self {
            root: Some(root),
            instructions,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.instructions.is_empty()
    }

    /// Render thành các khối `<project-instructions>` cho system prompt.
    pub fn render(&self) -> String {
        if self.instructions.is_empty() {
            return String::new();
        }
        let mut out = String::new();
        for file in &self.instructions {
            let label = self
                .root
                .as_deref()
                .and_then(|root| file.path.strip_prefix(root).ok())
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| file.path.display().to_string());
            out.push_str(&format!(
                "<project-instructions path=\"{label}\">\n{}\n</project-instructions>\n",
                file.content
            ));
        }
        out.trim_end().to_string()
    }

    /// Thư mục skill riêng của dự án: `<root>/.tez/skills`.
    pub fn skills_dir(&self) -> Option<PathBuf> {
        self.root
            .as_ref()
            .map(|root| root.join(".tez").join("skills"))
    }

    /// File MCP riêng của dự án: `<root>/.tez/mcp.json`.
    pub fn mcp_config_path(&self) -> Option<PathBuf> {
        self.root
            .as_ref()
            .map(|root| root.join(".tez").join("mcp.json"))
    }
}

fn truncate(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n… (đã cắt bớt)", &text[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_and_renders_project_instructions() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "Dùng tiếng Việt.").unwrap();
        std::fs::create_dir_all(dir.path().join(".tez")).unwrap();
        std::fs::write(
            dir.path().join(".tez").join("AGENTS.md"),
            "Chạy test trước.",
        )
        .unwrap();

        let project = ProjectContext::load(dir.path());
        assert_eq!(project.instructions.len(), 2);

        let rendered = project.render();
        assert!(rendered.contains("path=\"AGENTS.md\""));
        assert!(rendered.contains("Dùng tiếng Việt."));
        assert!(rendered.contains("path=\".tez/AGENTS.md\""));
        assert!(rendered.contains("Chạy test trước."));
    }

    #[test]
    fn missing_root_is_empty() {
        let project = ProjectContext::load("/tmp/tez-project-khong-ton-tai");
        assert!(project.is_empty());
        assert_eq!(project.render(), "");
        assert!(project.root.is_none());
    }

    #[test]
    fn truncates_long_instructions() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("AGENTS.md"),
            "a".repeat(MAX_INSTRUCTIONS_BYTES + 500),
        )
        .unwrap();
        let project = ProjectContext::load(dir.path());
        let content = &project.instructions[0].content;
        assert!(content.contains("đã cắt bớt"));
        assert!(content.len() <= MAX_INSTRUCTIONS_BYTES + 32);
    }

    #[test]
    fn exposes_project_extension_paths() {
        let dir = tempfile::tempdir().unwrap();
        let project = ProjectContext::load(dir.path());
        assert_eq!(
            project.skills_dir().unwrap(),
            dir.path().join(".tez").join("skills")
        );
        assert_eq!(
            project.mcp_config_path().unwrap(),
            dir.path().join(".tez").join("mcp.json")
        );
    }
}
