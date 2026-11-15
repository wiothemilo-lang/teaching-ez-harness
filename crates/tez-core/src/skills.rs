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

/// Giới hạn kích thước một file skill để không nuốt cả file lớn vào prompt.
const MAX_SKILL_BYTES: u64 = 256 * 1024;
/// Id và tên hiển thị cũng cần trần để prompt/catalog không bị một skill chiếm hết.
const MAX_SKILL_ID_CHARS: usize = 64;
const MAX_SKILL_NAME_CHARS: usize = 120;

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

fn source_rank(source: &str) -> u8 {
    match source {
        "project" => 0,
        "user" => 1,
        "plugin" => 2,
        "builtin" => 3,
        _ => 4,
    }
}

impl SkillRegistry {
    /// Nạp skill của user và seed bộ skill mặc định nếu thư mục chưa có.
    pub fn load(dir: impl AsRef<Path>) -> Result<Self, SkillError> {
        let dir = dir.as_ref();
        let skills_dir = dir.join("skills");
        match std::fs::symlink_metadata(&skills_dir) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                // Không đi qua symlink ở cấp thư mục gốc: nó có thể trỏ ra
                // ngoài dữ liệu user và biến mọi file lạ thành skill.
                eprintln!(
                    "[skills] bỏ qua '{}': thư mục skills là symlink",
                    skills_dir.display()
                );
                return Ok(Self::default());
            }
            Ok(_) => {}
            Err(_) => seed_builtin(&skills_dir)?,
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

    /// Nạp skill riêng của dự án từ `<root>/.tez/skills`.
    ///
    /// Skill dự án ghi đè skill cùng id từ user/plugin, để dự án có thể tùy
    /// biến hành vi mà không sửa dữ liệu toàn cục.
    pub fn load_project(&mut self, root: impl AsRef<Path>) -> Result<(), SkillError> {
        let dir = root.as_ref().join(".tez").join("skills");
        self.load_dir(&dir, "project")
    }

    /// Nạp thêm skill từ plugin (đường dẫn tuyệt đối hoặc tương đối với plugin).
    pub fn load_extra(&mut self, paths: &[PathBuf], source: &str) -> Result<(), SkillError> {
        for path in paths {
            // Không đi theo symlink: một plugin có thể trỏ ra ngoài thư mục của nó.
            let Ok(metadata) = std::fs::symlink_metadata(path) else {
                continue;
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                self.load_dir(path, source)?;
            } else if metadata.is_file() {
                self.load_into(path, source);
            }
        }
        Ok(())
    }

    /// Nạp một file skill; lỗi ở một file chỉ cảnh báo và bỏ qua file đó.
    fn load_into(&mut self, path: &Path, source: &str) {
        match load_file(path, source) {
            Ok(Some(skill)) => self.upsert(skill),
            Ok(None) => {}
            Err(error) => {
                eprintln!("[skills] bỏ qua '{}': {error}", path.display());
            }
        }
    }

    fn load_dir(&mut self, dir: &Path, source: &str) -> Result<(), SkillError> {
        let Ok(metadata) = std::fs::symlink_metadata(dir) else {
            return Ok(());
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Ok(());
        }
        let mut entries: Vec<PathBuf> = match std::fs::read_dir(dir) {
            Ok(entries) => entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .collect(),
            Err(error) => {
                eprintln!("[skills] bỏ qua '{}': {error}", dir.display());
                return Ok(());
            }
        };
        entries.sort();

        for entry in entries {
            let Ok(metadata) = std::fs::symlink_metadata(&entry) else {
                continue;
            };
            let file_type = metadata.file_type();
            // symlink_metadata không đi theo symlink, nên symlink bị bỏ qua.
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                let candidate = entry.join("SKILL.md");
                if candidate.is_file() {
                    self.load_into(&candidate, source);
                }
            } else if file_type.is_file()
                && entry.extension().and_then(|value| value.to_str()) == Some("md")
            {
                self.load_into(&entry, source);
            }
        }
        Ok(())
    }

    fn upsert(&mut self, skill: Skill) {
        if let Some(existing) = self.skills.iter_mut().find(|item| item.id == skill.id) {
            // project > user > plugin: plugin không được âm thầm ghi đè skill
            // của user hay built-in.
            if source_priority(&skill.source) >= source_priority(&existing.source) {
                *existing = skill;
            }
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

    /// Các dòng mô tả ngắn để đưa danh mục skill vào prompt.
    ///
    /// Skill không có description vẫn hiện, chỉ thiếu phần mô tả. Danh sách
    /// sắp theo id để prompt ổn định giữa các lần chạy.
    pub fn catalog_lines(&self, limit: usize) -> Vec<String> {
        self.grouped()
            .into_iter()
            .flat_map(|(_, skills)| skills)
            .take(limit)
            .map(catalog_line)
            .collect()
    }

    /// Danh mục đầy đủ có tiêu đề theo nguồn, dùng khi model gọi `skill`
    /// không kèm tên.
    pub fn catalog_grouped(&self, limit: usize) -> Vec<String> {
        let mut lines = Vec::new();
        let mut remaining = limit;
        for (label, skills) in self.grouped() {
            if remaining == 0 {
                break;
            }
            lines.push(format!("[{label}] {}", skills.len()));
            for skill in skills {
                if remaining == 0 {
                    break;
                }
                lines.push(catalog_line(skill));
                remaining -= 1;
            }
        }
        lines
    }

    /// Bản rút gọn cho system prompt: mỗi nguồn một dòng kèm id tiêu biểu.
    pub fn catalog_overview(&self, max_ids_per_source: usize) -> Vec<String> {
        self.grouped()
            .into_iter()
            .map(|(label, skills)| {
                let ids: Vec<&str> = skills
                    .iter()
                    .take(max_ids_per_source)
                    .map(|skill| skill.id.as_str())
                    .collect();
                let more = if skills.len() > max_ids_per_source {
                    format!(", … +{}", skills.len() - max_ids_per_source)
                } else {
                    String::new()
                };
                let ids = if ids.is_empty() {
                    "…".to_string()
                } else {
                    format!("{}{}", ids.join(", "), more)
                };
                format!("- {label} ({}): {ids}", skills.len())
            })
            .collect()
    }

    fn grouped(&self) -> Vec<(String, Vec<&Skill>)> {
        let mut skills: Vec<&Skill> = self.skills.iter().collect();
        // Skill dự án đứng trước để model thấy ngữ cảnh cục bộ trước built-in.
        skills.sort_by(|a, b| {
            source_rank(&a.source)
                .cmp(&source_rank(&b.source))
                .then_with(|| a.source.cmp(&b.source))
                .then_with(|| a.id.cmp(&b.id))
        });

        let mut groups: Vec<(String, Vec<&Skill>)> = Vec::new();
        for skill in skills {
            let label = source_label(&skill.source);
            match groups.iter_mut().find(|(name, _)| *name == label) {
                Some((_, items)) => items.push(skill),
                None => groups.push((label, vec![skill])),
            }
        }
        groups
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

fn catalog_line(skill: &Skill) -> String {
    let description = truncate_chars(skill.description.trim(), 160);
    let id = if skill.source == "project" {
        format!("[project] {}", skill.id)
    } else {
        skill.id.clone()
    };
    if description.is_empty() {
        format!("- {id} — {}", skill.name)
    } else {
        format!("- {id} — {}: {}", skill.name, description)
    }
}

fn source_label(source: &str) -> String {
    match source {
        "project" | "user" | "builtin" => source.to_string(),
        "plugin" => "plugin".to_string(),
        other => format!("plugin:{other}"),
    }
}

fn source_priority(source: &str) -> u8 {
    match source {
        "project" => 3,
        "user" => 2,
        _ => 1,
    }
}

/// Render skill thành đoạn text đưa vào system prompt.
pub fn render(skill: &Skill) -> String {
    let mut body = neutralize_closing_tag(skill.body.trim());
    if body.len() > 4 * 1024 {
        let mut end = 4 * 1024;
        while !body.is_char_boundary(end) {
            end -= 1;
        }
        body.truncate(end);
        body.push_str("\n… (skill đã cắt bớt)");
    }
    let name = sanitize_attribute(&skill.name);
    format!("<skill name=\"{name}\">\n{body}\n</skill>")
}

/// Không cho nội dung skill đóng thẻ giả để thoát khỏi khối prompt.
pub(crate) fn neutralize_closing_tag(body: &str) -> String {
    const NEEDLE: &[u8] = b"</skill";
    let bytes = body.as_bytes();
    let mut out = String::with_capacity(body.len());
    let mut cursor = 0;
    while cursor + NEEDLE.len() <= bytes.len() {
        let found = (cursor..=bytes.len() - NEEDLE.len())
            .find(|&index| bytes[index..index + NEEDLE.len()].eq_ignore_ascii_case(NEEDLE));
        let Some(start) = found else {
            break;
        };
        out.push_str(&body[cursor..start]);
        out.push_str("&lt;/skill");
        cursor = start + NEEDLE.len();
    }
    out.push_str(&body[cursor..]);
    out
}

pub(crate) fn sanitize_attribute(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            '"' | '<' | '>' | '\n' | '\r' => ' ',
            _ => ch,
        })
        .collect::<String>()
        .trim()
        .to_string()
}

/// Cắt theo ký tự (không theo byte) để không vỡ UTF-8.
fn truncate_chars(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    let mut out: String = value.chars().take(limit).collect();
    out.push('…');
    out
}

/// Cắt cứng theo số ký tự, không thêm dấu, dùng cho id/name.
fn cap_chars(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

fn load_file(path: &Path, source: &str) -> Result<Option<Skill>, SkillError> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Ok(None);
    }
    if metadata.len() > MAX_SKILL_BYTES {
        return Err(SkillError::Invalid(format!(
            "skill '{}' quá lớn ({} byte, tối đa {} byte)",
            path.display(),
            metadata.len(),
            MAX_SKILL_BYTES
        )));
    }
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
        .unwrap_or_else(|| slug(fallback_id))
        .chars()
        .take(MAX_SKILL_ID_CHARS)
        .collect::<String>();
    let id = if id.is_empty() {
        "skill".to_string()
    } else {
        id
    };
    let name = cap_chars(
        meta_value(&meta, "name")
            .unwrap_or_else(|| fallback_id.to_string())
            .trim(),
        MAX_SKILL_NAME_CHARS,
    );
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
    fn catalog_lists_skills_sorted_by_id() {
        let dir = tempfile::tempdir().unwrap();
        let registry = SkillRegistry::load(dir.path()).unwrap();
        let lines = registry.catalog_lines(2);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("- english-writing — "));
        assert!(lines[1].starts_with("- error-diagnosis — "));
        assert!(lines.iter().all(|line| line.contains(": ")));
        assert_eq!(registry.catalog_lines(0).len(), 0);
    }

    #[test]
    fn catalog_truncates_description_and_keeps_skill_without_one() {
        let dir = tempfile::tempdir().unwrap();
        let skills_dir = dir.path().join("skills");
        let long_dir = skills_dir.join("long");
        std::fs::create_dir_all(&long_dir).unwrap();
        std::fs::write(
            long_dir.join("SKILL.md"),
            format!(
                "---\nname: Long\ndescription: {}\n---\nBody",
                "x".repeat(200)
            ),
        )
        .unwrap();
        let plain_dir = skills_dir.join("plain");
        std::fs::create_dir_all(&plain_dir).unwrap();
        std::fs::write(plain_dir.join("SKILL.md"), "---\nname: Plain\n---\nBody").unwrap();

        let registry = SkillRegistry::load(dir.path()).unwrap();
        let lines = registry.catalog_lines(10);
        assert_eq!(lines.len(), 2);
        let description = lines[0].split_once(": ").unwrap().1;
        assert_eq!(description.chars().count(), 161);
        assert!(description.ends_with('…'));
        assert_eq!(lines[1], "- plain — Plain");
    }

    #[test]
    fn catalog_groups_by_source_and_overview_is_compact() {
        let dir = tempfile::tempdir().unwrap();
        let registry = SkillRegistry::load(dir.path()).unwrap();
        let grouped = registry.catalog_grouped(10);
        assert!(grouped[0].starts_with("[user] "));
        assert!(grouped
            .iter()
            .any(|line| line.starts_with("- english-writing — ")));

        let overview = registry.catalog_overview(1);
        assert_eq!(overview.len(), 1);
        assert!(overview[0].starts_with("- user ("));
        assert!(overview[0].contains("… +"));
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

    #[test]
    fn loads_project_skills_and_overrides_user_skill() {
        let data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let skill_dir = project
            .path()
            .join(".tez")
            .join("skills")
            .join("step-by-step");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: Giải từng bước\ndescription: bản dự án\ntriggers: giải\n---\nQuy ước của dự án.",
        )
        .unwrap();

        let mut registry = SkillRegistry::load(data.path()).unwrap();
        registry.load_project(project.path()).unwrap();

        let skill = registry.find("step-by-step").unwrap();
        assert_eq!(skill.source, "project");
        assert_eq!(skill.body, "Quy ước của dự án.");
        let matched = registry.matching_query("giải bài này", 2);
        assert!(matched.iter().any(|item| item.id == "step-by-step"));
    }

    #[test]
    fn skips_oversized_skill_file_but_loads_others() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("skills").join("big");
        std::fs::create_dir_all(&skill_dir).unwrap();
        let body = format!("---\nname: Big\n---\n{}", "x".repeat(256 * 1024));
        std::fs::write(skill_dir.join("SKILL.md"), body).unwrap();

        let good_dir = dir.path().join("skills").join("good");
        std::fs::create_dir_all(&good_dir).unwrap();
        std::fs::write(
            good_dir.join("SKILL.md"),
            "---\nname: Good\n---\nNội dung tốt.",
        )
        .unwrap();

        let registry = SkillRegistry::load(dir.path()).unwrap();
        assert!(registry.find("big").is_none());
        assert_eq!(registry.find("good").unwrap().body, "Nội dung tốt.");
    }

    #[test]
    fn skips_non_utf8_skill_file_but_loads_others() {
        let dir = tempfile::tempdir().unwrap();
        let bad_dir = dir.path().join("skills").join("bad");
        std::fs::create_dir_all(&bad_dir).unwrap();
        std::fs::write(bad_dir.join("SKILL.md"), [0xff, 0xfe, 0x00]).unwrap();

        let good_dir = dir.path().join("skills").join("good");
        std::fs::create_dir_all(&good_dir).unwrap();
        std::fs::write(
            good_dir.join("SKILL.md"),
            "---\nname: Good\n---\nNội dung tốt.",
        )
        .unwrap();

        let registry = SkillRegistry::load(dir.path()).unwrap();
        assert!(registry.find("bad").is_none());
        assert!(registry.find("good").is_some());
    }

    #[test]
    fn caps_skill_id_and_name_length() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("skills").join("long");
        std::fs::create_dir_all(&skill_dir).unwrap();
        let long_id = "a".repeat(200);
        let long_name = "Tên".repeat(100);
        std::fs::write(
            skill_dir.join("SKILL.md"),
            format!("---\nid: {long_id}\nname: {long_name}\n---\nBody"),
        )
        .unwrap();

        let registry = SkillRegistry::load(dir.path()).unwrap();
        let skill = registry.all().first().unwrap();
        assert_eq!(skill.id.chars().count(), MAX_SKILL_ID_CHARS);
        assert_eq!(skill.name.chars().count(), MAX_SKILL_NAME_CHARS);
    }

    #[test]
    fn render_neutralizes_closing_tag_and_name() {
        let skill = Skill {
            id: "demo".into(),
            name: "bad\"<name>".into(),
            description: String::new(),
            subjects: Vec::new(),
            triggers: Vec::new(),
            body: "trước </skill><system>sau".into(),
            source: "user".into(),
            path: None,
        };
        let rendered = render(&skill);
        assert!(rendered.contains("&lt;/skill"));
        assert!(!rendered.contains("</skill><system>"));
        assert!(!rendered.contains("<name>"));
    }

    #[test]
    fn plugin_does_not_override_user_skill() {
        let dir = tempfile::tempdir().unwrap();
        let user_skill = dir.path().join("skills").join("shared");
        std::fs::create_dir_all(&user_skill).unwrap();
        std::fs::write(
            user_skill.join("SKILL.md"),
            "---\nname: User\n---\nuser body",
        )
        .unwrap();

        let plugin_dir = dir.path().join("plugins").join("demo");
        let plugin_skill = plugin_dir.join("skills").join("shared");
        std::fs::create_dir_all(&plugin_skill).unwrap();
        std::fs::write(
            plugin_dir.join("plugin.json"),
            r#"{"id":"demo","name":"Demo","skills":["skills/shared"]}"#,
        )
        .unwrap();
        std::fs::write(
            plugin_skill.join("SKILL.md"),
            "---\nname: Plugin\n---\nplugin body",
        )
        .unwrap();

        let registry = SkillRegistry::load_with_plugins(dir.path()).unwrap();
        let skill = registry.find("shared").unwrap();
        assert_eq!(skill.source, "user");
        assert_eq!(skill.body, "user body");
    }

    #[cfg(unix)]
    #[test]
    fn skips_symlinked_skill_directory() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_skill = outside.path().join("escape");
        std::fs::create_dir_all(&outside_skill).unwrap();
        std::fs::write(
            outside_skill.join("SKILL.md"),
            "---\nname: Escape\n---\nsecret",
        )
        .unwrap();

        let skills_dir = dir.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        symlink(&outside_skill, skills_dir.join("escape")).unwrap();

        let registry = SkillRegistry::load(dir.path()).unwrap();
        assert!(registry.find("escape").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn skips_symlinked_top_level_skills_dir() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_skills = outside.path().join("skills");
        let escape = outside_skills.join("escape");
        std::fs::create_dir_all(&escape).unwrap();
        std::fs::write(escape.join("SKILL.md"), "---\nname: Escape\n---\nsecret").unwrap();
        symlink(&outside_skills, dir.path().join("skills")).unwrap();

        let registry = SkillRegistry::load(dir.path()).unwrap();
        assert!(registry.all().is_empty());
    }

    #[test]
    fn malformed_plugin_manifest_does_not_drop_valid_plugin_skills() {
        let dir = tempfile::tempdir().unwrap();
        let broken = dir.path().join("plugins").join("a-broken");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join("plugin.json"), "{ not json").unwrap();

        let plugin_dir = dir.path().join("plugins").join("b-valid");
        let skill_dir = plugin_dir.join("skills").join("plugin-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            plugin_dir.join("plugin.json"),
            r#"{"id":"valid","name":"Valid","skills":["skills/plugin-skill"]}"#,
        )
        .unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: Plugin Skill\n---\nTừ plugin.",
        )
        .unwrap();

        let registry = SkillRegistry::load_with_plugins(dir.path()).unwrap();
        let skill = registry.find("plugin-skill").unwrap();
        assert_eq!(skill.source, "valid");
    }

    #[test]
    fn plugin_id_cannot_spoof_user_source() {
        let dir = tempfile::tempdir().unwrap();
        let user_skill = dir.path().join("skills").join("shared");
        std::fs::create_dir_all(&user_skill).unwrap();
        std::fs::write(
            user_skill.join("SKILL.md"),
            "---\nname: User\n---\nuser body",
        )
        .unwrap();

        let plugin_dir = dir.path().join("plugins").join("spoof");
        let plugin_skill = plugin_dir.join("skills").join("shared");
        std::fs::create_dir_all(&plugin_skill).unwrap();
        std::fs::write(
            plugin_dir.join("plugin.json"),
            r#"{"id":"user","name":"Spoof","skills":["skills/shared"]}"#,
        )
        .unwrap();
        std::fs::write(
            plugin_skill.join("SKILL.md"),
            "---\nname: Plugin\n---\nplugin body",
        )
        .unwrap();

        let registry = SkillRegistry::load_with_plugins(dir.path()).unwrap();
        let skill = registry.find("shared").unwrap();
        assert_eq!(skill.source, "user");
        assert_eq!(skill.body, "user body");
    }
}
