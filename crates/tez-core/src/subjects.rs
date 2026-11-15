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
    /// Pack tổng quát, không gắn với môn học nào.
    pub fn is_general(&self) -> bool {
        self.id == GENERAL.id
    }

    /// Prompt system đầy đủ cho môn này.
    pub fn system_prompt(&self) -> String {
        if self.is_general() {
            return self.tutor_prompt.to_string();
        }
        format!(
            "Bạn là gia sư môn {}. Chiến lược dạy: {}.\n{}",
            self.name, self.strategy, self.tutor_prompt
        )
    }
}

/// Pack mặc định: trợ lý tổng quát, không giới hạn môn học.
pub const GENERAL: SubjectPack = SubjectPack {
    id: "general",
    name: "Tổng quát",
    aliases: &["chung", "general", "assistant", "tro ly", "trợ lý", "any"],
    strategy: "assistant",
    tutor_prompt: "Bạn là trợ lý AI local-first, thân thiện và thẳng thắn. \
        Trả lời bằng tiếng Việt trừ khi người dùng dùng ngôn ngữ khác. \
        Không giả định người dùng là học sinh hay đang học môn nào: hãy thích nghi với câu hỏi. \
        Khi câu hỏi cần tính toán, tra cứu hoặc kiểm tra, dùng tool thay vì đoán. \
        Nếu không chắc, nói rõ mức độ chắc chắn. Không bịa nguồn hoặc số liệu.",
    tools: &["calculate"],
};

pub const MATH: SubjectPack = SubjectPack {
    id: "math",
    name: "Toán",
    aliases: &[
        "toan",
        "toán",
        "toan hoc",
        "toán học",
        "hinh hoc",
        "hình học",
        "dai so",
        "đại số",
        "math",
        "mathematics",
        "maths",
    ],
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
    aliases: &[
        "ly",
        "lý",
        "li",
        "lí",
        "vat ly",
        "vật lý",
        "vat li",
        "vật lí",
        "ly hoc",
        "lý học",
        "vat ly hoc",
        "vật lý học",
        "physics",
        "physic",
    ],
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
    aliases: &[
        "hoa",
        "hóa",
        "hoá",
        "hoa hoc",
        "hóa học",
        "hoá học",
        "chemistry",
        "chem",
    ],
    strategy: "socratic",
    tutor_prompt: "Cân bằng phương trình trước khi tính toán định lượng. \
        Dùng molar_mass để tính khối lượng mol và balance_equation để cân bằng phương trình. \
        Giải thích rõ tỉ lệ mol giữa các chất. Không tự bịa công thức hóa học.",
    tools: &["calculate", "molar_mass", "balance_equation"],
};

pub const ENGLISH: SubjectPack = SubjectPack {
    id: "english",
    name: "Tiếng Anh",
    aliases: &[
        "anh",
        "anh van",
        "anh văn",
        "tieng anh",
        "tiếng anh",
        "english",
        "eng",
    ],
    strategy: "coach",
    tutor_prompt: "Dạy tiếng Anh theo hướng giao tiếp và sửa lỗi nhẹ nhàng. \
        Khi học sinh viết, chỉ ra lỗi ngữ pháp/chính tả cụ thể, đưa câu sửa mẫu, \
        rồi giải thích ngắn bằng tiếng Việt. Dùng check_grammar cho lỗi cơ bản và \
        readability để đánh giá độ khó của đoạn văn. Khuyến khích học sinh viết lại.",
    tools: &["check_grammar", "readability"],
};

/// Mọi pack có sẵn. `GENERAL` đứng đầu và là lựa chọn mặc định.
pub const ALL: &[SubjectPack] = &[GENERAL, MATH, PHYSICS, CHEMISTRY, ENGLISH];

/// Các pack môn học (không gồm pack tổng quát).
pub const SUBJECTS: &[SubjectPack] = &[MATH, PHYSICS, CHEMISTRY, ENGLISH];

/// Chuẩn hóa chuỗi để so khớp: chữ thường, bỏ dấu tiếng Việt, gộp khoảng trắng.
///
/// Cách này xử lý cả NFC (`Toán`) và NFD (`Toa` + dấu sắc rời) vì dấu tổ hợp
/// bị loại bỏ sau khi chữ cái gốc được giữ lại.
fn normalize(input: &str) -> String {
    let folded: String = input
        .trim()
        .chars()
        .flat_map(char::to_lowercase)
        .filter_map(fold_char)
        .collect();
    folded.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_combining_mark(ch: char) -> bool {
    matches!(
        ch as u32,
        0x0300..=0x036f
            | 0x1ab0..=0x1aff
            | 0x1dc0..=0x1dff
            | 0x20d0..=0x20ff
            | 0xfe20..=0xfe2f
    )
}

/// Bỏ dấu các nguyên âm tiếng Việt (đã ở dạng chữ thường).
fn fold_char(ch: char) -> Option<char> {
    if is_combining_mark(ch) {
        return None;
    }
    let base = match ch {
        'à' | 'á' | 'ạ' | 'ả' | 'ã' | 'â' | 'ầ' | 'ấ' | 'ậ' | 'ẩ' | 'ẫ' | 'ă' | 'ằ' | 'ắ' | 'ặ'
        | 'ẳ' | 'ẵ' => 'a',
        'è' | 'é' | 'ẹ' | 'ẻ' | 'ẽ' | 'ê' | 'ề' | 'ế' | 'ệ' | 'ể' | 'ễ' => 'e',
        'ì' | 'í' | 'ị' | 'ỉ' | 'ĩ' => 'i',
        'ò' | 'ó' | 'ọ' | 'ỏ' | 'õ' | 'ô' | 'ồ' | 'ố' | 'ộ' | 'ổ' | 'ỗ' | 'ơ' | 'ờ' | 'ớ' | 'ợ'
        | 'ở' | 'ỡ' => 'o',
        'ù' | 'ú' | 'ụ' | 'ủ' | 'ũ' | 'ư' | 'ừ' | 'ứ' | 'ự' | 'ử' | 'ữ' => 'u',
        'ỳ' | 'ý' | 'ỵ' | 'ỷ' | 'ỹ' => 'y',
        'đ' => 'd',
        _ => ch,
    };
    Some(base)
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

/// Như [`resolve`] nhưng không bao giờ lỗi: chuỗi rỗng hoặc không khớp trả về
/// pack tổng quát. Dùng cho chat tự do để người dùng không bị gò bó vào môn học.
pub fn resolve_or_general(input: &str) -> &'static SubjectPack {
    if input.trim().is_empty() {
        return &GENERAL;
    }
    resolve(input).unwrap_or(&GENERAL)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_ids_names_and_aliases() {
        assert_eq!(resolve("math").unwrap().id, "math");
        assert_eq!(resolve("Toán").unwrap().id, "math");
        assert_eq!(resolve("toan").unwrap().id, "math");
        assert_eq!(resolve("toán học").unwrap().id, "math");
        assert_eq!(resolve("Lý").unwrap().id, "physics");
        assert_eq!(resolve("lí").unwrap().id, "physics");
        assert_eq!(resolve("vật lý").unwrap().id, "physics");
        assert_eq!(resolve("hoa hoc").unwrap().id, "chemistry");
        assert_eq!(resolve("hoá").unwrap().id, "chemistry");
        assert_eq!(resolve("English").unwrap().id, "english");
        assert_eq!(resolve("tiếng anh").unwrap().id, "english");
    }

    #[test]
    fn normalizes_decomposed_unicode() {
        // "Toán", "Hoá học" và "Vật lí" ở dạng NFD (dấu tách rời).
        assert_eq!(resolve("Toa\u{0301}n").unwrap().id, "math");
        assert_eq!(resolve("Hoa\u{0301} ho\u{0323}c").unwrap().id, "chemistry");
        assert_eq!(resolve("Va\u{0323}t li\u{0301}").unwrap().id, "physics");
    }

    #[test]
    fn unknown_subject_is_none_but_falls_back_to_general() {
        assert!(resolve("sinh học").is_none());
        assert_eq!(resolve_or_general("sinh học").id, "general");
        assert_eq!(resolve_or_general("").id, "general");
        assert_eq!(resolve_or_general("math").id, "math");
    }

    #[test]
    fn general_is_the_default_pack() {
        assert_eq!(ALL[0].id, "general");
        assert!(GENERAL.is_general());
        assert!(!MATH.is_general());
        assert!(!GENERAL.system_prompt().contains("gia sư môn"));
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
