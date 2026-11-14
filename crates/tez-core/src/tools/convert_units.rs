use async_trait::async_trait;
use serde_json::{json, Value};
use tez_llm::ToolSpec;

use super::{Tool, ToolError, ToolOutput};

struct Unit {
    dimension: &'static str,
    factor: f64,
}

fn lookup(unit: &str) -> Option<Unit> {
    let (dimension, factor) = match unit {
        "m" => ("length", 1.0),
        "cm" => ("length", 0.01),
        "mm" => ("length", 0.001),
        "km" => ("length", 1000.0),
        "kg" => ("mass", 1.0),
        "g" => ("mass", 0.001),
        "mg" => ("mass", 1e-6),
        "s" => ("time", 1.0),
        "min" => ("time", 60.0),
        "h" => ("time", 3600.0),
        "N" => ("force", 1.0),
        "kN" => ("force", 1000.0),
        "J" => ("energy", 1.0),
        "kJ" => ("energy", 1000.0),
        "W" => ("power", 1.0),
        "kW" => ("power", 1000.0),
        "Pa" => ("pressure", 1.0),
        "kPa" => ("pressure", 1000.0),
        "V" => ("voltage", 1.0),
        "mV" => ("voltage", 0.001),
        "A" => ("current", 1.0),
        "mA" => ("current", 0.001),
        _ => return None,
    };
    Some(Unit { dimension, factor })
}

/// Đổi đơn vị vật lý cơ bản, kiểm tra cùng chiều.
pub struct ConvertUnits;

#[async_trait]
impl Tool for ConvertUnits {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "convert_units".into(),
            description:
                "Đổi đơn vị vật lý cơ bản (m, cm, km, kg, g, s, min, h, N, J, W, Pa, V, A...)."
                    .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "value": { "type": "number" },
                    "from": { "type": "string" },
                    "to": { "type": "string" }
                },
                "required": ["value", "from", "to"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let value = args
            .get("value")
            .and_then(Value::as_f64)
            .ok_or_else(|| ToolError::InvalidArgs("thiếu 'value'".into()))?;
        let from = args
            .get("from")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArgs("thiếu 'from'".into()))?;
        let to = args
            .get("to")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArgs("thiếu 'to'".into()))?;

        let from_unit = lookup(from)
            .ok_or_else(|| ToolError::InvalidArgs(format!("không biết đơn vị '{from}'")))?;
        let to_unit = lookup(to)
            .ok_or_else(|| ToolError::InvalidArgs(format!("không biết đơn vị '{to}'")))?;

        if from_unit.dimension != to_unit.dimension {
            return Ok(ToolOutput::error(format!(
                "không cùng chiều: {from} ({}) và {to} ({})",
                from_unit.dimension, to_unit.dimension
            )));
        }

        let converted = value * from_unit.factor / to_unit.factor;
        Ok(ToolOutput::ok(format!(
            "{} {} = {} {}",
            trim(value),
            from,
            trim(converted),
            to
        )))
    }
}

fn trim(value: f64) -> String {
    let mut s = format!("{value:.6}");
    while s.contains('.') && s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn converts_km_to_m() {
        let tool = ConvertUnits;
        let out = tool
            .execute(json!({"value": 1.5, "from": "km", "to": "m"}))
            .await
            .unwrap();
        assert!(!out.is_error);
        assert_eq!(out.content, "1.5 km = 1500 m");
    }

    #[tokio::test]
    async fn rejects_dimension_mismatch() {
        let tool = ConvertUnits;
        let out = tool
            .execute(json!({"value": 1.0, "from": "kg", "to": "m"}))
            .await
            .unwrap();
        assert!(out.is_error);
    }
}
