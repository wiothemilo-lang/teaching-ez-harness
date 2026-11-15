use async_trait::async_trait;
use serde_json::{json, Value};
use tez_llm::ToolSpec;

use super::{Tool, ToolError, ToolOutput};

/// Ước lượng độ dễ đọc Flesch Reading Ease (tiếng Anh).
pub struct Readability;

#[async_trait]
impl Tool for Readability {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "readability".into(),
            description: "Ước lượng độ dễ đọc của đoạn văn tiếng Anh (Flesch Reading Ease).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string" }
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
            .ok_or_else(|| ToolError::InvalidArgs("thiếu 'text'".into()))?;

        super::check_input_bounds(text).map_err(ToolError::InvalidArgs)?;

        let sentences = text
            .chars()
            .filter(|c| matches!(c, '.' | '!' | '?'))
            .count()
            .max(1) as f64;
        let words: Vec<&str> = text.split_whitespace().collect();
        if words.is_empty() {
            return Ok(ToolOutput::error("đoạn văn rỗng".to_string()));
        }
        let word_count = words.len() as f64;

        let syllables: usize = words.iter().map(|w| count_syllables(w)).sum();
        let syllable_count = syllables.max(1) as f64;

        let raw_score =
            206.835 - 1.015 * (word_count / sentences) - 84.6 * (syllable_count / word_count);
        let score = raw_score.clamp(0.0, 100.0);
        let level = match score {
            s if s >= 80.0 => "rất dễ",
            s if s >= 60.0 => "dễ",
            s if s >= 40.0 => "trung bình",
            s if s >= 20.0 => "khó",
            _ => "rất khó",
        };

        Ok(ToolOutput::ok(format!(
            "Flesch {:.1}/100 ({level}); {} từ, {} câu",
            score, word_count as usize, sentences as usize
        )))
    }
}

fn count_syllables(word: &str) -> usize {
    let word = word.to_lowercase();
    let mut count = 0;
    let mut previous_vowel = false;
    for ch in word.chars() {
        let is_vowel = matches!(ch, 'a' | 'e' | 'i' | 'o' | 'u' | 'y');
        if is_vowel && !previous_vowel {
            count += 1;
        }
        previous_vowel = is_vowel;
    }
    if word.ends_with('e') && count > 1 {
        count -= 1;
    }
    count.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_oversized_text() {
        let text = "word ".repeat(1000);
        let error = Readability
            .execute(json!({"text": text}))
            .await
            .unwrap_err();
        assert!(matches!(error, ToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn scores_simple_text() {
        let tool = Readability;
        let out = tool
            .execute(json!({"text": "The cat sat on the mat. It was happy."}))
            .await
            .unwrap();
        assert!(!out.is_error);
        assert!(out.content.contains("Flesch"));
    }
}
