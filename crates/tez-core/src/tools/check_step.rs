use async_trait::async_trait;
use serde_json::{json, Value};
use tez_llm::ToolSpec;

use super::{Tool, ToolError, ToolOutput};

/// Nhận biểu thức dạng chuỗi hoặc số (model hay gửi 4 thay vì "4").
fn expression_arg(args: &Value, field: &str) -> Result<String, ToolError> {
    match args.get(field) {
        Some(Value::String(value)) => Ok(value.clone()),
        Some(Value::Number(value)) => Ok(value.to_string()),
        _ => Err(ToolError::InvalidArgs(format!("thiếu '{field}'"))),
    }
}

/// Chuẩn hóa số nguyên literal (kể cả > i64) để so sánh chính xác.
fn integer_literal(text: &str) -> Option<String> {
    let text = text.trim();
    let (sign, digits) = match text.strip_prefix('-') {
        Some(digits) => ("-", digits),
        None => ("", text.strip_prefix('+').unwrap_or(text)),
    };
    if digits.is_empty() || !digits.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let digits = digits.trim_start_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    Some(if digits == "0" {
        "0".to_string()
    } else {
        format!("{sign}{digits}")
    })
}

/// So sánh hai biểu thức số học, dùng để kiểm tra một bước giải.
pub struct CheckStep;

#[async_trait]
impl Tool for CheckStep {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "check_step".into(),
            description:
                "Kiểm tra một bước giải có tương đương về mặt số học với kết quả mong đợi.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "expected": { "type": "string", "description": "Biểu thức/kết quả mong đợi" },
                    "actual": { "type": "string", "description": "Biểu thức/kết quả học sinh đưa ra" },
                    "tolerance": { "type": "number", "description": "Sai số cho phép, mặc định 1e-6" }
                },
                "required": ["expected", "actual"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let expected = expression_arg(&args, "expected")?;
        let actual = expression_arg(&args, "actual")?;
        let tolerance = match args.get("tolerance") {
            None => 1e-6,
            Some(value) => value
                .as_f64()
                .filter(|tolerance| tolerance.is_finite() && *tolerance >= 0.0)
                .ok_or_else(|| {
                    ToolError::InvalidArgs("'tolerance' phải là số hữu hạn >= 0".into())
                })?,
        };
        super::check_input_bounds(&expected).map_err(ToolError::InvalidArgs)?;
        super::check_input_bounds(&actual).map_err(ToolError::InvalidArgs)?;

        // Hai số nguyên literal: so sánh chuỗi đã chuẩn hóa, không qua f64.
        if let (Some(a), Some(b)) = (integer_literal(&expected), integer_literal(&actual)) {
            return if a == b {
                Ok(ToolOutput::ok(format!(
                    "tương đương: {expected} = {actual} = {a}"
                )))
            } else {
                Ok(ToolOutput::error(format!(
                    "chưa tương đương: {expected} = {a}, {actual} = {b}"
                )))
            };
        }

        let expected_normalized = super::normalize_math_functions(&expected);
        let actual_normalized = super::normalize_math_functions(&actual);
        let expected_value = evalexpr::eval(&expected_normalized)
            .map_err(|e| ToolError::InvalidArgs(format!("expected không hợp lệ: {e}")))?;
        let actual_value = evalexpr::eval(&actual_normalized)
            .map_err(|e| ToolError::InvalidArgs(format!("actual không hợp lệ: {e}")))?;

        let as_number = |value: &evalexpr::Value| {
            value
                .as_float()
                .ok()
                .or_else(|| value.as_int().ok().map(|number| number as f64))
        };
        let (Some(a), Some(b)) = (as_number(&expected_value), as_number(&actual_value)) else {
            return Ok(ToolOutput::error(
                "chỉ so sánh được biểu thức cho kết quả số".to_string(),
            ));
        };

        // So sánh số nguyên chính xác trước: ép i64 sang f64 làm sai số lớn.
        let ints = (expected_value.as_int().ok(), actual_value.as_int().ok());
        let equal = match ints {
            (Some(a), Some(b)) => a == b || ((a as f64) - (b as f64)).abs() <= tolerance,
            _ => a == b || (a - b).abs() <= tolerance,
        };
        if equal {
            Ok(ToolOutput::ok(format!(
                "tương đương: {expected} = {actual} = {a}"
            )))
        } else {
            Ok(ToolOutput::error(format!(
                "chưa tương đương: {expected} = {a}, {actual} = {b}"
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn compares_numbers() {
        let tool = CheckStep;
        let out = tool
            .execute(json!({"expected": "3.0/4.0", "actual": "0.75"}))
            .await
            .unwrap();
        assert!(!out.is_error);
        assert!(out.content.contains("tương đương"));
    }

    #[tokio::test]
    async fn compares_integers() {
        let tool = CheckStep;
        let out = tool
            .execute(json!({"expected": "4", "actual": "2+2"}))
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("tương đương"));
    }

    #[tokio::test]
    async fn accepts_numeric_actual_and_plain_math_names() {
        let tool = CheckStep;
        let out = tool
            .execute(json!({"expected": "sqrt(16)", "actual": 4}))
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
    }

    #[tokio::test]
    async fn rejects_invalid_tolerance() {
        let tool = CheckStep;
        for tolerance in [json!("0.2"), json!(-0.1), json!(null)] {
            let error = tool
                .execute(json!({"expected": "1", "actual": "1", "tolerance": tolerance}))
                .await
                .unwrap_err();
            assert!(matches!(error, ToolError::InvalidArgs(_)));
        }
    }

    #[tokio::test]
    async fn compares_large_integers_exactly() {
        let tool = CheckStep;
        let out = tool
            .execute(json!({
                "expected": "1234567890123456789012345678901",
                "actual": "1234567890123456789012345678902"
            }))
            .await
            .unwrap();
        assert!(
            out.is_error,
            "hai số nguyên khác nhau phải không tương đương"
        );
    }

    #[tokio::test]
    async fn rejects_deeply_nested_expressions() {
        let tool = CheckStep;
        let deep = format!("{}1{}", "(".repeat(5000), ")".repeat(5000));
        let error = tool
            .execute(json!({"expected": deep, "actual": "1"}))
            .await
            .unwrap_err();
        assert!(matches!(error, ToolError::InvalidArgs(_)));
    }
}
