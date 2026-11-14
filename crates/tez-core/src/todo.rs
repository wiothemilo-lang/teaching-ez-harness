//! TodoWrite: kế hoạch ngắn hạn của agent, hiển thị được trên UI.
//!
//! `todo_write` không thêm khả năng thực thi; nó thêm khả năng lập kế hoạch.
//! Danh sách được giữ trong [`TodoState`] để agent và UI cùng đọc.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tez_llm::ToolSpec;

use crate::tools::{Tool, ToolError, ToolOutput};

pub const MAX_TODOS: usize = 20;
const MAX_CONTENT_CHARS: usize = 240;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    #[default]
    Pending,
    InProgress,
    Completed,
}

impl TodoStatus {
    fn parse(value: &str) -> Self {
        match value.trim().to_lowercase().replace('-', "_").as_str() {
            "in_progress" | "inprogress" | "doing" => Self::InProgress,
            "completed" | "done" | "complete" => Self::Completed,
            _ => Self::Pending,
        }
    }

    fn marker(self) -> &'static str {
        match self {
            Self::Pending => "[ ]",
            Self::InProgress => "[>]",
            Self::Completed => "[x]",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoItem {
    pub content: String,
    #[serde(default)]
    pub status: TodoStatus,
}

impl TodoItem {
    pub fn new(content: impl Into<String>, status: TodoStatus) -> Self {
        Self {
            content: content.into(),
            status,
        }
    }
}

/// Trạng thái todo dùng chung giữa tool, agent và UI.
#[derive(Default)]
pub struct TodoState {
    items: Mutex<Vec<TodoItem>>,
    writes: AtomicUsize,
}

impl TodoState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> Vec<TodoItem> {
        self.items
            .lock()
            .map(|items| items.clone())
            .unwrap_or_default()
    }

    pub fn writes(&self) -> usize {
        self.writes.load(Ordering::Relaxed)
    }

    pub fn replace(&self, items: Vec<TodoItem>) -> Result<Vec<TodoItem>, String> {
        validate(&items)?;
        if let Ok(mut current) = self.items.lock() {
            *current = items;
            self.writes.fetch_add(1, Ordering::Relaxed);
            return Ok(current.clone());
        }
        Err("không cập nhật được todo".to_string())
    }

    pub fn render(&self) -> String {
        render(&self.snapshot())
    }
}

fn validate(items: &[TodoItem]) -> Result<(), String> {
    if items.is_empty() {
        return Err("danh sách todo rỗng".to_string());
    }
    if items.len() > MAX_TODOS {
        return Err(format!("tối đa {MAX_TODOS} mục todo"));
    }
    let mut in_progress = 0;
    for (index, item) in items.iter().enumerate() {
        let content = item.content.trim();
        if content.is_empty() {
            return Err(format!("mục {} không có nội dung", index + 1));
        }
        if content.chars().count() > MAX_CONTENT_CHARS {
            return Err(format!(
                "mục {} dài quá {MAX_CONTENT_CHARS} ký tự",
                index + 1
            ));
        }
        if item.status == TodoStatus::InProgress {
            in_progress += 1;
        }
    }
    if in_progress > 1 {
        return Err("chỉ được có một mục in_progress".to_string());
    }
    Ok(())
}

/// Render danh sách todo thành text cho model/UI.
pub fn render(items: &[TodoItem]) -> String {
    if items.is_empty() {
        return "(chưa có kế hoạch)".to_string();
    }
    let done = items
        .iter()
        .filter(|item| item.status == TodoStatus::Completed)
        .count();
    let mut out = format!("Kế hoạch ({done}/{}):\n", items.len());
    for item in items {
        out.push_str(item.status.marker());
        out.push(' ');
        out.push_str(item.content.trim());
        out.push('\n');
    }
    out.trim_end().to_string()
}

/// Tool `todo_write`: thay toàn bộ danh sách todo.
pub struct TodoWrite {
    state: Arc<TodoState>,
}

impl TodoWrite {
    pub fn new(state: Arc<TodoState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl Tool for TodoWrite {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "todo_write".into(),
            description: "Tạo hoặc cập nhật kế hoạch làm việc. Dùng cho việc nhiều bước: liệt kê các bước trước khi làm, cập nhật trạng thái khi tiến triển. Chỉ một mục được in_progress."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "todos": {
                        "type": "array",
                        "maxItems": MAX_TODOS,
                        "items": {
                            "type": "object",
                            "properties": {
                                "content": { "type": "string", "description": "Bước cần làm, ngắn gọn" },
                                "status": {
                                    "type": "string",
                                    "enum": ["pending", "in_progress", "completed"]
                                }
                            },
                            "required": ["content", "status"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["todos"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let raw = args
            .get("todos")
            .and_then(Value::as_array)
            .ok_or_else(|| ToolError::InvalidArgs("thiếu mảng 'todos'".to_string()))?;

        let mut items = Vec::with_capacity(raw.len());
        for value in raw {
            let content = value
                .get("content")
                .and_then(Value::as_str)
                .ok_or_else(|| ToolError::InvalidArgs("todo thiếu 'content'".to_string()))?;
            let status = value
                .get("status")
                .and_then(Value::as_str)
                .map(TodoStatus::parse)
                .unwrap_or_default();
            items.push(TodoItem::new(content.trim(), status));
        }

        match self.state.replace(items) {
            Ok(items) => Ok(ToolOutput::ok(render(&items))),
            Err(error) => Ok(ToolOutput::error(error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_statuses() {
        let items = vec![
            TodoItem::new("đọc file", TodoStatus::Completed),
            TodoItem::new("sửa lỗi", TodoStatus::InProgress),
            TodoItem::new("chạy test", TodoStatus::Pending),
        ];
        let text = render(&items);
        assert!(text.contains("(1/3)"));
        assert!(text.contains("[x] đọc file"));
        assert!(text.contains("[>] sửa lỗi"));
        assert!(text.contains("[ ] chạy test"));
    }

    #[test]
    fn rejects_two_in_progress() {
        let state = TodoState::new();
        let result = state.replace(vec![
            TodoItem::new("a", TodoStatus::InProgress),
            TodoItem::new("b", TodoStatus::InProgress),
        ]);
        assert!(result.is_err());
        assert_eq!(state.writes(), 0);
    }

    #[tokio::test]
    async fn tool_updates_state() {
        let state = Arc::new(TodoState::new());
        let tool = TodoWrite::new(Arc::clone(&state));
        let output = tool
            .execute(json!({
                "todos": [
                    {"content": "khảo sát", "status": "completed"},
                    {"content": "triển khai", "status": "in_progress"}
                ]
            }))
            .await
            .unwrap();
        assert!(!output.is_error);
        assert_eq!(state.snapshot().len(), 2);
        assert_eq!(state.writes(), 1);
    }
}
