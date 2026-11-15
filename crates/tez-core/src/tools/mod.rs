use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tez_llm::ToolSpec;
use thiserror::Error;

pub mod balance_equation;
pub mod calculator;
pub mod check_grammar;
pub mod check_step;
pub mod chem;
pub mod compact;
pub mod convert_units;
pub mod molar_mass;
pub mod physics_formula;
pub mod plot_function;
pub mod readability;
pub mod workspace;

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("invalid args: {0}")]
    InvalidArgs(String),
    #[error("execution failed: {0}")]
    Execution(String),
    #[error("unknown tool: {0}")]
    Unknown(String),
}

#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
}

impl ToolOutput {
    pub fn ok(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
        }
    }

    pub fn error(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: true,
        }
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;
    /// Thời gian tối đa cho một lần chạy; tool dài như `run_command` có thể ghi đè.
    fn timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(60)
    }
    async fn execute(&self, args: Value) -> Result<ToolOutput, ToolError>;
}

/// Giới hạn đầu vào để parser đệ quy không tràn stack với input do model/người dùng sinh.
pub const MAX_INPUT_LEN: usize = 4096;
pub const MAX_NESTING_DEPTH: usize = 64;

/// Chặn input quá dài hoặc lồng ngoặc quá sâu trước khi đưa vào parser đệ quy.
pub fn check_input_bounds(input: &str) -> Result<(), String> {
    if input.len() > MAX_INPUT_LEN {
        return Err(format!("đầu vào quá dài (tối đa {MAX_INPUT_LEN} ký tự)"));
    }
    let mut depth = 0usize;
    for ch in input.chars() {
        match ch {
            '(' | '[' | '{' => {
                depth += 1;
                if depth > MAX_NESTING_DEPTH {
                    return Err(format!(
                        "lồng ngoặc quá sâu (tối đa {MAX_NESTING_DEPTH} cấp)"
                    ));
                }
            }
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(())
}

/// Thay lời gọi hàm toán học bằng tên `math::...` mà evalexpr hiểu.
///
/// Chỉ thay khi tên hàm đứng riêng: `sin(` được thay, `asin(` và
/// `math::sin(` thì không (tránh `math::amath::sin`).
pub(crate) fn normalize_math_functions(expression: &str) -> String {
    const FUNCTIONS: &[(&str, &str)] = &[
        ("sqrt", "math::sqrt"),
        ("cbrt", "math::cbrt"),
        ("asin", "math::asin"),
        ("acos", "math::acos"),
        ("atan", "math::atan"),
        ("sinh", "math::sinh"),
        ("cosh", "math::cosh"),
        ("tanh", "math::tanh"),
        ("sin", "math::sin"),
        ("cos", "math::cos"),
        ("tan", "math::tan"),
        ("exp", "math::exp"),
        ("abs", "math::abs"),
        ("floor", "math::floor"),
        ("ceil", "math::ceil"),
        ("round", "math::round"),
        ("ln", "math::ln"),
        ("log", "math::log10"),
    ];
    let mut result = expression.to_string();
    for (name, replacement) in FUNCTIONS {
        result = replace_function_call(&result, name, replacement);
    }
    result
}

fn replace_function_call(input: &str, name: &str, replacement: &str) -> String {
    let needle = format!("{name}(");
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(position) = rest.find(&needle) {
        let previous = rest[..position].chars().last();
        let boundary = previous
            .map(|ch| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == ':'))
            .unwrap_or(true);
        out.push_str(&rest[..position]);
        if boundary {
            out.push_str(replacement);
            out.push('(');
        } else {
            out.push_str(&needle);
        }
        rest = &rest[position + needle.len()..];
    }
    out.push_str(rest);
    out
}

#[derive(Default, Clone)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<T: Tool + 'static>(&mut self, tool: T) {
        let spec = tool.spec();
        self.tools.insert(spec.name, Arc::new(tool));
    }

    pub fn specs(&self) -> Vec<ToolSpec> {
        let mut specs: Vec<ToolSpec> = self.tools.values().map(|tool| tool.spec()).collect();
        specs.sort_by(|a, b| a.name.cmp(&b.name));
        specs
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    /// Bản sao chỉ chứa các tool được phép (dùng cho subject pack).
    pub fn filtered(&self, allowed: &[&str]) -> Self {
        let tools = self
            .tools
            .iter()
            .filter(|(name, _)| allowed.contains(&name.as_str()))
            .map(|(name, tool)| (name.clone(), Arc::clone(tool)))
            .collect();
        Self { tools }
    }

    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.tools.keys().cloned().collect();
        names.sort();
        names
    }

    /// Bản sao bỏ các tool bị người dùng tắt trong cấu hình.
    pub fn without(&self, blocked: &[String]) -> Self {
        let tools = self
            .tools
            .iter()
            .filter(|(name, _)| !blocked.iter().any(|item| item == *name))
            .map(|(name, tool)| (name.clone(), Arc::clone(tool)))
            .collect();
        Self { tools }
    }

    pub async fn execute(&self, name: &str, args: Value) -> Result<ToolOutput, ToolError> {
        let tool = self
            .get(name)
            .ok_or_else(|| ToolError::Unknown(name.to_string()))?;
        tool.execute(args).await
    }
}

/// Registry cho chế độ Code, giới hạn trong `root`.
pub fn code_registry(root: impl AsRef<Path>) -> Result<ToolRegistry, ToolError> {
    let workspace = workspace::Workspace::new(root)?;
    let mut registry = ToolRegistry::new();
    registry.register(workspace::ReadFile::new(workspace.clone()));
    registry.register(workspace::WriteFile::new(workspace.clone()));
    registry.register(workspace::EditFile::new(workspace.clone()));
    registry.register(workspace::ListFiles::new(workspace.clone()));
    registry.register(workspace::GlobFiles::new(workspace.clone()));
    registry.register(workspace::GrepFiles::new(workspace.clone()));
    registry.register(workspace::RunCommand::new(workspace));
    registry.register(compact::Compact);
    Ok(registry)
}

/// Tên tool của chế độ Code, dùng cho prompt và kiểm tra.
pub const CODE_TOOL_NAMES: &[&str] = &[
    "read_file",
    "write_file",
    "edit_file",
    "list_files",
    "glob",
    "grep",
    "run_command",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_normal_expression() {
        assert!(check_input_bounds("(3 + 5) * 2").is_ok());
        assert!(check_input_bounds("H2O").is_ok());
    }

    #[test]
    fn rejects_deeply_nested_input() {
        let deep = format!("{}1{}", "(".repeat(5000), ")".repeat(5000));
        assert!(check_input_bounds(&deep).is_err());
        let at_limit = format!(
            "{}1{}",
            "(".repeat(MAX_NESTING_DEPTH),
            ")".repeat(MAX_NESTING_DEPTH)
        );
        assert!(check_input_bounds(&at_limit).is_ok());
    }

    #[test]
    fn rejects_overlong_input() {
        let long = "1+".repeat(MAX_INPUT_LEN);
        assert!(check_input_bounds(&long).is_err());
    }
}
