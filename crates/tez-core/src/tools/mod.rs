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
    async fn execute(&self, args: Value) -> Result<ToolOutput, ToolError>;
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
    registry.register(workspace::ListFiles::new(workspace.clone()));
    registry.register(workspace::RunCommand::new(workspace));
    Ok(registry)
}
