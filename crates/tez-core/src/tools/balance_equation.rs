use std::collections::{BTreeSet, HashSet};

use async_trait::async_trait;
use serde_json::{json, Value};
use tez_llm::ToolSpec;

use super::chem::{parse_formula, Composition};
use super::{Tool, ToolError, ToolOutput};

/// Số hữu tỉ chính xác để giải hệ phương trình cân bằng.
#[derive(Clone, Copy, Debug)]
struct Frac {
    n: i128,
    d: i128,
}

fn gcd(a: i128, b: i128) -> i128 {
    if b == 0 {
        a.abs().max(1)
    } else {
        gcd(b, a % b)
    }
}

fn lcm(a: i128, b: i128) -> i128 {
    a / gcd(a, b) * b
}

impl Frac {
    fn new(n: i128, d: i128) -> Self {
        debug_assert!(d != 0);
        let (n, d) = if d < 0 { (-n, -d) } else { (n, d) };
        let g = gcd(n, d);
        Frac { n: n / g, d: d / g }
    }

    fn zero() -> Self {
        Frac { n: 0, d: 1 }
    }

    fn is_zero(self) -> bool {
        self.n == 0
    }

    fn add(self, other: Self) -> Self {
        Frac::new(self.n * other.d + other.n * self.d, self.d * other.d)
    }

    fn sub(self, other: Self) -> Self {
        self.add(other.neg())
    }

    fn mul(self, other: Self) -> Self {
        Frac::new(self.n * other.n, self.d * other.d)
    }

    fn div(self, other: Self) -> Self {
        Frac::new(self.n * other.d, self.d * other.n)
    }

    fn neg(self) -> Self {
        Frac {
            n: -self.n,
            d: self.d,
        }
    }
}

fn strip_coefficient(species: &str) -> &str {
    let trimmed = species.trim();
    let start = trimmed
        .char_indices()
        .take_while(|(_, c)| c.is_ascii_digit())
        .map(|(i, c)| i + c.len_utf8())
        .last()
        .unwrap_or(0);
    trimmed[start..].trim()
}

fn split_equation(input: &str) -> Option<(&str, &str)> {
    for separator in ["->", "=>", "→", "="] {
        if let Some((left, right)) = input.split_once(separator) {
            return Some((left, right));
        }
    }
    None
}

fn parse_side(side: &str) -> Result<Vec<Composition>, String> {
    let mut species = Vec::new();
    for raw in side.split('+') {
        let formula = strip_coefficient(raw);
        if formula.is_empty() {
            return Err(format!("vế phương trình có chất rỗng: '{side}'"));
        }
        species.push(parse_formula(formula)?);
    }
    Ok(species)
}

/// Tìm vector hệ số dương nhỏ nhất trong không gian nghiệm.
fn nullspace(matrix: &mut [Vec<Frac>], cols: usize) -> Vec<Vec<Frac>> {
    let rows = matrix.len();
    let mut pivots: Vec<usize> = Vec::new();
    let mut row = 0;

    for col in 0..cols {
        if row >= rows {
            break;
        }
        let Some(pivot_row) = (row..rows).find(|&r| !matrix[r][col].is_zero()) else {
            continue;
        };
        matrix.swap(row, pivot_row);

        let pivot = matrix[row][col];
        for value in matrix[row].iter_mut().take(cols) {
            *value = value.div(pivot);
        }

        for r in 0..rows {
            if r == row || matrix[r][col].is_zero() {
                continue;
            }
            let factor = matrix[r][col];
            for c in 0..cols {
                matrix[r][c] = matrix[r][c].sub(factor.mul(matrix[row][c]));
            }
        }

        pivots.push(col);
        row += 1;
    }

    let pivot_set: HashSet<usize> = pivots.iter().copied().collect();
    let mut basis = Vec::new();
    for free in (0..cols).filter(|c| !pivot_set.contains(c)) {
        let mut vector = vec![Frac::zero(); cols];
        vector[free] = Frac::new(1, 1);
        for (row, &pivot_col) in pivots.iter().enumerate() {
            vector[pivot_col] = matrix[row][free].neg();
        }
        basis.push(vector);
    }
    basis
}

fn to_integer(vector: &[Frac]) -> Option<Vec<i128>> {
    let mut denominator = 1;
    for value in vector {
        denominator = lcm(denominator, value.d);
    }
    let integers: Vec<i128> = vector
        .iter()
        .map(|value| value.n * (denominator / value.d))
        .collect();
    let common = integers.iter().fold(0, |acc, &value| gcd(acc, value));
    if common == 0 {
        return None;
    }
    Some(integers.iter().map(|value| value / common).collect())
}

fn search_positive(
    basis: &[Vec<i128>],
    index: usize,
    current: &mut [i128],
    max_multiplier: i128,
    best: &mut Option<Vec<i128>>,
) {
    if index == basis.len() {
        let common = current.iter().fold(0, |acc, &value| gcd(acc, value));
        if common == 0 {
            return;
        }
        let reduced: Vec<i128> = current.iter().map(|value| value / common).collect();
        if reduced.iter().any(|&value| value <= 0) {
            return;
        }
        let score: i128 = reduced.iter().sum();
        let current_score = best
            .as_ref()
            .map(|values| values.iter().sum::<i128>())
            .unwrap_or(i128::MAX);
        if score < current_score {
            *best = Some(reduced);
        }
        return;
    }

    for multiplier in 1..=max_multiplier {
        for (slot, value) in current.iter_mut().zip(&basis[index]) {
            *slot += value * multiplier;
        }
        search_positive(basis, index + 1, current, max_multiplier, best);
        for (slot, value) in current.iter_mut().zip(&basis[index]) {
            *slot -= value * multiplier;
        }
    }
}

fn balance(left: &[Composition], right: &[Composition]) -> Result<Vec<i64>, String> {
    let elements: BTreeSet<String> = left
        .iter()
        .chain(right)
        .flat_map(|composition| composition.keys().cloned())
        .collect();

    for element in &elements {
        let in_left = left.iter().any(|c| c.contains_key(element));
        let in_right = right.iter().any(|c| c.contains_key(element));
        if !in_left || !in_right {
            return Err(format!(
                "không cân bằng được: nguyên tố {element} chỉ xuất hiện ở một vế"
            ));
        }
    }

    let cols = left.len() + right.len();
    let mut matrix: Vec<Vec<Frac>> = elements
        .iter()
        .map(|element| {
            let mut row = vec![Frac::zero(); cols];
            for (index, composition) in left.iter().enumerate() {
                row[index] = Frac::new(*composition.get(element).unwrap_or(&0) as i128, 1);
            }
            for (index, composition) in right.iter().enumerate() {
                row[left.len() + index] =
                    Frac::new(-(*composition.get(element).unwrap_or(&0) as i128), 1);
            }
            row
        })
        .collect();

    let basis = nullspace(&mut matrix, cols);
    if basis.is_empty() {
        return Err("không tìm thấy nghiệm cân bằng".to_string());
    }

    let integer_basis: Vec<Vec<i128>> = basis.iter().filter_map(|v| to_integer(v)).collect();
    if integer_basis.is_empty() {
        return Err("không tìm thấy nghiệm nguyên".to_string());
    }

    let max_multiplier = if integer_basis.len() <= 3 { 8 } else { 4 };
    let mut current = vec![0i128; cols];
    let mut best = None;
    search_positive(&integer_basis, 0, &mut current, max_multiplier, &mut best);

    let solution = best.ok_or_else(|| "không tìm thấy hệ số dương cân bằng".to_string())?;

    // Kiểm tra lại bằng số nguyên tử thực tế.
    for element in &elements {
        let left_count: i128 = left
            .iter()
            .enumerate()
            .map(|(index, composition)| {
                solution[index] * (*composition.get(element).unwrap_or(&0) as i128)
            })
            .sum();
        let right_count: i128 = right
            .iter()
            .enumerate()
            .map(|(index, composition)| {
                solution[left.len() + index] * (*composition.get(element).unwrap_or(&0) as i128)
            })
            .sum();
        if left_count != right_count {
            return Err("nghiệm tìm được không cân bằng, hãy kiểm tra lại công thức".to_string());
        }
    }

    solution
        .iter()
        .map(|&value| i64::try_from(value).map_err(|_| "hệ số quá lớn".to_string()))
        .collect()
}

/// Cân bằng phương trình hóa học bằng đại số tuyến tính.
pub struct BalanceEquation;

#[async_trait]
impl Tool for BalanceEquation {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "balance_equation".into(),
            description: "Cân bằng phương trình hóa học, ví dụ 'H2 + O2 -> H2O'. Trả về hệ số nguyên nhỏ nhất và kiểm tra lại số nguyên tử.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "equation": {
                        "type": "string",
                        "description": "Phương trình, dùng '->' hoặc '=' giữa hai vế"
                    }
                },
                "required": ["equation"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let equation = args
            .get("equation")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArgs("thiếu trường 'equation'".into()))?;

        let Some((left_text, right_text)) = split_equation(equation) else {
            return Ok(ToolOutput::error(
                "phương trình phải có '->' hoặc '=' giữa hai vế".to_string(),
            ));
        };

        let left = match parse_side(left_text) {
            Ok(species) => species,
            Err(error) => return Ok(ToolOutput::error(error)),
        };
        let right = match parse_side(right_text) {
            Ok(species) => species,
            Err(error) => return Ok(ToolOutput::error(error)),
        };

        let coefficients = match balance(&left, &right) {
            Ok(coefficients) => coefficients,
            Err(error) => return Ok(ToolOutput::error(error)),
        };

        let left_formulas: Vec<String> = left_text
            .split('+')
            .map(|s| strip_coefficient(s).to_string())
            .collect();
        let right_formulas: Vec<String> = right_text
            .split('+')
            .map(|s| strip_coefficient(s).to_string())
            .collect();

        let render = |formulas: &[String], offset: usize| -> String {
            formulas
                .iter()
                .enumerate()
                .map(|(index, formula)| {
                    let coefficient = coefficients[offset + index];
                    if coefficient == 1 {
                        formula.clone()
                    } else {
                        format!("{coefficient}{formula}")
                    }
                })
                .collect::<Vec<_>>()
                .join(" + ")
        };

        let balanced = format!(
            "{} -> {}",
            render(&left_formulas, 0),
            render(&right_formulas, left.len())
        );
        let total: i64 = coefficients.iter().sum();

        Ok(ToolOutput::ok(format!(
            "{balanced} (đã cân bằng; tổng hệ số {total})"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn run(equation: &str) -> ToolOutput {
        BalanceEquation
            .execute(json!({ "equation": equation }))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn balances_water() {
        let out = run("H2 + O2 -> H2O").await;
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("2H2 + O2 -> 2H2O"));
    }

    #[tokio::test]
    async fn balances_combustion() {
        let out = run("CH4 + O2 -> CO2 + H2O").await;
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("CH4 + 2O2 -> CO2 + 2H2O"));
    }

    #[tokio::test]
    async fn balances_with_parentheses() {
        let out = run("Fe + O2 -> Fe2O3").await;
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("4Fe + 3O2 -> 2Fe2O3"));
    }

    #[tokio::test]
    async fn rejects_unbalanced_elements() {
        let out = run("H2 + O2 -> H2O2 + Na").await;
        assert!(out.is_error);
    }

    #[tokio::test]
    async fn rejects_missing_arrow() {
        let out = run("H2 + O2").await;
        assert!(out.is_error);
    }
}
