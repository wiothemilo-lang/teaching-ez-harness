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

use crate::agent::{AgentOutcome, StopReason};

/// Giới hạn kế hoạch để file và prompt không phình quá mức.
pub const MAX_PLAN_CHARS: usize = 200_000;

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
}
