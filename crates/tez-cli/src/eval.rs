//! Bộ eval hồi quy tối giản cho provider.
//!
//! Mục tiêu không phải chấm điểm mô hình một cách học thuật, mà là phát hiện
//! regression rõ ràng: câu trả lời thiếu dữ kiện bắt buộc, hoặc chứa đáp án sai
//! đã biết. Mỗi case chạy một prompt, so khớp `contains`/`not_contains`.

use std::path::Path;
use std::time::Instant;

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use tez_llm::{ChatRequest, ChatResponse, LlmProvider, Message, Usage};

#[derive(Debug, Clone, Deserialize)]
pub struct EvalSuite {
    #[serde(default = "default_suite_name")]
    pub name: String,
    pub cases: Vec<EvalCase>,
}

fn default_suite_name() -> String {
    "eval".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct EvalCase {
    pub id: String,
    #[serde(default)]
    pub subject: String,
    pub prompt: String,
    #[serde(default)]
    pub system: Option<String>,
    /// Tất cả chuỗi này phải xuất hiện (không phân biệt hoa thường).
    #[serde(default)]
    pub contains: Vec<String>,
    /// Không được xuất hiện chuỗi nào trong danh sách này.
    #[serde(default)]
    pub not_contains: Vec<String>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub temperature: Option<f32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvalReport {
    pub suite: String,
    pub model: String,
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub cases: Vec<CaseResult>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CaseResult {
    pub id: String,
    pub subject: String,
    pub passed: bool,
    pub latency_ms: u128,
    /// Bản rút gọn để log/JSON không phình theo câu trả lời dài.
    pub answer: String,
    pub answer_chars: usize,
    pub failures: Vec<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

pub fn load(path: &Path) -> anyhow::Result<EvalSuite> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("không đọc được bộ eval: {}", path.display()))?;
    let suite: EvalSuite = serde_json::from_str(&raw)
        .with_context(|| format!("JSON eval không hợp lệ: {}", path.display()))?;
    if suite.cases.is_empty() {
        anyhow::bail!("Bộ eval không có case nào: {}", path.display());
    }
    for case in &suite.cases {
        if case.id.trim().is_empty() {
            anyhow::bail!("Có case eval thiếu id: {}", path.display());
        }
        if case.prompt.trim().is_empty() {
            anyhow::bail!("Case '{}' thiếu prompt.", case.id);
        }
        if case.contains.is_empty() && case.not_contains.is_empty() {
            anyhow::bail!(
                "Case '{}' cần ít nhất một tiêu chí contains/not_contains.",
                case.id
            );
        }
    }
    Ok(suite)
}

/// Chấm một câu trả lời theo tiêu chí của case.
///
/// Trả về danh sách lỗi; rỗng nghĩa là pass. So khớp không phân biệt hoa thường
/// để tránh fail vì model viết hoa tên riêng.
pub fn score(case: &EvalCase, answer: &str) -> Vec<String> {
    let haystack = answer.to_lowercase();
    let mut failures = Vec::new();
    for needle in &case.contains {
        let needle = needle.trim();
        if needle.is_empty() {
            continue;
        }
        if !haystack.contains(&needle.to_lowercase()) {
            failures.push(format!("thiếu \"{needle}\""));
        }
    }
    for needle in &case.not_contains {
        let needle = needle.trim();
        if needle.is_empty() {
            continue;
        }
        if haystack.contains(&needle.to_lowercase()) {
            failures.push(format!("chứa đáp án sai \"{needle}\""));
        }
    }
    failures
}

pub async fn run_suite(
    provider: &dyn LlmProvider,
    suite: &EvalSuite,
    model: &str,
    progress: bool,
) -> EvalReport {
    let mut cases = Vec::with_capacity(suite.cases.len());
    for case in &suite.cases {
        if progress {
            eprintln!("[eval] {} ...", case.id);
        }
        let started = Instant::now();
        let mut messages = Vec::new();
        if let Some(system) = case
            .system
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            messages.push(Message::system(system));
        }
        messages.push(Message::user(&case.prompt));
        let request = ChatRequest {
            model: model.to_string(),
            messages,
            tools: Vec::new(),
            temperature: Some(case.temperature.unwrap_or(0.0)),
            top_p: None,
            max_tokens: Some(case.max_tokens.unwrap_or(512)),
        };

        let (answer, usage, error) = match provider.chat(request).await {
            Ok(ChatResponse::Text { content, usage }) => (content, usage, None),
            Ok(ChatResponse::ToolCalls { content, usage, .. }) => {
                (content.unwrap_or_default(), usage, None)
            }
            Err(error) => (String::new(), Usage::default(), Some(error)),
        };
        let latency_ms = started.elapsed().as_millis();
        let mut failures = if let Some(error) = &error {
            vec![format!("provider lỗi: {error}")]
        } else {
            score(case, &answer)
        };
        if answer.trim().is_empty() && error.is_none() {
            failures.push("câu trả lời rỗng".to_string());
        }
        let passed = failures.is_empty();
        if progress {
            eprintln!(
                "[eval] {} {} ({}ms)",
                if passed { "PASS" } else { "FAIL" },
                case.id,
                latency_ms
            );
        }
        cases.push(CaseResult {
            id: case.id.clone(),
            subject: case.subject.clone(),
            passed,
            latency_ms,
            answer: preview(&answer, 2_000),
            answer_chars: answer.chars().count(),
            failures,
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
        });
    }
    let passed = cases.iter().filter(|case| case.passed).count();
    EvalReport {
        suite: suite.name.clone(),
        model: model.to_string(),
        total: cases.len(),
        passed,
        failed: cases.len() - passed,
        cases,
    }
}

pub fn print_report(report: &EvalReport) {
    println!(
        "Bộ eval: {} · model: {} · {} case",
        report.suite, report.model, report.total
    );
    for case in &report.cases {
        let status = if case.passed { "PASS" } else { "FAIL" };
        let subject = if case.subject.trim().is_empty() {
            String::new()
        } else {
            format!(" · {}", case.subject)
        };
        println!(
            "[{status}] {}{subject} · {}ms · {} token ra",
            case.id, case.latency_ms, case.output_tokens
        );
        if !case.passed {
            for failure in &case.failures {
                println!("  - {failure}");
            }
            if !case.answer.trim().is_empty() {
                println!("  trả lời: {}", case.answer.replace('\n', " "));
            }
        }
    }
    println!(
        "Kết quả: {}/{} pass, {} fail",
        report.passed, report.total, report.failed
    );
}

fn preview(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let mut out: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use tez_llm::LlmError;

    struct FakeProvider;

    #[async_trait]
    impl LlmProvider for FakeProvider {
        fn name(&self) -> &str {
            "fake"
        }

        async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, LlmError> {
            let prompt = request
                .messages
                .last()
                .and_then(|message| message.content.as_deref())
                .unwrap_or_default();
            let content = if prompt.contains("17 x 23") {
                "391"
            } else {
                "không biết"
            };
            Ok(ChatResponse::Text {
                content: content.to_string(),
                usage: Usage {
                    input_tokens: 3,
                    output_tokens: 1,
                },
            })
        }
    }

    fn case(id: &str, prompt: &str, contains: &[&str]) -> EvalCase {
        EvalCase {
            id: id.to_string(),
            subject: String::new(),
            prompt: prompt.to_string(),
            system: None,
            contains: contains.iter().map(|value| value.to_string()).collect(),
            not_contains: Vec::new(),
            max_tokens: None,
            temperature: None,
        }
    }

    #[test]
    fn parses_suite_shape() {
        let suite: EvalSuite = serde_json::from_str(
            r#"{"name":"basic","cases":[{"id":"a","prompt":"hi","contains":["hi"]}]}"#,
        )
        .unwrap();
        assert_eq!(suite.name, "basic");
        assert_eq!(suite.cases.len(), 1);
    }

    #[test]
    fn rejects_empty_suite_on_load() {
        let path = std::env::temp_dir().join(format!("tez-eval-empty-{}.json", std::process::id()));
        std::fs::write(&path, r#"{"name":"empty","cases":[]}"#).unwrap();
        let result = load(&path);
        let _ = std::fs::remove_file(&path);
        assert!(result.is_err());
    }

    #[test]
    fn scores_contains_and_not_contains_case_insensitively() {
        let mut case = case("math", "17 x 23", &["391"]);
        case.not_contains = vec!["392".to_string()];
        assert!(score(&case, "Đáp án là 391").is_empty());
        assert!(!score(&case, "Đáp án là 392").is_empty());
        assert!(!score(&case, "không biết").is_empty());
    }

    #[tokio::test]
    async fn runs_suite_and_counts_results() {
        let suite = EvalSuite {
            name: "fake".to_string(),
            cases: vec![
                case("pass", "17 x 23", &["391"]),
                case("fail", "câu khác", &["391"]),
            ],
        };
        let report = run_suite(&FakeProvider, &suite, "fake-model", false).await;
        assert_eq!(report.total, 2);
        assert_eq!(report.passed, 1);
        assert_eq!(report.failed, 1);
        assert!(report.cases[0].passed);
        assert!(!report.cases[1].passed);
    }
}
