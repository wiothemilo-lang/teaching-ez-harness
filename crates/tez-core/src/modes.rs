//! Ba chế độ làm việc của tez: Chat (trò chuyện tổng quát), Plan (chỉ đọc và
//! lập kế hoạch) và Code (trợ lý lập trình).

use std::path::Path;

/// Chế độ của một phiên làm việc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Trò chuyện, hỏi đáp, học tập — không giới hạn môn học.
    #[default]
    Chat,
    /// Chỉ đọc workspace, khảo sát và trình kế hoạch trước khi thực thi.
    Plan,
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
            "plan" | "planning" | "plan_mode" | "plan-mode" => Some(Self::Plan),
            "code" | "coding" | "dev" => Some(Self::Code),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Plan => "plan",
            Self::Code => "code",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Chat => "Chat",
            Self::Plan => "Plan",
            Self::Code => "Code",
        }
    }
}

/// Tool được phép dùng trong chế độ Plan: chỉ đọc và ghi kế hoạch.
pub const PLAN_TOOLS: &[&str] = &[
    "read_file",
    "list_files",
    "glob",
    "grep",
    "todo_write",
    "memory_search",
];

/// Prompt system cho chế độ Plan.
pub fn plan_system_prompt(workspace: &Path) -> String {
    format!(
        "Bạn là agent lập kế hoạch trong tez, chạy local trên máy người dùng.\n\
         Workspace hiện tại: {workspace}.\n\n\
         Chế độ Plan chỉ đọc: không ghi file, không chạy lệnh, không gọi tool thay đổi trạng thái.\n\
         Nhiệm vụ: khảo sát workspace bằng read_file/list_files/glob/grep, xác định file và ràng buộc liên quan, \
         rồi trình một kế hoạch ngắn gọn để người dùng duyệt trước khi chuyển sang Code.\n\n\
         Kế hoạch phải dùng đúng các mục Markdown cấp 2 sau, theo thứ tự, mỗi mục có nội dung:\n\
         ## Mục tiêu\n\
         ## Phạm vi\n\
         ## File và thay đổi\n\
         ## Các bước\n\
         ## Rủi ro\n\
         ## Kiểm chứng\n\n\
         Trong đó: phạm vi nêu rõ việc không làm; file/thư mục sẽ đọc hoặc sửa kèm lý do; \
         các bước theo thứ tự, mỗi bước nhỏ và kiểm chứng được; rủi ro gồm điểm chưa chắc chắn \
         và câu hỏi cần người dùng trả lời; kiểm chứng ghi build, test hoặc lệnh cụ thể.\n\n\
         Dùng todo_write để lưu kế hoạch. Không tự chuyển sang thực thi; khi người dùng chuyển sang Code, \
         kế hoạch sẽ là nguồn chỉ dẫn.\n\
         Trả lời ngắn gọn; code và tên định danh giữ nguyên tiếng Anh.\n",
        workspace = workspace.display()
    )
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
         - Trả lời ngắn gọn; code và tên định danh giữ nguyên tiếng Anh.\n\
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
        assert_eq!(Mode::parse("plan"), Mode::Plan);
        assert_eq!(Mode::parse(""), Mode::Chat);
        assert_eq!(Mode::default(), Mode::Chat);
        assert_eq!(Mode::Code.as_str(), "code");
        assert_eq!(Mode::Plan.as_str(), "plan");
        assert_eq!(Mode::try_parse("code"), Some(Mode::Code));
        assert_eq!(Mode::try_parse("chat"), Some(Mode::Chat));
        assert_eq!(Mode::try_parse("plan"), Some(Mode::Plan));
        assert_eq!(Mode::try_parse("bogus"), None);
    }

    #[test]
    fn plan_prompt_is_read_only() {
        let prompt = plan_system_prompt(Path::new("/tmp/project"));
        assert!(prompt.contains("/tmp/project"));
        assert!(prompt.contains("Chế độ Plan chỉ đọc"));
        assert!(prompt.contains("## Mục tiêu"));
        assert!(prompt.contains("## Kiểm chứng"));
        assert!(prompt.contains("todo_write"));
        assert!(!PLAN_TOOLS.contains(&"write_file"));
        assert!(!PLAN_TOOLS.contains(&"run_command"));
    }

    #[test]
    fn code_prompt_mentions_workspace_and_safety() {
        let prompt = code_system_prompt(Path::new("/tmp/project"));
        assert!(prompt.contains("/tmp/project"));
        assert!(prompt.contains("read_file"));
        assert!(prompt.contains("Không chạy lệnh phá hủy"));
    }
}
