//! Skill packs: file Markdown trong `skills/<id>/SKILL.md`.
//!
//! Frontmatter tối giản:
//! ```text
//! ---
//! name: Giải từng bước
//! description: ...
//! subjects: math, physics
//! triggers: giải, bước, chứng minh
//! ---
//! Nội dung hướng dẫn cho gia sư...
//! ```
//!
//! Skill không chạy code; nó là prompt pack được chèn vào context khi khớp
//! môn học + từ khóa trong câu hỏi.

use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SkillError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("skill không hợp lệ: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub subjects: Vec<String>,
    pub triggers: Vec<String>,
    pub body: String,
    pub source: String,
    pub path: Option<PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub struct SkillRegistry {
    skills: Vec<Skill>,
}

impl SkillRegistry {
    /// Nạp skill của user và seed bộ skill mặc định nếu thư mục chưa có.
    pub fn load(dir: impl AsRef<Path>) -> Result<Self, SkillError> {
        let dir = dir.as_ref();
        let skills_dir = dir.join("skills");
        if !skills_dir.exists() {
            seed_builtin(&skills_dir)?;
        }
        let mut registry = Self::default();
        registry.load_dir(&skills_dir, "user")?;
        Ok(registry)
    }

    /// Nạp skill của user + skill đi kèm mọi plugin local.
    pub fn load_with_plugins(dir: impl AsRef<Path>) -> Result<Self, SkillError> {
        let dir = dir.as_ref();
        let mut registry = Self::load(dir)?;
        let plugins = crate::plugins::load_all(dir)
            .map_err(|error| SkillError::Invalid(error.to_string()))?;
        for plugin in plugins {
            registry.load_extra(&plugin.skill_paths(), &plugin.manifest.id)?;
        }
        Ok(registry)
    }

    /// Nạp thêm skill từ plugin (đường dẫn tuyệt đối hoặc tương đối với plugin).
    pub fn load_extra(&mut self, paths: &[PathBuf], source: &str) -> Result<(), SkillError> {
        for path in paths {
            if path.is_dir() {
                self.load_dir(path, source)?;
            } else if path.is_file() {
                if let Some(skill) = load_file(path, source)? {
                    self.upsert(skill);
                }
            }
        }
        Ok(())
    }

    fn load_dir(&mut self, dir: &Path, source: &str) -> Result<(), SkillError> {
        if !dir.is_dir() {
            return Ok(());
        }
        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect();
        entries.sort();

        for entry in entries {
            if entry.is_dir() {
                let candidate = entry.join("SKILL.md");
                if candidate.is_file() {
                    if let Some(skill) = load_file(&candidate, source)? {
                        self.upsert(skill);
                    }
                }
            } else if entry.extension().and_then(|value| value.to_str()) == Some("md") {
                if let Some(skill) = load_file(&entry, source)? {
                    self.upsert(skill);
                }
            }
        }
        Ok(())
    }

    fn upsert(&mut self, skill: Skill) {
        if let Some(existing) = self.skills.iter_mut().find(|item| item.id == skill.id) {
            *existing = skill;
        } else {
            self.skills.push(skill);
        }
    }

    pub fn all(&self) -> &[Skill] {
        &self.skills
    }

    pub fn find(&self, id: &str) -> Option<&Skill> {
        self.skills.iter().find(|skill| skill.id == id)
    }

    /// Chọn tối đa `limit` skill khớp môn và từ khóa.
    pub fn matching(&self, subject: &str, query: &str, limit: usize) -> Vec<&Skill> {
        self.select(query, limit, |skill| {
            skill.subjects.is_empty()
                || skill
                    .subjects
                    .iter()
                    .any(|item| item == "all" || item == subject)
        })
    }

    /// Chọn skill chỉ theo từ khóa, không ràng buộc môn học.
    pub fn matching_query(&self, query: &str, limit: usize) -> Vec<&Skill> {
        self.select(query, limit, |_| true)
    }

    fn select<F>(&self, query: &str, limit: usize, subject_ok: F) -> Vec<&Skill>
    where
        F: Fn(&Skill) -> bool,
    {
        let query = query.to_lowercase();
        let mut matched: Vec<&Skill> = self
            .skills
            .iter()
            .filter(|skill| {
                let trigger_ok = skill.triggers.is_empty()
                    || skill
                        .triggers
                        .iter()
                        .any(|trigger| query.contains(&trigger.to_lowercase()));
                subject_ok(skill) && trigger_ok
            })
            .collect();
        matched.sort_by(|a, b| a.name.cmp(&b.name));
        matched.truncate(limit);
        matched
    }
}

/// Render skill thành đoạn text đưa vào system prompt.
pub fn render(skill: &Skill) -> String {
    let mut body = skill.body.trim().to_string();
    if body.len() > 4 * 1024 {
        let mut end = 4 * 1024;
        while !body.is_char_boundary(end) {
            end -= 1;
        }
        body.truncate(end);
        body.push_str("\n… (skill đã cắt bớt)");
    }
    format!("<skill name=\"{}\">\n{}\n</skill>", skill.name, body)
}

fn load_file(path: &Path, source: &str) -> Result<Option<Skill>, SkillError> {
    let raw = std::fs::read_to_string(path)?;
    let raw = raw.replace("\r\n", "\n");
    let (meta, body) = split_frontmatter(&raw);
    let body = body.trim().to_string();
    if body.is_empty() {
        return Ok(None);
    }

    let fallback_id = if path.file_name().and_then(|name| name.to_str()) == Some("SKILL.md") {
        path.parent()
            .and_then(|parent| parent.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or("skill")
    } else {
        path.file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("skill")
    };
    let id = meta_value(&meta, "id")
        .map(|value| slug(&value))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| slug(fallback_id));
    let name = meta_value(&meta, "name").unwrap_or_else(|| fallback_id.to_string());
    let description = meta_value(&meta, "description").unwrap_or_default();
    let subjects = meta_list(&meta, "subjects");
    let triggers = meta_list(&meta, "triggers");

    Ok(Some(Skill {
        id,
        name,
        description,
        subjects,
        triggers,
        body,
        source: source.to_string(),
        path: Some(path.to_path_buf()),
    }))
}

fn split_frontmatter(raw: &str) -> (Vec<(String, String)>, &str) {
    let trimmed = raw.trim_start_matches('\u{feff}');
    if !trimmed.starts_with("---\n") {
        return (Vec::new(), trimmed);
    }
    let rest = &trimmed[4..];
    if let Some(end) = rest.find("\n---") {
        let meta_block = &rest[..end];
        let body = rest[end + 4..].trim_start_matches('\n');
        let mut meta = Vec::new();
        for line in meta_block.lines() {
            if let Some((key, value)) = line.split_once(':') {
                let key = key.trim().to_lowercase();
                let value = value
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string();
                if !key.is_empty() {
                    meta.push((key, value));
                }
            }
        }
        return (meta, body);
    }
    (Vec::new(), trimmed)
}

fn meta_value(meta: &[(String, String)], key: &str) -> Option<String> {
    meta.iter()
        .find(|(item, _)| item == key)
        .map(|(_, value)| value.clone())
        .filter(|value| !value.trim().is_empty())
}

fn meta_list(meta: &[(String, String)], key: &str) -> Vec<String> {
    meta_value(meta, key)
        .map(|value| {
            value
                .split([',', ';'])
                .map(|item| item.trim().to_lowercase())
                .filter(|item| !item.is_empty())
                .collect()
        })
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

const BUILTINS: &[(&str, &str)] = &[
    (
        "step-by-step",
        r#"---
name: Giải từng bước
description: Chia bài thành các bước nhỏ, kiểm tra từng bước trước khi đi tiếp.
subjects: math, physics, chemistry
triggers: giải, bước, chứng minh, tính, bài tập
---
Khi hướng dẫn:
1. Tóm tắt dữ kiện và yêu cầu trong 1-2 dòng.
2. Chia thành các bước nhỏ, mỗi lần chỉ hỏi hoặc giải một bước.
3. Sau mỗi bước, hỏi học sinh đã hiểu chưa trước khi sang bước tiếp.
4. Dùng tool để tính/kiểm tra thay vì tính nhẩm.
5. Kết thúc bằng một câu hỏi kiểm tra hiểu bài.
"#,
    ),
    (
        "error-diagnosis",
        r#"---
name: Chẩn đoán lỗi
description: Tìm đúng chỗ sai trong lời giải của học sinh, không chỉ đưa đáp án đúng.
subjects: math, physics, chemistry
triggers: sai, nhầm, lỗi, kiểm tra bước, không ra, đáp án
---
Khi học sinh đưa lời giải:
1. Chỉ ra chính xác dòng/bước đầu tiên bị sai.
2. Giải thích vì sao sai bằng ngôn ngữ đơn giản.
3. Đưa một câu hỏi gợi mở để học sinh tự sửa.
4. Chỉ đưa lời giải đầy đủ sau khi học sinh đã thử lại.
5. Nếu lỗi do hiểu sai khái niệm, nhắc lại khái niệm đó bằng ví dụ ngắn.
"#,
    ),
    (
        "exam-review",
        r#"---
name: Ôn thi có trọng tâm
description: Hệ thống hóa kiến thức, luyện dạng bài và theo dõi điểm yếu.
subjects: all
triggers: ôn thi, ôn tập, đề, kiểm tra, thi, tổng hợp
---
Khi ôn tập:
1. Hỏi mục tiêu và thời gian còn lại.
2. Chọn 3-5 dạng bài quan trọng nhất, không dàn trải.
3. Với mỗi dạng: ví dụ mẫu ngắn, sau đó một bài để học sinh tự làm.
4. Ghi lại lỗi lặp lại và nhắc lại vào cuối buổi.
5. Kết thúc bằng checklist việc cần làm trước kỳ thi.
"#,
    ),
    (
        "english-writing",
        r#"---
name: Sửa bài viết tiếng Anh
description: Chữa lỗi theo mức độ, giữ giọng văn của học sinh.
subjects: english
triggers: viết, sửa, essay, paragraph, grammar, email, đoạn văn
---
Khi chữa bài viết:
1. Khen một điểm làm tốt cụ thể.
2. Sửa lỗi theo thứ tự: ngữ pháp rõ ràng → từ vựng → mạch văn.
3. Với mỗi lỗi, đưa câu gốc, câu sửa và giải thích ngắn bằng tiếng Việt.
4. Không viết lại toàn bộ bài trừ khi học sinh yêu cầu.
5. Đề xuất một câu hỏi hoặc bài tập nhỏ để luyện đúng lỗi vừa sửa.
"#,
    ),
];

fn seed_builtin(skills_dir: &Path) -> Result<(), SkillError> {
    std::fs::create_dir_all(skills_dir)?;
    for (id, content) in BUILTINS {
        let dir = skills_dir.join(id);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("SKILL.md"), content)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_builtin_skills() {
        let dir = tempfile::tempdir().unwrap();
        let registry = SkillRegistry::load(dir.path()).unwrap();
        assert!(registry.find("step-by-step").is_some());
        assert!(registry.find("english-writing").is_some());
    }

    #[test]
    fn matches_subject_and_trigger() {
        let dir = tempfile::tempdir().unwrap();
        let registry = SkillRegistry::load(dir.path()).unwrap();
        let math = registry.matching("math", "giải bài này từng bước", 2);
        assert!(math.iter().any(|skill| skill.id == "step-by-step"));
        let english = registry.matching("english", "sửa đoạn văn này", 2);
        assert!(english.iter().any(|skill| skill.id == "english-writing"));
        let physics = registry.matching("physics", "sửa đoạn văn", 2);
        assert!(physics.iter().all(|skill| skill.id != "english-writing"));
    }

    #[test]
    fn parses_custom_skill() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("skills").join("custom");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: Custom Skill\ndescription: test\nsubjects: math\ntriggers: đồ thị\n---\nNội dung.",
        )
        .unwrap();

        let registry = SkillRegistry::load(dir.path()).unwrap();
        let skill = registry.find("custom").unwrap();
        assert_eq!(skill.subjects, vec!["math"]);
        assert_eq!(skill.triggers, vec!["đồ thị"]);
        assert_eq!(skill.body, "Nội dung.");
    }

    #[test]
    fn loads_skills_from_plugins() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("plugins").join("demo");
        let skill_dir = plugin_dir.join("skills").join("plugin-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            plugin_dir.join("plugin.json"),
            r#"{"id":"demo","name":"Demo","skills":["skills/plugin-skill"]}"#,
        )
        .unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: Plugin Skill\nsubjects: math\ntriggers: plugin\n---\nTừ plugin.",
        )
        .unwrap();

        let registry = SkillRegistry::load_with_plugins(dir.path()).unwrap();
        let skill = registry.find("plugin-skill").unwrap();
        assert_eq!(skill.source, "demo");
        assert_eq!(skill.body, "Từ plugin.");
    }
}
