use async_trait::async_trait;
use serde_json::{json, Value};
use tez_llm::ToolSpec;

use super::chem::{format_composition, molar_mass, parse_formula};
use super::{Tool, ToolError, ToolOutput};

/// Tính khối lượng mol từ công thức hóa học.
pub struct MolarMass;

#[async_trait]
impl Tool for MolarMass {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "molar_mass".into(),
            description: "Tính khối lượng mol (g/mol) từ công thức hóa học. Hỗ trợ ngoặc và hydrate, ví dụ Ca(OH)2, Fe2(SO4)3, CuSO4·5H2O.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "formula": {
                        "type": "string",
                        "description": "Công thức hóa học, ví dụ H2SO4"
                    }
                },
                "required": ["formula"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let formula = args
            .get("formula")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArgs("thiếu trường 'formula'".into()))?;

        let composition = match parse_formula(formula) {
            Ok(composition) => composition,
            Err(error) => return Ok(ToolOutput::error(error)),
        };
        let mass = match molar_mass(&composition) {
            Ok(mass) => mass,
            Err(error) => return Ok(ToolOutput::error(error)),
        };

        let breakdown: Vec<String> = composition
            .iter()
            .map(|(element, count)| format!("{element} {count}"))
            .collect();

        Ok(ToolOutput::ok(format!(
            "{}: {:.3} g/mol ({}); công thức chuẩn hóa: {}",
            formula.trim(),
            mass,
            breakdown.join(", "),
            format_composition(&composition)
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn computes_water() {
        let tool = MolarMass;
        let out = tool.execute(json!({"formula": "H2O"})).await.unwrap();
        assert!(!out.is_error);
        assert!(out.content.contains("18.015 g/mol"));
    }

    #[tokio::test]
    async fn reports_unknown_element() {
        let tool = MolarMass;
        let out = tool.execute(json!({"formula": "XxO"})).await.unwrap();
        assert!(out.is_error);
    }
}
