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
        super::check_input_bounds(expression).map_err(ToolError::InvalidArgs)?;

        match evalexpr::eval(expression) {
            Ok(value) => Ok(ToolOutput::ok(value.to_string())),
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
