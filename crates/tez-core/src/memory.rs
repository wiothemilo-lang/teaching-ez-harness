//! Memory: lưu kiến thức tái dùng giữa các phiên.
//!
//! Mỗi bản ghi là một file Markdown trong `<data_dir>/memory/`, có frontmatter
//! `name`, `description`, `type`. `MEMORY.md` là index ngắn; chỉ bản ghi liên
//! quan mới được nạp vào context.
//!
//! Bốn loại: `user` (sở thích), `feedback` (cách làm việc), `project` (sự thật
//! dự án), `reference` (đầu mối tra cứu).

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tez_llm::ToolSpec;

use crate::tools::{Tool, ToolError, ToolOutput};

const MAX_BODY_CHARS: usize = 8_000;
const MAX_RECALL_CHARS: usize = 6_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryRecord {
    pub name: String,
    pub description: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub body: String,
    pub path: PathBuf,
}

impl MemoryRecord {
    pub fn preview(&self, limit: usize) -> String {
        let mut body: String = self.body.chars().take(limit).collect();
        if self.body.chars().count() > limit {
            body.push('…');
        }
        body
    }
}

#[derive(Debug, Clone)]
pub struct MemoryStore {
    dir: PathBuf,
}

impl MemoryStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn ensure_dir(&self) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir)
            .map_err(|error| format!("không tạo được thư mục memory: {error}"))
    }

    /// Nạp tất cả bản ghi, bỏ qua file hỏng.
    pub fn records(&self) -> Vec<MemoryRecord> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut records: Vec<MemoryRecord> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension().and_then(|value| value.to_str()) == Some("md")
                    && path.file_name().and_then(|value| value.to_str()) != Some("MEMORY.md")
            })
            .filter_map(|path| parse_file(&path).ok())
            .collect();
        records.sort_by_key(|record| record.name.to_lowercase());
        records
    }

    pub fn find(&self, name: &str) -> Option<MemoryRecord> {
        let target = slug(name);
        self.records()
            .into_iter()
            .find(|record| slug(record.name.as_str()) == target)
    }

    pub fn save(
        &self,
        name: &str,
        description: &str,
        kind: &str,
        body: &str,
    ) -> Result<MemoryRecord, String> {
        let name = name.trim();
        let body = body.trim();
        if name.is_empty() {
            return Err("tên bản ghi không được rỗng".to_string());
        }
        if body.is_empty() {
            return Err("nội dung bản ghi không được rỗng".to_string());
        }
        if body.chars().count() > MAX_BODY_CHARS {
            return Err(format!("nội dung quá dài (tối đa {MAX_BODY_CHARS} ký tự)"));
        }
        let kind = normalize_kind(kind);
        let description = if description.trim().is_empty() {
            first_line(body)
        } else {
            description.trim().to_string()
        };
        let file_slug = slug(name);
        if file_slug.is_empty() {
            return Err("tên bản ghi không hợp lệ".to_string());
        }

        self.ensure_dir()?;
        let path = self.dir.join(format!("{file_slug}.md"));
        let document =
            format!("---\nname: {name}\ndescription: {description}\ntype: {kind}\n---\n\n{body}\n");
        std::fs::write(&path, document)
            .map_err(|error| format!("không ghi được memory: {error}"))?;
        self.rebuild_index()?;

        Ok(MemoryRecord {
            name: name.to_string(),
            description,
            kind,
            body: body.to_string(),
            path,
        })
    }

    pub fn forget(&self, name: &str) -> Result<bool, String> {
        let slug = slug(name);
        let path = self.dir.join(format!("{slug}.md"));
        if path.exists() {
            std::fs::remove_file(&path)
                .map_err(|error| format!("không xoá được memory: {error}"))?;
            self.rebuild_index()?;
            return Ok(true);
        }
        Ok(false)
    }

    /// Chọn bản ghi liên quan bằng chấm điểm từ khoá, không gọi model.
    pub fn search(&self, query: &str, limit: usize) -> Vec<MemoryRecord> {
        let terms = tokens(query);
        if terms.is_empty() {
            return Vec::new();
        }
        let mut scored: Vec<(usize, MemoryRecord)> = self
            .records()
            .into_iter()
            .filter_map(|record| {
                let name = record.name.to_lowercase();
                let description = record.description.to_lowercase();
                let body = record.body.to_lowercase();
                let mut score = 0;
                for term in &terms {
                    if name.contains(term) {
                        score += 6;
                    }
                    if description.contains(term) {
                        score += 3;
                    }
                    if body.contains(term) {
                        score += 1;
                    }
                }
                (score > 0).then_some((score, record))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
        scored
            .into_iter()
            .take(limit)
            .map(|(_, record)| record)
            .collect()
    }

    /// Render index `MEMORY.md` từ các file hiện có.
    pub fn rebuild_index(&self) -> Result<(), String> {
        self.ensure_dir()?;
        let records = self.records();
        let mut index = String::from("# Bộ nhớ của tez\n\n");
        if records.is_empty() {
            index.push_str("(chưa có bản ghi)\n");
        } else {
            index.push_str("Các bản ghi đang lưu (chỉ nạp khi liên quan):\n\n");
            for record in &records {
                index.push_str(&format!(
                    "- [{}]({}.md) — {} ({})\n",
                    record.name,
                    slug(&record.name),
                    record.description,
                    record.kind
                ));
            }
        }
        std::fs::write(self.dir.join("MEMORY.md"), index)
            .map_err(|error| format!("không ghi được index memory: {error}"))
    }

    pub fn index_text(&self) -> String {
        std::fs::read_to_string(self.dir.join("MEMORY.md")).unwrap_or_default()
    }
}

fn normalize_kind(kind: &str) -> String {
    match kind.trim().to_lowercase().as_str() {
        "user" | "preference" => "user".to_string(),
        "feedback" | "instruction" => "feedback".to_string(),
        "reference" | "link" => "reference".to_string(),
        _ => "project".to_string(),
    }
}

fn first_line(body: &str) -> String {
    body.lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim().chars().take(160).collect())
        .unwrap_or_default()
}

fn slug(value: &str) -> String {
    let mut slug = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if (ch.is_whitespace() || ch == '-' || ch == '_') && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_matches('-').to_string()
}

fn tokens(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|term| term.chars().count() >= 2)
        .map(ToOwned::to_owned)
        .collect()
}

fn parse_file(path: &Path) -> Result<MemoryRecord, String> {
    let raw = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    let raw = raw.replace("\r\n", "\n");
    let mut name = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("memory")
        .to_string();
    let mut description = String::new();
    let mut kind = "project".to_string();

    let body = if let Some(rest) = raw.strip_prefix("---\n") {
        if let Some(end) = rest.find("\n---") {
            for line in rest[..end].lines() {
                if let Some((key, value)) = line.split_once(':') {
                    let value = value
                        .trim()
                        .trim_matches('"')
                        .trim_matches('\'')
                        .to_string();
                    match key.trim().to_lowercase().as_str() {
                        "name" => name = value,
                        "description" => description = value,
                        "type" | "kind" => kind = normalize_kind(&value),
                        _ => {}
                    }
                }
            }
            rest[end + 4..].trim_start_matches('\n').to_string()
        } else {
            raw.clone()
        }
    } else {
        raw.clone()
    };

    Ok(MemoryRecord {
        name,
        description,
        kind,
        body: body.trim().to_string(),
        path: path.to_path_buf(),
    })
}

/// Render các bản ghi đã chọn vào system prompt.
pub fn render_recalled(records: &[MemoryRecord]) -> Vec<String> {
    let mut out = Vec::new();
    let mut used = 0usize;
    for record in records {
        if used >= MAX_RECALL_CHARS {
            break;
        }
        let remaining = MAX_RECALL_CHARS - used;
        let preview = record.preview(remaining.min(1_500));
        used += preview.chars().count();
        out.push(format!(
            "<memory name=\"{}\" type=\"{}\">\n{}\n</memory>",
            record.name, record.kind, preview
        ));
    }
    out
}

pub struct MemorySave {
    store: MemoryStore,
}

impl MemorySave {
    pub fn new(store: MemoryStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for MemorySave {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "memory_save".into(),
            description: "Lưu một thông tin tái dùng được cho các phiên sau (sở thích người dùng, quy ước dự án, cách làm việc, đầu mối tra cứu). Không lưu thông tin tạm thời của phiên hiện tại."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Tên ngắn, duy nhất" },
                    "body": { "type": "string", "description": "Nội dung cần nhớ" },
                    "description": { "type": "string", "description": "Một dòng mô tả" },
                    "kind": {
                        "type": "string",
                        "enum": ["user", "feedback", "project", "reference"]
                    }
                },
                "required": ["name", "body"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let name = args
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArgs("thiếu 'name'".to_string()))?;
        let body = args
            .get("body")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArgs("thiếu 'body'".to_string()))?;
        let description = args
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let kind = args
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("project");

        match self.store.save(name, description, kind, body) {
            Ok(record) => Ok(ToolOutput::ok(format!(
                "đã lưu memory '{}' ({})",
                record.name, record.kind
            ))),
            Err(error) => Ok(ToolOutput::error(error)),
        }
    }
}

pub struct MemorySearch {
    store: MemoryStore,
}

impl MemorySearch {
    pub fn new(store: MemoryStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for MemorySearch {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "memory_search".into(),
            description: "Tìm trong bộ nhớ đã lưu các bản ghi liên quan tới một chủ đề.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Từ khoá hoặc câu hỏi" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 10 }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArgs("thiếu 'query'".to_string()))?;
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(5)
            .clamp(1, 10) as usize;
        let records = self.store.search(query, limit);
        if records.is_empty() {
            return Ok(ToolOutput::ok("(không tìm thấy memory liên quan)"));
        }
        let mut out = String::new();
        for record in records {
            out.push_str(&format!(
                "## {} ({})\n{}\n\n{}\n\n",
                record.name,
                record.kind,
                record.description,
                record.preview(1_200)
            ));
        }
        Ok(ToolOutput::ok(out.trim_end().to_string()))
    }
}

pub struct MemoryForget {
    store: MemoryStore,
}

impl MemoryForget {
    pub fn new(store: MemoryStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for MemoryForget {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "memory_forget".into(),
            description: "Xoá một bản ghi bộ nhớ theo tên khi thông tin đã cũ hoặc sai.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Tên bản ghi cần xoá" }
                },
                "required": ["name"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let name = args
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArgs("thiếu 'name'".to_string()))?;
        match self.store.forget(name) {
            Ok(true) => Ok(ToolOutput::ok(format!("đã xoá memory '{name}'"))),
            Ok(false) => Ok(ToolOutput::error(format!("không tìm thấy memory '{name}'"))),
            Err(error) => Ok(ToolOutput::error(error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_searches_and_forgets() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(dir.path().join("memory"));
        store
            .save(
                "Dùng tabs",
                "Người dùng thích tab để thụt lề",
                "user",
                "Luôn dùng tab, không dùng space.",
            )
            .unwrap();
        store
            .save(
                "Build command",
                "Lệnh build của dự án",
                "project",
                "Chạy `pnpm build` trước khi commit.",
            )
            .unwrap();

        let hits = store.search("thụt lề bằng tab", 3);
        assert_eq!(hits[0].name, "Dùng tabs");
        let build = store.search("build commit", 3);
        assert!(build.iter().any(|record| record.name == "Build command"));

        let index = store.index_text();
        assert!(index.contains("Dùng tabs"));
        assert!(index.contains("Build command"));

        assert!(store.forget("Dùng tabs").unwrap());
        assert!(store.find("Dùng tabs").is_none());
    }

    #[test]
    fn overwrites_record_with_same_slug() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(dir.path());
        store.save("Style", "", "user", "dùng space").unwrap();
        store.save("Style", "", "user", "dùng tab").unwrap();
        assert_eq!(store.records().len(), 1);
        assert!(store.find("Style").unwrap().body.contains("tab"));
    }

    #[tokio::test]
    async fn tools_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(dir.path());
        let save = MemorySave::new(store.clone());
        let output = save
            .execute(json!({
                "name": "Ngôn ngữ trả lời",
                "body": "Trả lời bằng tiếng Việt.",
                "kind": "user"
            }))
            .await
            .unwrap();
        assert!(!output.is_error, "{}", output.content);

        let search = MemorySearch::new(store.clone());
        let output = search
            .execute(json!({"query": "tiếng Việt"}))
            .await
            .unwrap();
        assert!(output.content.contains("Ngôn ngữ trả lời"));

        let forget = MemoryForget::new(store);
        let output = forget
            .execute(json!({"name": "Ngôn ngữ trả lời"}))
            .await
            .unwrap();
        assert!(!output.is_error);
    }
}
