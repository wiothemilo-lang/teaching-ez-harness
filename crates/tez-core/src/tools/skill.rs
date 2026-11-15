//! Tool `skill`: nạp nội dung đầy đủ của một skill theo id.

use async_trait::async_trait;
use serde_json::{json, Value};
use tez_llm::ToolSpec;

use super::{Tool, ToolError, ToolOutput};
use crate::skills::{self, Skill, SkillRegistry};

/// Giới hạn nội dung skill trả về model để không nuốt cả context.
const MAX_BODY_CHARS: usize = 12_000;
/// Số id gợi ý tối đa khi model gọi sai tên.
const MAX_SUGGESTIONS: usize = 30;

/// Tool cho model tự nạp skill khi danh mục gợi ý phù hợp.
pub struct SkillTool {
    registry: SkillRegistry,
}

impl SkillTool {
    pub fn new(registry: SkillRegistry) -> Self {
        Self { registry }
    }

    fn find(&self, name: &str) -> Option<&Skill> {
        if let Some(skill) = self.registry.find(name) {
            return Some(skill);
        }
        let needle = name.to_lowercase();
        self.registry
            .all()
            .iter()
            .find(|skill| skill.id.to_lowercase() == needle || skill.name.to_lowercase() == needle)
    }

    fn unknown_message(&self, name: &str) -> String {
        let mut ids: Vec<&str> = self
            .registry
            .all()
            .iter()
            .map(|skill| skill.id.as_str())
            .collect();
        ids.sort_unstable();
        ids.truncate(MAX_SUGGESTIONS);
        if ids.is_empty() {
            return format!("không tìm thấy skill '{name}' và chưa có skill nào");
        }
        format!(
            "không tìm thấy skill '{name}'. Id khả dụng: {}",
            ids.join(", ")
        )
    }
}

#[async_trait]
impl Tool for SkillTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "skill".into(),
            description: "Nạp hướng dẫn đầy đủ của một skill theo id hoặc tên khi danh mục \
                          skill gợi ý phù hợp."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Id hoặc tên skill, ví dụ: step-by-step"
                    }
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
            .ok_or_else(|| ToolError::InvalidArgs("thiếu trường 'name'".into()))?;
        let name = name.trim();
        if name.is_empty() {
            return Ok(ToolOutput::error("tên skill rỗng".to_string()));
        }

        let Some(skill) = self.find(name) else {
            return Ok(ToolOutput::error(self.unknown_message(name)));
        };

        let mut body = skills::neutralize_closing_tag(skill.body.trim());
        if body.chars().count() > MAX_BODY_CHARS {
            body = body.chars().take(MAX_BODY_CHARS).collect();
            body.push_str("\n… (skill đã cắt bớt)");
        }
        let id = skills::sanitize_attribute(&skill.id);
        let name = skills::sanitize_attribute(&skill.name);
        Ok(ToolOutput::ok(format!(
            "<skill id=\"{id}\" name=\"{name}\">\n{body}\n</skill>"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_skill(root: &std::path::Path, id: &str, name: &str, description: &str, body: &str) {
        let dir = root.join("skills").join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\n{body}"),
        )
        .unwrap();
    }

    fn review_registry() -> (tempfile::TempDir, SkillRegistry) {
        let dir = tempfile::tempdir().unwrap();
        write_skill(
            dir.path(),
            "review",
            "Ôn tập nhanh",
            "Rà lại kiến thức trước khi kiểm tra.",
            "Nội dung review.",
        );
        let registry = SkillRegistry::load(dir.path()).unwrap();
        (dir, registry)
    }

    #[test]
    fn spec_describes_name_argument() {
        let (_dir, registry) = review_registry();
        let spec = SkillTool::new(registry).spec();
        assert_eq!(spec.name, "skill");
        assert_eq!(spec.parameters["required"], json!(["name"]));
        assert_eq!(spec.parameters["properties"]["name"]["type"], "string");
    }

    #[tokio::test]
    async fn loads_skill_body_by_id() {
        let (_dir, registry) = review_registry();
        let tool = SkillTool::new(registry);
        let out = tool.execute(json!({"name": "review"})).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(
            out.content,
            "<skill id=\"review\" name=\"Ôn tập nhanh\">\nNội dung review.\n</skill>"
        );
    }

    #[tokio::test]
    async fn trims_name_and_matches_case_insensitive() {
        let (_dir, registry) = review_registry();
        let tool = SkillTool::new(registry);
        let by_id = tool.execute(json!({"name": "  REVIEW  "})).await.unwrap();
        assert!(!by_id.is_error, "{}", by_id.content);
        let by_name = tool.execute(json!({"name": "ôn tập nhanh"})).await.unwrap();
        assert!(!by_name.is_error, "{}", by_name.content);
        assert_eq!(by_id.content, by_name.content);
    }

    #[tokio::test]
    async fn reports_unknown_skill_with_available_ids() {
        let dir = tempfile::tempdir().unwrap();
        let registry = SkillRegistry::load(dir.path()).unwrap();
        let tool = SkillTool::new(registry);
        let out = tool.execute(json!({"name": "không-có"})).await.unwrap();
        assert!(out.is_error);
        assert!(out.content.contains("không tìm thấy skill 'không-có'"));
        assert!(out.content.contains("step-by-step"));
    }

    #[tokio::test]
    async fn rejects_missing_and_empty_name() {
        let (_dir, registry) = review_registry();
        let tool = SkillTool::new(registry);
        let missing = tool.execute(json!({})).await.unwrap_err();
        assert!(matches!(missing, ToolError::InvalidArgs(_)));
        let empty = tool.execute(json!({"name": "   "})).await.unwrap();
        assert!(empty.is_error);
    }

    #[tokio::test]
    async fn truncates_long_body_on_char_boundary() {
        let dir = tempfile::tempdir().unwrap();
        write_skill(
            dir.path(),
            "long",
            "Dài",
            "Thân bài dài.",
            &"é".repeat(MAX_BODY_CHARS + 100),
        );
        let registry = SkillRegistry::load(dir.path()).unwrap();
        let tool = SkillTool::new(registry);
        let out = tool.execute(json!({"name": "long"})).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.ends_with("… (skill đã cắt bớt)\n</skill>"));
        assert_eq!(out.content.matches('é').count(), MAX_BODY_CHARS);
    }
}
