use async_trait::async_trait;
use serde_json::{json, Value};
use tez_llm::ToolSpec;

use super::{Tool, ToolError, ToolOutput};

/// Tool tính toán an toàn, không eval code.
pub struct Calculator;

#[async_trait]
impl Tool for Calculator {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "calculate".into(),
            description: "Tính biểu thức số học. Dùng cho phép tính đơn giản, kiểm tra bước giải."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "expression": {
                        "type": "string",
                        "description": "Biểu thức số học, ví dụ: (3+5)*2 hoặc sqrt(16)"
                    }
                },
                "required": ["expression"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let expression = args
            .get("expression")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArgs("thiếu trường 'expression'".into()))?;
        let expression = expression.trim();
        if expression.is_empty() {
            return Ok(ToolOutput::error("biểu thức rỗng".to_string()));
        }
        super::check_input_bounds(expression).map_err(ToolError::InvalidArgs)?;

        let normalized = super::normalize_math_functions(expression);
        match evalexpr::eval(&normalized) {
            Ok(value) => {
                if let Ok(number) = value.as_number() {
                    if !number.is_finite() {
                        return Ok(ToolOutput::error(
                            "kết quả không hữu hạn (chia cho 0 hoặc tràn số)".to_string(),
                        ));
                    }
                }
                Ok(ToolOutput::ok(value.to_string()))
            }
            Err(error) => Ok(ToolOutput::error(format!("không tính được: {error}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn evaluates_expression() {
        let tool = Calculator;
        let out = tool
            .execute(json!({"expression": "2 + 3 * 4"}))
            .await
            .unwrap();
        assert!(!out.is_error);
        assert_eq!(out.content, "14");
    }

    #[tokio::test]
    async fn reports_bad_expression() {
        let tool = Calculator;
        let out = tool.execute(json!({"expression": "2 +"})).await.unwrap();
        assert!(out.is_error);
    }

    #[tokio::test]
    async fn accepts_plain_math_names() {
        let tool = Calculator;
        let out = tool
            .execute(json!({"expression": "sqrt(16)"}))
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(out.content, "4");
    }

    #[tokio::test]
    async fn rejects_empty_and_non_finite_results() {
        let tool = Calculator;
        let empty = tool.execute(json!({"expression": "   "})).await.unwrap();
        assert!(empty.is_error);
        for expression in ["1/0.0", "1e308*10"] {
            let out = tool
                .execute(json!({"expression": expression}))
                .await
                .unwrap();
            assert!(out.is_error, "{expression} -> {}", out.content);
        }
    }

    #[tokio::test]
    async fn rejects_deeply_nested_expression() {
        let tool = Calculator;
        let deep = format!("{}1{}", "(".repeat(5000), ")".repeat(5000));
        let error = tool
            .execute(json!({ "expression": deep }))
            .await
            .unwrap_err();
        assert!(matches!(error, ToolError::InvalidArgs(_)));
    }
}
