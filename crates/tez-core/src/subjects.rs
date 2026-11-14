//! Subject packs — cấu hình môn học dùng chung cho CLI, desktop và agent.
//!
//! Pack ở đây là bản compile-time, mỏng và ổn định. Sau này có thể load
//! `subjects/<id>/subject.toml` để giáo viên sửa mà không cần build lại.

/// Một môn học: tên, phong cách dạy, prompt và danh sách tool được phép.
#[derive(Debug, Clone, Copy)]
pub struct SubjectPack {
    pub id: &'static str,
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub strategy: &'static str,
    pub tutor_prompt: &'static str,
    pub tools: &'static [&'static str],
}

impl SubjectPack {
    /// Prompt system đầy đủ cho môn này.
    pub fn system_prompt(&self) -> String {
        format!(
            "Bạn là gia sư môn {}. Chiến lược dạy: {}.\n{}",
            self.name, self.strategy, self.tutor_prompt
        )
    }
}

pub const MATH: SubjectPack = SubjectPack {
    id: "math",
    name: "Toán",
    aliases: &["toan", "toán", "math", "mathematics"],
    strategy: "socratic",
    tutor_prompt: "Hướng dẫn từng bước, hỏi học sinh trước khi chốt đáp án. \
        Khi học sinh làm sai, chỉ ra chỗ sai cụ thể thay vì đưa lời giải ngay. \
        Dùng tool để tính toán và kiểm tra từng bước, không tính nhẩm các biểu thức dài. \
        Khi bài có đồ thị, dùng plot_function để minh họa.",
    tools: &["calculate", "check_step", "plot_function"],
};

pub const PHYSICS: SubjectPack = SubjectPack {
    id: "physics",
    name: "Vật lý",
    aliases: &["ly", "lý", "vat ly", "vật lý", "physics", "physic"],
    strategy: "socratic",
    tutor_prompt: "Luôn kiểm tra đơn vị và đại lượng đã cho trước khi tính. \
        Viết công thức, thay số, rồi mới ra kết quả. Dùng convert_units khi đổi đơn vị \
        và physics_formula khi cần nhắc công thức. Chỉ rõ đơn vị trong kết quả cuối.",
    tools: &[
        "calculate",
        "check_step",
        "convert_units",
        "physics_formula",
    ],
};

pub const CHEMISTRY: SubjectPack = SubjectPack {
    id: "chemistry",
    name: "Hóa học",
    aliases: &["hoa", "hóa", "hoa hoc", "hóa học", "chemistry", "chem"],
    strategy: "socratic",
    tutor_prompt: "Cân bằng phương trình trước khi tính toán định lượng. \
        Dùng molar_mass để tính khối lượng mol và balance_equation để cân bằng phương trình. \
        Giải thích rõ tỉ lệ mol giữa các chất. Không tự bịa công thức hóa học.",
    tools: &["calculate", "molar_mass", "balance_equation"],
};

pub const ENGLISH: SubjectPack = SubjectPack {
    id: "english",
    name: "Tiếng Anh",
    aliases: &["anh", "tieng anh", "tiếng anh", "english", "eng"],
    strategy: "coach",
    tutor_prompt: "Dạy tiếng Anh theo hướng giao tiếp và sửa lỗi nhẹ nhàng. \
        Khi học sinh viết, chỉ ra lỗi ngữ pháp/chính tả cụ thể, đưa câu sửa mẫu, \
        rồi giải thích ngắn bằng tiếng Việt. Dùng check_grammar cho lỗi cơ bản và \
        readability để đánh giá độ khó của đoạn văn. Khuyến khích học sinh viết lại.",
    tools: &["check_grammar", "readability"],
};

pub const ALL: &[SubjectPack] = &[MATH, PHYSICS, CHEMISTRY, ENGLISH];

/// Chuẩn hóa chuỗi để so khớp: chữ thường, bỏ khoảng trắng thừa.
fn normalize(input: &str) -> String {
    input.trim().to_lowercase()
}

/// Tìm pack theo id, tên tiếng Việt hoặc alias. Không phân biệt hoa thường.
pub fn resolve(input: &str) -> Option<&'static SubjectPack> {
    let needle = normalize(input);
    ALL.iter().find(|pack| {
        pack.id == needle
            || normalize(pack.name) == needle
            || pack.aliases.iter().any(|alias| normalize(alias) == needle)
    })
}

pub fn get(id: &str) -> Option<&'static SubjectPack> {
    ALL.iter().find(|pack| pack.id == normalize(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_ids_names_and_aliases() {
        assert_eq!(resolve("math").unwrap().id, "math");
        assert_eq!(resolve("Toán").unwrap().id, "math");
        assert_eq!(resolve("toan").unwrap().id, "math");
        assert_eq!(resolve("Lý").unwrap().id, "physics");
        assert_eq!(resolve("hoa hoc").unwrap().id, "chemistry");
        assert_eq!(resolve("English").unwrap().id, "english");
    }

    #[test]
    fn unknown_subject_is_none() {
        assert!(resolve("sinh học").is_none());
    }

    #[test]
    fn every_pack_has_prompt_and_tools() {
        for pack in ALL {
            assert!(!pack.tutor_prompt.is_empty());
            assert!(!pack.tools.is_empty());
            assert!(resolve(pack.name).is_some());
        }
    }
}
