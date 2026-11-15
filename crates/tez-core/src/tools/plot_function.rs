use async_trait::async_trait;
use evalexpr::{
    eval_with_context, ContextWithMutableVariables, HashMapContext, Value as EvalValue,
};
use serde_json::{json, Value};
use tez_llm::ToolSpec;

use super::{Tool, ToolError, ToolOutput};

/// Chuẩn hóa tên hàm cho thân thiện với học sinh, dùng chung với calculator.
fn normalize_expression(expression: &str) -> String {
    super::normalize_math_functions(expression)
}

fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

/// Vẽ đồ thị hàm một biến bằng ASCII, phù hợp CLI và chat văn bản.
pub struct PlotFunction;

#[async_trait]
impl Tool for PlotFunction {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "plot_function".into(),
            description: "Vẽ đồ thị hàm số y = f(x) bằng ASCII. Dùng biến x; hỗ trợ ^, sqrt, sin, cos, log, ln. Ví dụ: x^2 - 2*x + 1.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "expression": {
                        "type": "string",
                        "description": "Biểu thức theo x, ví dụ x^2 - 2*x + 1"
                    },
                    "x_min": { "type": "number", "description": "Mặc định -10" },
                    "x_max": { "type": "number", "description": "Mặc định 10" },
                    "width": { "type": "integer", "description": "Số cột, mặc định 60" },
                    "height": { "type": "integer", "description": "Số dòng, mặc định 20" }
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
        let x_min = args.get("x_min").and_then(Value::as_f64).unwrap_or(-10.0);
        let x_max = args.get("x_max").and_then(Value::as_f64).unwrap_or(10.0);
        let width = args
            .get("width")
            .and_then(Value::as_u64)
            .unwrap_or(60)
            .clamp(20, 100) as usize;
        let height = args
            .get("height")
            .and_then(Value::as_u64)
            .unwrap_or(20)
            .clamp(8, 40) as usize;

        if !x_min.is_finite() || !x_max.is_finite() || x_min >= x_max {
            return Ok(ToolOutput::error(
                "x_min và x_max phải hữu hạn và x_min < x_max".to_string(),
            ));
        }

        let normalized = normalize_expression(expression);
        let mut context = HashMapContext::new();
        let _ = context.set_value("pi".into(), EvalValue::Float(std::f64::consts::PI));
        let _ = context.set_value("e".into(), EvalValue::Float(std::f64::consts::E));

        let step = (x_max - x_min) / (width.saturating_sub(1)) as f64;
        let mut samples: Vec<(usize, f64)> = Vec::new();
        for column in 0..width {
            let x = x_min + step * column as f64;
            if context.set_value("x".into(), EvalValue::Float(x)).is_err() {
                return Ok(ToolOutput::error("không đặt được biến x".to_string()));
            }
            match eval_with_context(&normalized, &context) {
                Ok(value) => match value.as_number() {
                    Ok(y) if y.is_finite() => samples.push((column, y)),
                    Ok(_) => {}
                    Err(_) => {
                        return Ok(ToolOutput::error(
                            "biểu thức phải trả về một số".to_string(),
                        ))
                    }
                },
                Err(error) => {
                    return Ok(ToolOutput::error(format!(
                        "không tính được '{expression}': {error}"
                    )))
                }
            }
        }

        if samples.is_empty() {
            return Ok(ToolOutput::error(
                "không có điểm nào hữu hạn trong khoảng đã chọn".to_string(),
            ));
        }

        let mut ys: Vec<f64> = samples.iter().map(|(_, y)| *y).collect();
        ys.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let (mut y_min, mut y_max) = (ys[0], ys[ys.len() - 1]);
        if ys.len() >= 20 {
            let low = percentile(&ys, 0.05);
            let high = percentile(&ys, 0.95);
            // Nếu có tiệm cận làm range nổ tung, cắt theo percentile.
            if (y_max - y_min) > 10.0 * (high - low).max(f64::EPSILON) {
                y_min = low;
                y_max = high;
            }
        }
        if (y_max - y_min).abs() < f64::EPSILON {
            y_min -= 1.0;
            y_max += 1.0;
        }

        let mut grid = vec![vec![' '; width]; height];
        let axis_row = if y_min <= 0.0 && y_max >= 0.0 {
            Some((((y_max - 0.0) / (y_max - y_min)) * (height - 1) as f64).round() as usize)
        } else {
            None
        };
        let axis_col = if x_min <= 0.0 && x_max >= 0.0 {
            Some((((0.0 - x_min) / (x_max - x_min)) * (width - 1) as f64).round() as usize)
        } else {
            None
        };

        if let Some(row) = axis_row {
            for cell in grid[row].iter_mut() {
                *cell = '-';
            }
        }
        if let Some(col) = axis_col {
            for row in grid.iter_mut() {
                row[col] = '|';
            }
        }
        if let (Some(row), Some(col)) = (axis_row, axis_col) {
            grid[row][col] = '+';
        }

        for (column, y) in samples {
            let clamped = y.clamp(y_min, y_max);
            let row =
                (((y_max - clamped) / (y_max - y_min)) * (height - 1) as f64).round() as usize;
            grid[row.min(height - 1)][column] = '*';
        }

        let mut output = format!(
            "y = {expression}\nx ∈ [{}, {}], y ∈ [{:.3}, {:.3}]\n",
            trim(x_min),
            trim(x_max),
            y_min,
            y_max
        );
        for row in grid {
            output.push_str(&row.iter().collect::<String>());
            output.push('\n');
        }
        Ok(ToolOutput::ok(output.trim_end().to_string()))
    }
}

fn trim(value: f64) -> String {
    let mut text = format!("{value:.3}");
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn plots_parabola() {
        let tool = PlotFunction;
        let out = tool
            .execute(
                json!({"expression": "x^2", "x_min": -2, "x_max": 2, "width": 30, "height": 10}),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("y = x^2"));
        assert!(out.content.contains('*'));
    }

    #[tokio::test]
    async fn normalizes_sqrt() {
        assert_eq!(normalize_expression("sqrt(x)"), "math::sqrt(x)");
        assert_eq!(normalize_expression("math::sqrt(x)"), "math::sqrt(x)");
    }

    #[test]
    fn does_not_corrupt_asin_or_prefixed_names() {
        assert_eq!(normalize_expression("asin(x)"), "math::asin(x)");
        assert_eq!(normalize_expression("math::asin(x)"), "math::asin(x)");
        assert_eq!(
            normalize_expression("sin(x) + asin(x)"),
            "math::sin(x) + math::asin(x)"
        );
    }

    #[tokio::test]
    async fn rejects_bad_range() {
        let tool = PlotFunction;
        let out = tool
            .execute(json!({"expression": "x", "x_min": 5, "x_max": 1}))
            .await
            .unwrap();
        assert!(out.is_error);
    }
}
