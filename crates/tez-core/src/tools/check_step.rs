use async_trait::async_trait;
use serde_json::{json, Value};
use tez_llm::ToolSpec;

use super::{Tool, ToolError, ToolOutput};

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
        let expected = args
            .get("expected")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArgs("thiếu 'expected'".into()))?;
        let actual = args
            .get("actual")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArgs("thiếu 'actual'".into()))?;
        let tolerance = args
            .get("tolerance")
            .and_then(Value::as_f64)
            .unwrap_or(1e-6);
        super::check_input_bounds(expected).map_err(ToolError::InvalidArgs)?;
        super::check_input_bounds(actual).map_err(ToolError::InvalidArgs)?;

        let expected_value = evalexpr::eval(expected)
            .map_err(|e| ToolError::InvalidArgs(format!("expected không hợp lệ: {e}")))?;
        let actual_value = evalexpr::eval(actual)
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

        if (a - b).abs() <= tolerance {
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
