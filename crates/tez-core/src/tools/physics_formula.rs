use async_trait::async_trait;
use serde_json::{json, Value};
use tez_llm::ToolSpec;

use super::{Tool, ToolError, ToolOutput};

struct Formula {
    name: &'static str,
    formula: &'static str,
    variables: &'static str,
    unit: &'static str,
    keywords: &'static str,
}

const FORMULAS: &[Formula] = &[
    Formula {
        name: "Chuyển động thẳng đều",
        formula: "s = v·t",
        variables: "s: quãng đường; v: vận tốc; t: thời gian",
        unit: "m, m/s, s",
        keywords: "chuyen dong thang deu van toc quang duong",
    },
    Formula {
        name: "Vận tốc trung bình",
        formula: "v = s / t",
        variables: "s: quãng đường; t: thời gian",
        unit: "m/s",
        keywords: "van toc trung binh speed",
    },
    Formula {
        name: "Gia tốc",
        formula: "a = (v - v₀) / t",
        variables: "v: vận tốc cuối; v₀: vận tốc đầu; t: thời gian",
        unit: "m/s²",
        keywords: "gia toc acceleration",
    },
    Formula {
        name: "Vận tốc trong chuyển động biến đổi đều",
        formula: "v = v₀ + a·t",
        variables: "v₀: vận tốc đầu; a: gia tốc; t: thời gian",
        unit: "m/s",
        keywords: "van toc bien doi deu gia toc",
    },
    Formula {
        name: "Quãng đường trong chuyển động biến đổi đều",
        formula: "s = v₀·t + ½·a·t²",
        variables: "v₀: vận tốc đầu; a: gia tốc; t: thời gian",
        unit: "m",
        keywords: "quang duong bien doi deu",
    },
    Formula {
        name: "Hệ thức độc lập thời gian",
        formula: "v² - v₀² = 2·a·s",
        variables: "v, v₀: vận tốc; a: gia tốc; s: quãng đường",
        unit: "m/s, m/s², m",
        keywords: "he thuc doc lap thoi gian",
    },
    Formula {
        name: "Rơi tự do",
        formula: "s = ½·g·t²; v = g·t",
        variables: "g ≈ 9,8 m/s²; t: thời gian rơi",
        unit: "m, m/s",
        keywords: "roi tu do free fall trong luc",
    },
    Formula {
        name: "Định luật II Newton",
        formula: "F = m·a",
        variables: "F: hợp lực; m: khối lượng; a: gia tốc",
        unit: "N, kg, m/s²",
        keywords: "luc khoi luong gia toc newton",
    },
    Formula {
        name: "Trọng lực",
        formula: "P = m·g",
        variables: "m: khối lượng; g ≈ 9,8 m/s²",
        unit: "N",
        keywords: "trong luc trong luong",
    },
    Formula {
        name: "Lực ma sát",
        formula: "Fms = μ·N",
        variables: "μ: hệ số ma sát; N: áp lực",
        unit: "N",
        keywords: "ma sat friction",
    },
    Formula {
        name: "Lực đàn hồi",
        formula: "F = k·|Δl|",
        variables: "k: độ cứng lò xo; Δl: độ biến dạng",
        unit: "N/m, m",
        keywords: "dan hoi lo xo hooke",
    },
    Formula {
        name: "Áp suất",
        formula: "p = F / S",
        variables: "F: áp lực; S: diện tích bị ép",
        unit: "Pa, N, m²",
        keywords: "ap suat pressure",
    },
    Formula {
        name: "Áp suất chất lỏng",
        formula: "p = ρ·g·h",
        variables: "ρ: khối lượng riêng; h: độ sâu",
        unit: "Pa, kg/m³, m",
        keywords: "ap suat chat long do sau",
    },
    Formula {
        name: "Lực đẩy Ác-si-mét",
        formula: "FA = ρ·V·g",
        variables: "ρ: khối lượng riêng chất lỏng; V: thể tích phần chìm",
        unit: "N, kg/m³, m³",
        keywords: "ac si met archimedes luc day",
    },
    Formula {
        name: "Công cơ học",
        formula: "A = F·s·cosα",
        variables: "F: lực; s: quãng đường; α: góc giữa lực và hướng dịch chuyển",
        unit: "J",
        keywords: "cong co hoc work",
    },
    Formula {
        name: "Công suất",
        formula: "P = A / t",
        variables: "A: công; t: thời gian",
        unit: "W",
        keywords: "cong suat power",
    },
    Formula {
        name: "Động năng",
        formula: "Wđ = ½·m·v²",
        variables: "m: khối lượng; v: vận tốc",
        unit: "J",
        keywords: "dong nang kinetic",
    },
    Formula {
        name: "Thế năng trọng trường",
        formula: "Wt = m·g·h",
        variables: "m: khối lượng; h: độ cao so với mốc",
        unit: "J",
        keywords: "the nang trong truong potential",
    },
    Formula {
        name: "Cơ năng",
        formula: "W = Wđ + Wt",
        variables: "Wđ: động năng; Wt: thế năng",
        unit: "J",
        keywords: "co nang mechanical energy",
    },
    Formula {
        name: "Bảo toàn cơ năng",
        formula: "Wđ₁ + Wt₁ = Wđ₂ + Wt₂",
        variables: "trạng thái 1 và 2 khi chỉ có trọng lực/lực đàn hồi tác dụng",
        unit: "J",
        keywords: "bao toan co nang conservation",
    },
    Formula {
        name: "Nhiệt lượng",
        formula: "Q = m·c·Δt",
        variables: "m: khối lượng; c: nhiệt dung riêng; Δt: độ tăng nhiệt độ",
        unit: "J, kg, J/(kg·K), °C",
        keywords: "nhiet luong nhiet dung heat",
    },
    Formula {
        name: "Phương trình trạng thái khí lý tưởng",
        formula: "p₁V₁/T₁ = p₂V₂/T₂",
        variables: "p: áp suất; V: thể tích; T: nhiệt độ tuyệt đối (K)",
        unit: "Pa, m³, K",
        keywords: "khi ly tuong trang thai ideal gas",
    },
    Formula {
        name: "Định luật Ohm",
        formula: "I = U / R",
        variables: "I: cường độ dòng điện; U: hiệu điện thế; R: điện trở",
        unit: "A, V, Ω",
        keywords: "ohm dien tro hieu dien the cuong do",
    },
    Formula {
        name: "Công suất điện",
        formula: "P = U·I = I²·R = U²/R",
        variables: "U: hiệu điện thế; I: cường độ; R: điện trở",
        unit: "W",
        keywords: "cong suat dien electrical power",
    },
    Formula {
        name: "Điện năng tiêu thụ",
        formula: "A = U·I·t = P·t",
        variables: "P: công suất; t: thời gian",
        unit: "J hoặc kWh",
        keywords: "dien nang tieu thu energy",
    },
    Formula {
        name: "Điện trở nối tiếp",
        formula: "Rtd = R₁ + R₂ + ...",
        variables: "các điện trở mắc nối tiếp",
        unit: "Ω",
        keywords: "noi tiep series dien tro",
    },
    Formula {
        name: "Điện trở song song",
        formula: "1/Rtd = 1/R₁ + 1/R₂ + ...",
        variables: "các điện trở mắc song song",
        unit: "Ω",
        keywords: "song song parallel dien tro",
    },
    Formula {
        name: "Định luật Jun-Len-xơ",
        formula: "Q = I²·R·t",
        variables: "I: cường độ; R: điện trở; t: thời gian",
        unit: "J",
        keywords: "jun len xo nhiet luong toa",
    },
    Formula {
        name: "Tần số sóng",
        formula: "f = 1 / T",
        variables: "T: chu kỳ",
        unit: "Hz, s",
        keywords: "tan so chu ky song frequency",
    },
    Formula {
        name: "Bước sóng",
        formula: "λ = v / f",
        variables: "v: tốc độ truyền sóng; f: tần số",
        unit: "m",
        keywords: "buoc song wavelength",
    },
    Formula {
        name: "Thấu kính mỏng",
        formula: "1/f = 1/d + 1/d'",
        variables: "f: tiêu cự; d: khoảng cách vật; d': khoảng cách ảnh",
        unit: "cm hoặc m",
        keywords: "thau kinh tieu cu anh vat",
    },
];

/// Bỏ dấu tiếng Việt để tìm kiếm dễ hơn.
fn normalize(input: &str) -> String {
    input
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'a' | 'à' | 'á' | 'ả' | 'ã' | 'ạ' | 'ă' | 'ằ' | 'ắ' | 'ẳ' | 'ẵ' | 'ặ' | 'â' | 'ầ'
            | 'ấ' | 'ẩ' | 'ẫ' | 'ậ' => 'a',
            'e' | 'è' | 'é' | 'ẻ' | 'ẽ' | 'ẹ' | 'ê' | 'ề' | 'ế' | 'ể' | 'ễ' | 'ệ' => {
                'e'
            }
            'i' | 'ì' | 'í' | 'ỉ' | 'ĩ' | 'ị' => 'i',
            'o' | 'ò' | 'ó' | 'ỏ' | 'õ' | 'ọ' | 'ô' | 'ồ' | 'ố' | 'ổ' | 'ỗ' | 'ộ' | 'ơ' | 'ờ'
            | 'ớ' | 'ở' | 'ỡ' | 'ợ' => 'o',
            'u' | 'ù' | 'ú' | 'ủ' | 'ũ' | 'ụ' | 'ư' | 'ừ' | 'ứ' | 'ử' | 'ữ' | 'ự' => {
                'u'
            }
            'y' | 'ỳ' | 'ý' | 'ỷ' | 'ỹ' | 'ỵ' => 'y',
            'd' | 'đ' => 'd',
            _ => c,
        })
        .collect()
}

/// Tra cứu công thức vật lý phổ thông theo tên hoặc từ khóa.
pub struct PhysicsFormula;

#[async_trait]
impl Tool for PhysicsFormula {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "physics_formula".into(),
            description: "Tra cứu công thức vật lý phổ thông theo tên hoặc từ khóa (động năng, Ohm, áp suất, rơi tự do...). Trả về công thức, đại lượng và đơn vị.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Tên công thức hoặc từ khóa, ví dụ 'dong nang' hoặc 'Ohm'"
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArgs("thiếu trường 'query'".into()))?;
        let needle = normalize(query.trim());
        if needle.is_empty() {
            return Ok(ToolOutput::error("cần nhập tên hoặc từ khóa".to_string()));
        }

        let matches: Vec<&Formula> = FORMULAS
            .iter()
            .filter(|formula| {
                normalize(formula.name).contains(&needle)
                    || formula.keywords.contains(&needle)
                    || normalize(formula.formula).contains(&needle)
            })
            .take(5)
            .collect();

        if matches.is_empty() {
            let names: Vec<&str> = FORMULAS.iter().map(|f| f.name).collect();
            return Ok(ToolOutput::error(format!(
                "không tìm thấy công thức cho '{query}'. Một số chủ đề: {}",
                names.join(", ")
            )));
        }

        let mut output = String::new();
        for (index, formula) in matches.iter().enumerate() {
            if index > 0 {
                output.push('\n');
            }
            output.push_str(&format!(
                "{}\n  Công thức: {}\n  Đại lượng: {}\n  Đơn vị: {}",
                formula.name, formula.formula, formula.variables, formula.unit
            ));
        }
        Ok(ToolOutput::ok(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn finds_kinetic_energy_without_diacritics() {
        let out = PhysicsFormula
            .execute(json!({"query": "dong nang"}))
            .await
            .unwrap();
        assert!(!out.is_error);
        assert!(out.content.contains("Wđ = ½·m·v²"));
    }

    #[tokio::test]
    async fn finds_ohm() {
        let out = PhysicsFormula
            .execute(json!({"query": "Ohm"}))
            .await
            .unwrap();
        assert!(!out.is_error);
        assert!(out.content.contains("I = U / R"));
    }

    #[tokio::test]
    async fn trims_surrounding_spaces_in_query() {
        let out = PhysicsFormula
            .execute(json!({"query": "  Ohm  "}))
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("I = U / R"));
    }

    #[tokio::test]
    async fn reports_unknown_topic() {
        let out = PhysicsFormula
            .execute(json!({"query": "quantum gravity"}))
            .await
            .unwrap();
        assert!(out.is_error);
    }
}
