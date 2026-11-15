use async_trait::async_trait;
use serde_json::{json, Value};
use tez_llm::ToolSpec;

use super::{Tool, ToolError, ToolOutput};

fn trim_word(word: &str) -> &str {
    word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'')
}

fn starts_with_vowel(word: &str) -> bool {
    match word.chars().next().map(|c| c.to_ascii_lowercase()) {
        Some('a' | 'e' | 'i' | 'o' | 'u') => true,
        // "an 8", "an 11", "an 18", "an 80" — số đọc bắt đầu bằng nguyên âm.
        Some(c) if c.is_ascii_digit() => {
            let number: String = word.chars().take_while(|c| c.is_ascii_digit()).collect();
            number.starts_with('8') || number.starts_with("11") || number.starts_with("18")
        }
        _ => false,
    }
}

/// Từ viết tắt mà dấu chấm không phải là kết thúc câu.
const ABBREVIATIONS: &[&str] = &[
    "mr", "mrs", "ms", "dr", "prof", "sr", "jr", "st", "vs", "etc", "e.g", "i.e", "a.m", "p.m",
    "u.s", "u.k", "approx", "dept", "est", "fig", "no", "vol",
];

fn is_abbreviation_period(chars: &[char], index: usize) -> bool {
    let mut start = index;
    while start > 0 && !chars[start - 1].is_whitespace() {
        start -= 1;
    }
    let word: String = chars[start..index].iter().collect();
    let cleaned = word
        .trim_end_matches('.')
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '.')
        .to_ascii_lowercase();
    if cleaned.is_empty() {
        return false;
    }
    if cleaned.chars().count() == 1 && cleaned.chars().all(|c| c.is_ascii_alphabetic()) {
        return true;
    }
    ABBREVIATIONS.contains(&cleaned.as_str())
}

/// Các từ bắt đầu bằng nguyên âm nhưng đọc như phụ âm (a university).
const CONSONANT_SOUND: &[&str] = &[
    "university",
    "user",
    "unique",
    "unit",
    "union",
    "european",
    "one",
    "once",
    "useful",
    "usual",
];

/// Các từ bắt đầu bằng phụ âm nhưng đọc như nguyên âm (an hour).
const VOWEL_SOUND: &[&str] = &["hour", "honest", "honor", "honour", "heir"];

fn check_articles(text: &str, issues: &mut Vec<String>) {
    let words: Vec<&str> = text.split_whitespace().collect();
    for window in words.windows(2) {
        let article = trim_word(window[0]).to_ascii_lowercase();
        let next = trim_word(window[1]).to_ascii_lowercase();
        if next.is_empty() {
            continue;
        }
        if article == "a" && starts_with_vowel(&next) && !CONSONANT_SOUND.contains(&next.as_str()) {
            issues.push(format!("dùng 'an' thay cho 'a' trước '{next}'"));
        }
        if article == "an" && !starts_with_vowel(&next) && !VOWEL_SOUND.contains(&next.as_str()) {
            issues.push(format!("dùng 'a' thay cho 'an' trước '{next}'"));
        }
    }
}

fn check_repeated_words(text: &str, issues: &mut Vec<String>) {
    let words: Vec<&str> = text.split_whitespace().collect();
    for window in words.windows(2) {
        let first = trim_word(window[0]).to_ascii_lowercase();
        let second = trim_word(window[1]).to_ascii_lowercase();
        if !first.is_empty() && first == second {
            issues.push(format!("từ bị lặp: '{} {}'", window[0], window[1]));
        }
    }
}

fn check_capitalization(text: &str, issues: &mut Vec<String>) {
    let chars: Vec<char> = text.chars().collect();
    let mut at_sentence_start = true;
    for (index, &ch) in chars.iter().enumerate() {
        if at_sentence_start {
            if ch.is_whitespace() || ch == '"' || ch == '\'' {
                continue;
            }
            if ch.is_alphabetic() && ch.is_lowercase() {
                issues.push(format!("đầu câu chưa viết hoa (bắt đầu bằng '{ch}')"));
            }
            at_sentence_start = false;
        }
        if matches!(ch, '.' | '!' | '?') {
            // "e.g." / "Mr." không mở câu mới.
            if ch == '.' && is_abbreviation_period(&chars, index) {
                continue;
            }
            at_sentence_start = true;
        }
    }
}

fn check_punctuation_spacing(text: &str, issues: &mut Vec<String>) {
    let chars: Vec<char> = text.chars().collect();
    for (index, &ch) in chars.iter().enumerate() {
        if !matches!(ch, ',' | ';' | ':' | '!' | '?' | '.') {
            continue;
        }
        if index > 0 && chars[index - 1] == ' ' {
            issues.push(format!("có khoảng trắng trước '{ch}'"));
        }
        let Some(&next) = chars.get(index + 1) else {
            continue;
        };
        // Bỏ qua số thập phân, từ viết tắt và dấu chấm cuối câu.
        if ch == '.' && (next.is_ascii_digit() || is_abbreviation_period(&chars, index)) {
            continue;
        }
        if next.is_alphabetic() {
            issues.push(format!("thiếu khoảng trắng sau '{ch}'"));
        }
    }
}

fn check_final_punctuation(text: &str, issues: &mut Vec<String>) {
    let Some(last) = text.trim_end().chars().last() else {
        return;
    };
    if !matches!(last, '.' | '!' | '?' | '"' | '\'' | ')' | '”') {
        issues.push("cuối đoạn chưa có dấu câu".to_string());
    }
}

/// Kiểm tra các lỗi tiếng Anh cơ bản bằng heuristic (không thay thế giáo viên).
pub struct CheckGrammar;

#[async_trait]
impl Tool for CheckGrammar {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "check_grammar".into(),
            description: "Kiểm tra lỗi tiếng Anh cơ bản: mạo từ a/an, từ lặp, viết hoa đầu câu, khoảng trắng quanh dấu câu, dấu câu cuối đoạn. Đây là heuristic, không phải chấm ngữ pháp đầy đủ.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "Đoạn văn tiếng Anh cần kiểm tra" }
                },
                "required": ["text"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let text = args
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArgs("thiếu trường 'text'".into()))?;
        if text.trim().is_empty() {
            return Ok(ToolOutput::error("đoạn văn rỗng".to_string()));
        }
        super::check_input_bounds(text).map_err(ToolError::InvalidArgs)?;

        let mut issues = Vec::new();
        if text.contains("  ") {
            issues.push("có nhiều hơn một khoảng trắng liên tiếp".to_string());
        }
        check_repeated_words(text, &mut issues);
        check_articles(text, &mut issues);
        check_capitalization(text, &mut issues);
        check_punctuation_spacing(text, &mut issues);
        check_final_punctuation(text, &mut issues);

        // Giữ thứ tự nhưng bỏ trùng.
        let mut seen = std::collections::HashSet::new();
        issues.retain(|issue| seen.insert(issue.clone()));

        if issues.is_empty() {
            return Ok(ToolOutput::ok(
                "Không phát hiện lỗi cơ bản. Vẫn nên nhờ giáo viên kiểm tra ngữ pháp nâng cao."
                    .to_string(),
            ));
        }

        let total = issues.len();
        let shown = issues.iter().take(20);
        let mut output = format!("Phát hiện {total} lỗi cơ bản:\n");
        for issue in shown {
            output.push_str(&format!("- {issue}\n"));
        }
        if total > 20 {
            output.push_str("- ... (còn tiếp)\n");
        }
        Ok(ToolOutput::ok(output.trim_end().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn catches_article_and_spacing() {
        let out = CheckGrammar
            .execute(json!({"text": "I have a apple .  She  likes it"}))
            .await
            .unwrap();
        assert!(!out.is_error);
        assert!(out.content.contains("'an'"));
        assert!(out.content.contains("khoảng trắng"));
    }

    #[tokio::test]
    async fn accepts_clean_sentence() {
        let out = CheckGrammar
            .execute(json!({"text": "She goes to school every day."}))
            .await
            .unwrap();
        assert!(!out.is_error);
        assert!(out.content.contains("Không phát hiện"));
    }

    #[tokio::test]
    async fn accepts_an_before_number_with_vowel_sound() {
        let out = CheckGrammar
            .execute(json!({"text": "An 8-year-old boy is here."}))
            .await
            .unwrap();
        assert!(!out.is_error);
        assert!(out.content.contains("Không phát hiện"), "{}", out.content);
    }

    #[tokio::test]
    async fn ignores_abbreviation_periods() {
        let out = CheckGrammar
            .execute(json!({"text": "Use e.g. this example, i.e. the first one."}))
            .await
            .unwrap();
        assert!(!out.is_error);
        assert!(!out.content.contains("viết hoa"), "{}", out.content);
        assert!(
            !out.content.contains("thiếu khoảng trắng"),
            "{}",
            out.content
        );
    }

    #[tokio::test]
    async fn rejects_oversized_text() {
        let text = "a".repeat(5000);
        let error = CheckGrammar
            .execute(json!({"text": text}))
            .await
            .unwrap_err();
        assert!(matches!(error, ToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn catches_lowercase_start() {
        let out = CheckGrammar
            .execute(json!({"text": "this is a test"}))
            .await
            .unwrap();
        assert!(out.content.contains("viết hoa"));
        assert!(out.content.contains("dấu câu"));
    }
}
