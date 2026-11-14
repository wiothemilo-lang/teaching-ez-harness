//! Hai chế độ làm việc của tez: Chat (trò chuyện tổng quát) và Code (trợ lý lập trình).

use std::path::Path;

/// Chế độ của một phiên làm việc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Trò chuyện, hỏi đáp, học tập — không giới hạn môn học.
    #[default]
    Chat,
    /// Trợ lý lập trình: đọc/sửa file và chạy lệnh trong một workspace.
    Code,
}

impl Mode {
    /// Parse từ chuỗi người dùng/DB. Giá trị lạ rơi về `Chat`.
    pub fn parse(value: &str) -> Self {
        Self::try_parse(value).unwrap_or_default()
    }

    /// Parse chặt: trả `None` với giá trị không được hỗ trợ (dùng để báo lỗi CLI).
    pub fn try_parse(value: &str) -> Option<Self> {
        match value.trim().to_lowercase().as_str() {
            "chat" => Some(Self::Chat),
            "code" | "coding" | "dev" => Some(Self::Code),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Code => "code",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Chat => "Chat",
            Self::Code => "Code",
        }
    }
}

/// Tool được phép dùng trong chế độ Code.
pub const CODE_TOOLS: &[&str] = &[
    "read_file",
    "write_file",
    "edit_file",
    "list_files",
    "glob",
    "grep",
    "run_command",
    "todo_write",
    "task",
    "compact",
    "memory_save",
    "memory_search",
    "memory_forget",
];

/// Prompt system cho trợ lý lập trình, gắn với một workspace cụ thể.
pub fn code_system_prompt(workspace: &Path) -> String {
    format!(
        "Bạn là trợ lý lập trình trong tez, chạy local trên máy người dùng.\n\
         Workspace hiện tại: {workspace}.\n\n\
         Nguyên tắc:\n\
         - Trả lời bằng tiếng Việt, ngắn gọn; code và tên định danh giữ nguyên tiếng Anh.\n\
         - Luôn đọc file liên quan trước khi sửa. Không đoán nội dung file.\n\
         - Ưu tiên thay đổi nhỏ, đúng phong cách code xung quanh; không refactor ngoài phạm vi yêu cầu.\n\
         - Dùng read_file/list_files/glob/grep để khảo sát, edit_file để sửa chính xác, write_file khi tạo file mới, run_command để build/test.\n\
         - Với việc nhiều bước, gọi todo_write để lập kế hoạch và cập nhật trạng thái.\n\
         - Khi cần khảo sát nhiều file hoặc tách một nhánh công việc, dùng task để giao cho subagent.\n\
         - Chỉ thao tác trong workspace. Không đọc hoặc ghi file nằm ngoài workspace.\n\
         - Không chạy lệnh phá hủy (rm -rf, format, drop database, git push --force) khi chưa được yêu cầu rõ ràng.\n\
         - Không đưa bí mật, API key hoặc token vào code, log hay câu trả lời.\n\
         - Khi xong, tóm tắt ngắn những gì đã thay đổi và cách kiểm tra.\n",
        workspace = workspace.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modes() {
        assert_eq!(Mode::parse("code"), Mode::Code);
        assert_eq!(Mode::parse(" CODE "), Mode::Code);
        assert_eq!(Mode::parse("chat"), Mode::Chat);
        assert_eq!(Mode::parse(""), Mode::Chat);
        assert_eq!(Mode::default(), Mode::Chat);
        assert_eq!(Mode::Code.as_str(), "code");
        assert_eq!(Mode::try_parse("code"), Some(Mode::Code));
        assert_eq!(Mode::try_parse("chat"), Some(Mode::Chat));
        assert_eq!(Mode::try_parse("bogus"), None);
    }

    #[test]
    fn code_prompt_mentions_workspace_and_safety() {
        let prompt = code_system_prompt(Path::new("/tmp/project"));
        assert!(prompt.contains("/tmp/project"));
        assert!(prompt.contains("read_file"));
        assert!(prompt.contains("Không chạy lệnh phá hủy"));
    }
}
