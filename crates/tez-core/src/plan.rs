//! Kế hoạch của phiên Plan được lưu thành file để chuyển sang Code mà không
//! phụ thuộc vào lịch sử chat hoặc todos.
//!
//! Mỗi phiên có một file `plans/<session>.md`. Tên file được làm sạch để không
//! thể thoát khỏi thư mục plans. Chỉ lượt chạy kết thúc bình thường mới được
//! ghi đè kế hoạch; Stop hoặc chạm giới hạn bước không được thay kế hoạch tốt
//! bằng thông báo dừng.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::agent::{AgentOutcome, StopReason};

/// Giới hạn kế hoạch để file và prompt không phình quá mức.
pub const MAX_PLAN_CHARS: usize = 200_000;

/// Các mục bắt buộc của một kế hoạch có cấu trúc.
pub const PLAN_SECTIONS: &[&str] = &[
    "Mục tiêu",
    "Phạm vi",
    "File và thay đổi",
    "Các bước",
    "Rủi ro",
    "Kiểm chứng",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanSection {
    pub title: String,
    pub content: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanDocument {
    pub sections: Vec<PlanSection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanValidation {
    pub valid: bool,
    pub missing: Vec<String>,
}

impl PlanDocument {
    /// Tách kế hoạch Markdown theo heading cấp 2 (`##`). Heading cấp 3 vẫn nằm
    /// trong nội dung của mục cha.
    pub fn parse(text: &str) -> Self {
        let mut sections: Vec<PlanSection> = Vec::new();
        let mut current: Option<(String, Vec<&str>)> = None;

        for line in text.lines() {
            if let Some(title) = heading_title(line) {
                if let Some((title, lines)) = current.take() {
                    sections.push(PlanSection {
                        title,
                        content: lines.join("\n").trim().to_string(),
                    });
                }
                current = Some((title, Vec::new()));
            } else if let Some((_, lines)) = current.as_mut() {
                lines.push(line);
            }
        }

        if let Some((title, lines)) = current {
            sections.push(PlanSection {
                title,
                content: lines.join("\n").trim().to_string(),
            });
        }

        Self { sections }
    }

    pub fn section(&self, title: &str) -> Option<&PlanSection> {
        let wanted = normalize_title(title);
        self.sections
            .iter()
            .find(|section| normalize_title(&section.title) == wanted)
    }

    /// Mục bị thiếu hoặc có nội dung rỗng.
    pub fn missing(&self) -> Vec<String> {
        PLAN_SECTIONS
            .iter()
            .filter(|title| {
                self.section(title)
                    .map(|section| section.content.trim().is_empty())
                    .unwrap_or(true)
            })
            .map(|title| (*title).to_string())
            .collect()
    }

    pub fn is_complete(&self) -> bool {
        self.missing().is_empty()
    }
}

/// Kiểm tra nhanh văn bản kế hoạch mà không cần giữ `PlanDocument`.
pub fn validate(text: &str) -> PlanValidation {
    let document = PlanDocument::parse(text);
    let missing = document.missing();
    PlanValidation {
        valid: missing.is_empty(),
        missing,
    }
}

fn heading_title(line: &str) -> Option<String> {
    let rest = line.trim().strip_prefix("## ")?;
    let title = rest.trim().trim_end_matches(':').trim();
    if title.is_empty() {
        None
    } else {
        Some(title.to_string())
    }
}

fn normalize_title(title: &str) -> String {
    title.trim().trim_end_matches(':').trim().to_lowercase()
}

/// Đường dẫn file kế hoạch của một phiên.
pub fn plan_path(dir: impl AsRef<Path>, session_id: &str) -> PathBuf {
    dir.as_ref()
        .join("plans")
        .join(format!("{}.md", safe_name(session_id)))
}

/// Lưu kế hoạch; văn bản rỗng thì không tạo file.
pub fn save_plan(
    dir: impl AsRef<Path>,
    session_id: &str,
    text: &str,
) -> io::Result<Option<PathBuf>> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }

    let path = plan_path(dir, session_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let capped: String = text.chars().take(MAX_PLAN_CHARS).collect();
    fs::write(&path, capped)?;
    Ok(Some(path))
}

/// Chỉ lưu khi lượt chạy hoàn thành bình thường.
pub fn save_plan_if_completed(
    dir: impl AsRef<Path>,
    session_id: &str,
    outcome: &AgentOutcome,
) -> io::Result<Option<PathBuf>> {
    if outcome.stop_reason != StopReason::Completed {
        return Ok(None);
    }
    save_plan(dir, session_id, &outcome.text)
}

/// Đọc kế hoạch đã lưu; chưa có file thì trả `None`.
pub fn load_plan(dir: impl AsRef<Path>, session_id: &str) -> io::Result<Option<String>> {
    match fs::read_to_string(plan_path(dir, session_id)) {
        Ok(text) => Ok(Some(text.chars().take(MAX_PLAN_CHARS).collect())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn safe_name(session_id: &str) -> String {
    let mut name = String::new();
    for character in session_id.chars() {
        if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
            name.push(character);
        } else {
            name.push('_');
        }
        if name.len() >= 120 {
            break;
        }
    }
    if name.is_empty() {
        "session".to_string()
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(stop_reason: StopReason, text: &str) -> AgentOutcome {
        AgentOutcome {
            text: text.to_string(),
            stop_reason,
            ..AgentOutcome::default()
        }
    }

    #[test]
    fn saves_and_loads_plan() {
        let dir = tempfile::tempdir().unwrap();
        let path = save_plan(dir.path(), "sess-1", "  Kế hoạch\n- bước 1  ")
            .unwrap()
            .unwrap();
        assert!(path.ends_with("plans/sess-1.md"));
        assert_eq!(
            load_plan(dir.path(), "sess-1").unwrap().unwrap(),
            "Kế hoạch\n- bước 1"
        );
    }

    #[test]
    fn empty_plan_is_not_saved() {
        let dir = tempfile::tempdir().unwrap();
        assert!(save_plan(dir.path(), "sess-1", "   \n").unwrap().is_none());
        assert!(load_plan(dir.path(), "sess-1").unwrap().is_none());
    }

    #[test]
    fn stopped_turn_does_not_overwrite_good_plan() {
        let dir = tempfile::tempdir().unwrap();
        save_plan(dir.path(), "sess-1", "Kế hoạch tốt\n- bước 1").unwrap();

        let stopped = outcome(StopReason::Stopped, "Đã dừng theo yêu cầu.");
        assert!(save_plan_if_completed(dir.path(), "sess-1", &stopped)
            .unwrap()
            .is_none());
        assert_eq!(
            load_plan(dir.path(), "sess-1").unwrap().unwrap(),
            "Kế hoạch tốt\n- bước 1"
        );

        let max_steps = outcome(StopReason::MaxSteps, "Đã dừng sau 12 bước");
        assert!(save_plan_if_completed(dir.path(), "sess-1", &max_steps)
            .unwrap()
            .is_none());
        assert_eq!(
            load_plan(dir.path(), "sess-1").unwrap().unwrap(),
            "Kế hoạch tốt\n- bước 1"
        );
    }

    #[test]
    fn completed_turn_saves_plan() {
        let dir = tempfile::tempdir().unwrap();
        let completed = outcome(StopReason::Completed, "Kế hoạch mới");
        let path = save_plan_if_completed(dir.path(), "sess-1", &completed)
            .unwrap()
            .unwrap();
        assert!(path.ends_with("plans/sess-1.md"));
        assert_eq!(
            load_plan(dir.path(), "sess-1").unwrap().unwrap(),
            "Kế hoạch mới"
        );
    }

    #[test]
    fn session_name_cannot_escape_plans_dir() {
        let dir = tempfile::tempdir().unwrap();
        let path = plan_path(dir.path(), "../../etc/passwd");
        assert_eq!(path.parent(), Some(dir.path().join("plans").as_path()));
        assert!(!path.to_string_lossy().contains(".."));
        assert!(path.to_string_lossy().ends_with("______etc_passwd.md"));
    }

    #[test]
    fn long_plan_is_capped() {
        let dir = tempfile::tempdir().unwrap();
        let text = "x".repeat(MAX_PLAN_CHARS + 10);
        save_plan(dir.path(), "sess-1", &text).unwrap();
        let loaded = load_plan(dir.path(), "sess-1").unwrap().unwrap();
        assert_eq!(loaded.chars().count(), MAX_PLAN_CHARS);
    }

    #[test]
    fn parses_structured_plan_sections() {
        let text = "## Mục tiêu\nThêm test\n\n## Phạm vi\nChỉ tez-core\n\n\
                    ## File và thay đổi\nplan.rs\n\n## Các bước\n1. Viết test\n2. Chạy test\n\n\
                    ## Rủi ro\nKhông có\n\n## Kiểm chứng\ncargo test\n";
        let document = PlanDocument::parse(text);
        assert_eq!(document.sections.len(), 6);
        assert!(document.is_complete());
        assert_eq!(
            document.section("Các bước").unwrap().content,
            "1. Viết test\n2. Chạy test"
        );
        assert_eq!(document.section("Mục tiêu").unwrap().content, "Thêm test");
    }

    #[test]
    fn validation_reports_missing_or_empty_sections() {
        let text = "## Mục tiêu\n\n## Các bước\nLàm việc\n### Ghi chú\nvẫn thuộc bước\n";
        let validation = validate(text);
        assert!(!validation.valid);
        assert!(validation.missing.contains(&"Mục tiêu".to_string()));
        assert!(validation.missing.contains(&"Phạm vi".to_string()));
        assert!(!validation.missing.contains(&"Các bước".to_string()));
        assert!(!validation.missing.contains(&"Ghi chú".to_string()));
    }

    #[test]
    fn validation_is_case_and_colon_insensitive() {
        let text = "## mục tiêu:\nX\n## PHẠM VI\nY\n## file và thay đổi\nZ\n\
                    ## Các bước\n1\n## Rủi ro\nR\n## Kiểm chứng\nK\n";
        assert!(validate(text).valid);
    }
}
